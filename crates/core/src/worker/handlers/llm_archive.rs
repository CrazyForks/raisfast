//! Cron handler: llm_logs_archive — roll up `llm_logs` detail rows past the
//! retention window into `llm_logs_summary`, then delete the details (design
//! §9 retention: long-term reports survive, table growth is bounded).
//!
//! Retention comes from options `llm.log_retention_days` (default 90) and
//! `llm.log_retention_days_test` (source=test, default 7).

use crate::errors::app_error::AppResult;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Metadata for the admin task menu.
pub const META: HandlerMeta = HandlerMeta {
    id: "llm_logs_archive",
    display_name: "LLM Logs Archive",
    description: "Rolls old llm_logs detail rows up into llm_logs_summary, then deletes them",
    category: "AI / LLM",
    params_schema: None,
    icon: Some("archive"),
};

pub struct LlmArchiveHandler {
    pool: crate::db::Pool,
}

impl LlmArchiveHandler {
    /// Creates the handler.
    #[must_use]
    pub fn new(pool: crate::db::Pool) -> Self {
        Self { pool }
    }
}

/// Read an integer retention option, falling back to `default` when unset.
async fn retention_days(pool: &crate::db::Pool, key: &str, default: i64) -> i64 {
    crate::models::options::find_by_key(pool, key, None)
        .await
        .ok()
        .flatten()
        .and_then(|row| {
            row.value
                .as_i64()
                .or_else(|| row.value.as_str().and_then(|s| s.parse().ok()))
        })
        .filter(|d| *d > 0)
        .unwrap_or(default)
}

#[async_trait::async_trait]
impl JobHandler for LlmArchiveHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let _ = job;
        let default_days = retention_days(&self.pool, "llm.log_retention_days", 90).await;
        let test_days = retention_days(&self.pool, "llm.log_retention_days_test", 7).await;
        let report =
            crate::llm::models::log::archive_old_logs(&self.pool, default_days, test_days).await?;
        if report.deleted_rows > 0 || report.summary_rows > 0 {
            tracing::info!(
                default_days,
                test_days,
                summary_rows = report.summary_rows,
                deleted_rows = report.deleted_rows,
                "llm logs archive done"
            );
        }
        Ok(())
    }
}

crate::register_cron_handler!(&META, |deps| {
    Box::new(LlmArchiveHandler::new(deps.pool.clone()))
});
