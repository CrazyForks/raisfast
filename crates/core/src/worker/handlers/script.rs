//! Cron handler: run inline sandbox scripts (JS / Lua / Rhai).
//!
//! Triggered by `exec_kind = "script"` cron schedules. The handler reads the
//! latest script source from `cron_schedules` (not from the Job payload) so
//! that editing a schedule's script takes effect on the next tick without
//! needing to re-enqueue.
//!
//! Payload convention (`Job::Custom { job_type: "run_script", payload }`):
//! ```json
//! { "schedule_id": 12345, "entry": "on_cron_tick" }
//! ```
//! The actual `script_source` / `script_lang` / `timeout_secs` are looked up
//! from `cron_schedules` by `schedule_id`.

use std::sync::Arc;

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::plugins::Permissions;
use crate::plugins::PluginManager;
use crate::worker::{Job, JobHandler};

/// Handler that executes inline sandbox scripts for cron schedules.
pub struct ScriptJobHandler {
    plugins: Arc<PluginManager>,
    pool: Pool,
    /// Default permissions for cron scripts (configurable via AppConfig).
    permissions: Permissions,
}

impl ScriptJobHandler {
    /// Creates a new script handler
    #[must_use]
    pub fn new(plugins: Arc<PluginManager>, pool: Pool, permissions: Permissions) -> Self {
        Self {
            plugins,
            pool,
            permissions,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for ScriptJobHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };

        let schedule_id = payload
            .get("schedule_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "run_script: missing 'schedule_id' in payload"
                ))
            })?;

        let entry = payload
            .get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("on_cron_tick");

        // Fetch the latest script source from cron_schedules.
        use crate::db::{DbDriver, Driver};
        let row: Option<(Option<String>, Option<String>, String)> =
            sqlx::query_as(crate::db::safe_sql(&format!(
                "SELECT script_lang, script_source, label FROM cron_schedules WHERE id = {}",
                Driver::ph(1)
            )))
            .bind(schedule_id)
            .fetch_optional(&self.pool)
            .await?;

        let Some((script_lang, script_source, label)) = row else {
            return Err(AppError::not_found("cron_schedule"));
        };

        let lang = script_lang.as_deref().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("schedule {schedule_id} has no script_lang"))
        })?;
        let code = script_source.as_deref().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "schedule {schedule_id} has no script_source"
            ))
        })?;

        if code.trim().is_empty() {
            tracing::warn!(
                "[run_script] schedule {schedule_id} ('{label}') has empty script, skipping"
            );
            return Ok(());
        }

        let id = format!("__cron__{schedule_id}");

        tracing::info!(
            "[run_script] executing {lang} script for schedule {schedule_id} ('{label}'), entry={entry}"
        );

        self.plugins
            .run_inline_script(lang, &id, code, entry, payload, self.permissions.clone())
            .await?;

        Ok(())
    }
}
