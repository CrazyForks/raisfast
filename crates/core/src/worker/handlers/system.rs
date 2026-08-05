//! Cron handler: run system scripts (shell commands).
//!
//! Triggered by `exec_kind = "system"` cron schedules. Reads the command from
//! `cron_schedules.script_source`, executes it as a subprocess, and captures
//! stdout/stderr/exit_code.
//!
//! **Security**: Requires cargo feature `cron-system` to compile. At runtime,
//! `config.cron_allow_system_scripts` must be `true` — otherwise the handler
//! returns an error.

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::worker::{Job, JobHandler};

/// Handler that executes system commands for cron schedules.
pub struct SystemJobHandler {
    pool: Pool,
    config: std::sync::Arc<AppConfig>,
}

impl SystemJobHandler {
    /// Creates a new system script handler
    #[must_use]
    pub fn new(pool: Pool, config: std::sync::Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for SystemJobHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };

        if !self.config.cron_allow_system_scripts {
            return Err(AppError::Internal(anyhow::anyhow!(
                "system scripts are disabled (cron_allow_system_scripts=false)"
            )));
        }

        let schedule_id = payload
            .get("schedule_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "run_system: missing 'schedule_id' in payload"
                ))
            })?;

        // Fetch command + flags from cron_schedules
        use crate::db::{DbDriver, Driver};
        let row: Option<(Option<String>, bool, Option<i32>, String)> = sqlx::query_as(
            &format!(
                "SELECT script_source, use_shell, timeout_secs, label FROM cron_schedules WHERE id = {}",
                Driver::ph(1)
            ),
        )
        .bind(schedule_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((script_source, use_shell, timeout_secs, label)) = row else {
            return Err(AppError::not_found("cron_schedule"));
        };

        let command = script_source.as_deref().unwrap_or("");
        if command.trim().is_empty() {
            tracing::warn!(
                "[run_system] schedule {schedule_id} ('{label}') has empty command, skipping"
            );
            return Ok(());
        }

        let workdir = self
            .config
            .cron_system_workdir
            .as_deref()
            .unwrap_or(&self.config.storage_root_dir);

        let timeout_secs = timeout_secs.unwrap_or(300) as u64;

        tracing::info!(
            "[run_system] executing command for schedule {schedule_id} ('{label}'), shell={use_shell}, timeout={timeout_secs}s"
        );

        use std::process::Stdio;
        use tokio::process::Command;

        let mut cmd = if use_shell {
            let mut c = Command::new("/bin/sh");
            c.arg("-c").arg(command);
            c
        } else {
            let tokens = shell_words::split(command)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("parse command: {e}")))?;
            let (prog, args) = tokens
                .split_first()
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("empty command")))?;
            let mut c = Command::new(prog);
            c.args(args);
            c
        };

        cmd.env("CRON_PAYLOAD", payload.to_string())
            .current_dir(workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let output =
            tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
                .await
                .map_err(|_| {
                    AppError::Internal(anyhow::anyhow!(
                        "system script timed out after {timeout_secs}s"
                    ))
                })?
                .map_err(|e| AppError::Internal(anyhow::anyhow!("spawn failed: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(AppError::Internal(anyhow::anyhow!(
                "exit code {}: stderr={}, stdout={}",
                output.status.code().unwrap_or(-1),
                stderr.trim(),
                stdout.trim()
            )));
        }

        tracing::info!(
            "[run_system] schedule {schedule_id} completed, exit=0, stdout_len={}",
            output.stdout.len()
        );

        Ok(())
    }
}
