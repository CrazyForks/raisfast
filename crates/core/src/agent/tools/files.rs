//! Managed-file tools: `file_write` / `file_read` / `file_list` / `file_delete`
//! over the platform plugin VFS (`VirtualFs`), one sandbox dir per tenant
//! (`{plugin_vfs_root}/agent/{tenant}/`).
//!
//! Reusing `VirtualFs` gives escape protection (`..` rejected), single-file and
//! total-quota enforcement, and read/write gating for free — no bespoke path
//! logic in the agent layer. Reads are capped so one huge file cannot blow the
//! context; oversize reads return an explicit `{truncated:true, ...}` envelope.

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;

use crate::AppState;
use crate::middleware::auth::AuthUser;
use crate::plugins::vfs::VfsError;
use crate::plugins::{Permissions, vfs::VirtualFs};

/// Explicit cap for a single `file_read` payload handed to the model.
const READ_LIMIT_CHARS: usize = 200_000;

/// Agent workspace root (independent of plugin VFS). Override with
/// `RAISFAST_AGENT_WORKSPACE`; default `storage/agent/workspace`.
pub fn workspace_root() -> std::path::PathBuf {
    std::env::var("RAISFAST_AGENT_WORKSPACE")
        .ok()
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("storage/agent/workspace"))
}

pub struct ManagedFileTool {
    name: String,
    description: String,
    vfs: VirtualFs,
    writable: bool,
}

impl ManagedFileTool {
    fn new(name: &str, description: String, vfs: VirtualFs, writable: bool) -> Self {
        Self {
            name: name.to_string(),
            description,
            vfs,
            writable,
        }
    }

    fn err(e: VfsError) -> String {
        format!("file error: {e}")
    }
}

#[async_trait]
impl Tool for ManagedFileTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "relative path inside the sandbox (no leading /, no ..)"
                },
                "content": { "type": "string", "description": "text content (file_write)" },
                "dir": {
                    "type": "string",
                    "description": "directory to list; empty = sandbox root (file_list)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        match self.name.as_str() {
            "file_write" => {
                if !self.writable {
                    return Err("file_write: sandbox is read-only".to_string());
                }
                let path = path_arg(&args)?;
                let Some(content) = args.get("content").and_then(Value::as_str) else {
                    return Err("content (string) is required".to_string());
                };
                self.vfs
                    .write_file(path, content)
                    .map(|_| format!("wrote {path} in sandbox {}", self.vfs.root().display()))
                    .map_err(Self::err)
            }
            "file_delete" => {
                if !self.writable {
                    return Err("file_delete: sandbox is read-only".to_string());
                }
                let path = path_arg(&args)?;
                self.vfs
                    .delete_file(path)
                    .map(|_| format!("deleted {path}"))
                    .map_err(Self::err)
            }
            "file_read" => {
                let path = path_arg(&args)?;
                let info = self.vfs.stat(path).map_err(Self::err)?;
                if info.is_dir {
                    return Err(format!("{path} is a directory; use file_list"));
                }
                if info.size > READ_LIMIT_CHARS {
                    let head = self
                        .vfs
                        .read_file(path)
                        .map(|s| first_chars(&s, READ_LIMIT_CHARS).to_string())
                        .map_err(Self::err)?;
                    return Ok(serde_json::json!({
                        "truncated": true,
                        "total_bytes": info.size,
                        "value": head,
                        "next_steps": [
                            "file is larger than the read guard; this is only the prefix.",
                            "Prefer reading a smaller file, or transform the file with run_js and read its summarized result."
                        ]
                    })
                    .to_string());
                }
                self.vfs.read_file(path).map_err(Self::err)
            }
            "file_list" => {
                let dir = args.get("dir").and_then(Value::as_str).unwrap_or_default();
                self.vfs
                    .list_dir(dir)
                    .map(|entries| {
                        if entries.is_empty() {
                            format!("(empty sandbox at {})", self.vfs.root().display())
                        } else {
                            format!(
                                "sandbox {}:\n{}",
                                self.vfs.root().display(),
                                entries.join("\n")
                            )
                        }
                    })
                    .map_err(Self::err)
            }
            _ => Err("unknown file tool".to_string()),
        }
    }
}

fn path_arg(args: &Value) -> Result<&str, String> {
    args.get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "path (string) is required".to_string())
}

fn first_chars(s: &str, max: usize) -> &str {
    let mut boundary = max.min(s.len());
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

/// Register managed-file tools. Each tenant gets its own workspace sandbox dir
/// `{workspace}/{tenant}/`; paths may not escape it.
pub fn register(registry: &mut raisfast_agent::ToolRegistry, state: &AppState, auth: &AuthUser) {
    let tenant = auth.tenant_id().unwrap_or("default");
    let root_hint = workspace_root().join(tenant).display().to_string();

    let mk = |name: &'static str, writable: bool| -> ManagedFileTool {
        let perms = if writable {
            Permissions {
                filesystem: vec!["read-write".to_string()],
                ..Permissions::default()
            }
        } else {
            Permissions {
                filesystem: vec!["read".to_string()],
                ..Permissions::default()
            }
        };
        let sandbox = VirtualFs::new_at(
            workspace_root().join(tenant),
            state.config.plugin_vfs_max_file_size,
            state.config.plugin_vfs_max_total_size,
            &perms,
        );
        let description = match name {
            "file_write" => {
                "Create/overwrite a text file inside the tenant sandbox. Writes go through VFS quota and size guards; `..` is rejected."
            }
            "file_read" => {
                "Read a text file from the tenant sandbox. Oversize files return an explicit truncated envelope."
            }
            "file_list" => "List files/dirs inside the tenant sandbox (empty dir param = root).",
            _ => "Delete a file inside the tenant sandbox.",
        };
        ManagedFileTool::new(
            name,
            format!("{description} Sandbox: {root_hint}"),
            sandbox,
            writable,
        )
    };

    for (name, writable) in [
        ("file_list", false),
        ("file_read", false),
        ("file_write", true),
        ("file_delete", true),
    ] {
        registry.register(mk(name, writable));
    }
}
