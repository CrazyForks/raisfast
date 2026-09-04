//! Restricted local shell tool (`run_shell`) for the agent.
//!
//! Not registered unless `[ai].allow_shell` is on (env
//! `RAISFAST_AI_ALLOW_SHELL=true`) — default closed — and even then the
//! per-agent `tools` allowlist must name it. Execution is bounded: working dir
//! = the tenant workspace sandbox, scrubbed environment (allowlisted vars
//! only), a hard timeout (1-60s, killed on expiry) and capped output.
//!
//! **Risk note (by design, not hidden):** the child runs as the raisfast
//! process user, so it can reach anything that user can. This is a local /
//! single-tenant development facility, not a multi-tenant isolation boundary.
//! For real isolation use containerized execution (Docker, like zeroclaw's
//! `SkillShellTool + DockerRuntime`) — not implemented here.

use std::process::Stdio;

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::agent::tools::files::workspace_root;
use crate::middleware::auth::AuthUser;

/// Environment variables forwarded to the child (mirrors zeroclaw's safe set).
#[cfg(not(target_os = "windows"))]
const SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TERM", "LANG", "LC_ALL", "LC_CTYPE", "USER", "SHELL", "TMPDIR",
];
#[cfg(target_os = "windows")]
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "SystemRoot",
    "TEMP",
    "TMP",
    "USERNAME",
    "USERPROFILE",
];

/// Combined stdout+stderr cap returned to the model.
const MAX_OUTPUT_BYTES: usize = 200_000;

/// Hard-coded high-risk patterns (label, regex). Best-effort pre-spawn scan of
/// the raw command — NOT a sandbox; real isolation needs containerized
/// execution. Deny list semantics: an allowlist would block legitimate ad-hoc
/// shell work, so we block the dangerous primitives instead and document that
/// this is a policy, not a boundary.
const BASE_DENY: &[(&str, &str)] = &[
    (
        "reboot/poweroff",
        r"\b(reboot|poweroff|shutdown|halt)\b|\binit\s+0\b",
    ),
    (
        "disk destruction",
        r"\b(mkfs|fdisk|shred|mkswap)\b|\bdd\b[^\n]*\bof=",
    ),
    (
        "recursive destructive rm",
        r"\brm\b[^\n]*?(\s-[A-Za-z]*r[A-Za-z]*\b|--recursive\b)[^\n]*?(\s/[^\s]|\s/|~|\s\*)",
    ),
    (
        "world-writable chmod/chown",
        r"\bchmod\b[^\n]*(777|[-0-9a-f]{3,4})[^\n]*?(\s/|~)|\bchown\b[^\n]*-R[^\n]*?(\s/|~)",
    ),
    (
        "credential/sensitive read",
        r"(\.ssh|\.aws|\.gnupg|id_rsa|known_hosts|\.pem\b|/etc/passwd|\.credentials)",
    ),
    (
        "env/secret files",
        r"(\.env\b|raisfast\.db|raisfast\.key|\.sqlite3?|secret[s]?\.json)",
    ),
    (
        "privilege escalation",
        r"\b(sudo|gksudo)\b|\bsu\s+[-a-zA-Z]+\s",
    ),
    ("kill-all", r"\b(killall|pkill)\b"),
    ("service control", r"\b(systemctl|service)\b"),
];

/// Blocked if a pattern matches, with the label as the reason.
fn deny_reason(command: &str) -> Option<String> {
    static POLICY: std::sync::OnceLock<Vec<(String, regex::Regex)>> = std::sync::OnceLock::new();
    let policy = POLICY.get_or_init(|| {
        let mut compiled: Vec<(String, regex::Regex)> = BASE_DENY
            .iter()
            .filter_map(|(label, pat)| {
                regex::RegexBuilder::new(pat)
                    .case_insensitive(true)
                    .build()
                    .ok()
                    .map(|re| ((*label).to_string(), re))
            })
            .collect();
        // Operator-extendable via RAISFAST_AI_SHELL_DENY="pat,pat" (invalid
        // entries are skipped with a warn so a typo can't crash the server).
        for entry in std::env::var("RAISFAST_AI_SHELL_DENY")
            .ok()
            .into_iter()
            .flat_map(|v| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>())
        {
            match regex::RegexBuilder::new(&entry).case_insensitive(true).build() {
                Ok(re) => compiled.push(("RAISFAST_AI_SHELL_DENY".to_string(), re)),
                Err(e) => tracing::warn!(pattern = %entry, error = %e, "invalid RAISFAST_AI_SHELL_DENY entry ignored"),
            }
        }
        compiled
    });
    for (label, re) in policy {
        if re.is_match(command) {
            return Some(label.clone());
        }
    }
    None
}

pub struct RunShellTool {
    name: String,
    cwd: std::path::PathBuf,
}

impl RunShellTool {
    pub fn new(auth: &AuthUser) -> Self {
        let tenant = auth.tenant_id().unwrap_or("default");
        Self::for_tenant(tenant)
    }

    pub(crate) fn for_tenant(tenant: &str) -> Self {
        let cwd = workspace_root().join(tenant);
        Self {
            name: "run_shell".to_string(),
            cwd,
        }
    }
}

#[async_trait]
impl Tool for RunShellTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Runs a shell command on the local machine inside the tenant workspace. \
         Bounded: workspace cwd, scrubbed env, 1-60s timeout, capped output. \
         High-risk patterns are denied before spawn (reboot/disk wipe/recursive rm/credential \
         reads/sudo/kill-all/service control; extend via RAISFAST_AI_SHELL_DENY). \
         Only registered when the operator enabled RAISFAST_AI_ALLOW_SHELL."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "shell command to execute (cwd = tenant workspace)"
                },
                "timeout_secs": { "type": "integer", "minimum": 1, "maximum": 60, "default": 15 }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let Some(command) = args.get("command").and_then(Value::as_str) else {
            return Err("command (string) is required".to_string());
        };
        let command = command.trim();
        if command.is_empty() {
            return Err("command is empty".to_string());
        }
        if let Some(label) = deny_reason(command) {
            return Err(format!(
                "run_shell blocked by policy ({label}). The command matches a high-risk pattern; \
                 rework it or extend RAISFAST_AI_SHELL_DENY. This is a policy guard, not a sandbox."
            ));
        }
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(15)
            .clamp(1, 60);

        if let Err(e) = std::fs::create_dir_all(&self.cwd) {
            return Err(format!("workspace not usable: {e}"));
        }

        let mut cmd = build_shell_command(command);
        cmd.current_dir(&self.cwd)
            .env_clear()
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for var in SAFE_ENV_VARS {
            if let Ok(val) = std::env::var(var) {
                cmd.env(var, val);
            }
        }

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return Err(format!("spawn failed: {e}")),
        };

        let stdout_pipe = child.stdout.take();
        let stderr_pipe = child.stderr.take();
        let (code, stdout, stderr) =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
                let stdout = async {
                    match stdout_pipe {
                        Some(mut p) => read_all(&mut p).await,
                        None => String::new(),
                    }
                };
                let stderr = async {
                    match stderr_pipe {
                        Some(mut p) => read_all(&mut p).await,
                        None => String::new(),
                    }
                };
                let (stdout, stderr) = tokio::join!(stdout, stderr);
                let code = match child.wait().await {
                    Ok(s) => s
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".into()),
                    Err(e) => format!("wait error: {e}"),
                };
                (code, stdout, stderr)
            })
            .await
            {
                Ok(r) => r,
                Err(_) => {
                    // Timeout: the read future is dropped above; take the pipes back
                    // and kill the process so nothing keeps running detached.
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    ("timed out".to_string(), String::new(), String::new())
                }
            };

        let stdout = truncate(&stdout, MAX_OUTPUT_BYTES);
        let stderr = truncate(&stderr, MAX_OUTPUT_BYTES);

        let mut out = format!("exit: {code}\n");
        if !stdout.is_empty() {
            out.push_str(&stdout);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        if !stderr.is_empty() {
            out.push_str(&format!("-- stderr --\n{stderr}\n"));
        }
        if code == "timed out" {
            out.push_str(&format!("(killed after {timeout_secs}s)"));
        }
        Ok(out)
    }
}

/// Spawn the default shell to run `command` (POSIX `sh -c`, Windows `cmd /C`).
fn build_shell_command(command: &str) -> tokio::process::Command {
    #[cfg(not(target_os = "windows"))]
    let cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    #[cfg(target_os = "windows")]
    let cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    cmd
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut boundary = max.min(s.len());
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}… [output truncated]", &s[..boundary])
}

async fn read_all<R: tokio::io::AsyncRead + Unpin>(pipe: &mut R) -> String {
    let mut buf = String::new();
    let _ = pipe.read_to_string(&mut buf).await;
    buf
}

pub fn register(registry: &mut raisfast_agent::ToolRegistry, auth: &AuthUser) {
    registry.register(RunShellTool::new(auth));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> RunShellTool {
        RunShellTool::for_tenant("shell-test")
    }

    #[tokio::test]
    async fn runs_command_and_returns_output() {
        let out = tool()
            .execute(serde_json::json!({ "command": "printf 'hello-shell'" }))
            .await
            .unwrap();
        assert!(out.contains("hello-shell"), "output: {out}");
        assert!(out.contains("exit: 0"), "output: {out}");
    }

    #[tokio::test]
    async fn timeout_kills_long_command() {
        // `sleep 3` on POSIX (Windows test would need a different long cmd).
        let out = tool()
            .execute(serde_json::json!({ "command": "sleep 3", "timeout_secs": 1 }))
            .await
            .unwrap();
        assert!(out.contains("timed out"), "output: {out}");
    }

    #[tokio::test]
    async fn scrubbed_env_still_has_path() {
        let out = tool()
            .execute(serde_json::json!({ "command": "printf '%s' \"$PATH\"" }))
            .await
            .unwrap();
        assert!(!out.is_empty(), "PATH should be forwarded: {out}");
    }

    #[test]
    fn benign_commands_pass_policy() {
        for cmd in [
            "pwd",
            "ls -la",
            "printf 'hello'",
            "cat /tmp/data.txt",
            "git status",
            "curl -s https://example.com",
            "node -e 'console.log(1)'",
        ] {
            assert!(deny_reason(cmd).is_none(), "should allow: {cmd}");
        }
    }

    #[test]
    fn dangerous_commands_blocked() {
        for (cmd, label) in [
            ("rm -rf /", "recursive destructive rm"),
            ("rm -fr /usr", "recursive destructive rm"),
            ("rm -r /tmp/x", "recursive destructive rm"),
            ("sudo whoami", "privilege escalation"),
            ("cat ~/.ssh/id_rsa", "credential/sensitive read"),
            ("shutdown -h now", "reboot/poweroff"),
            ("dd if=/dev/zero of=/dev/sda", "disk destruction"),
            ("chmod -R 777 /", "world-writable chmod/chown"),
            ("pkill raisfast", "kill-all"),
            ("cat /etc/passwd", "credential/sensitive read"),
            ("cat /repo/.env", "env/secret files"),
        ] {
            let got = deny_reason(cmd);
            assert!(got.is_some(), "should block: {cmd}");
            assert_eq!(got.unwrap(), label, "label for: {cmd}");
        }
    }
}
