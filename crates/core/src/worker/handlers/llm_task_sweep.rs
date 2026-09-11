//! Cron handler: llm_task_sweep — expire overdue non-terminal LLM async
//! tasks (`llm_tasks`, video now / batch later) so abandoned pre-charges
//! are refunded. Active clients also expire tasks lazily on read
//! (poll-on-read); this sweep is the unattended safety net.

use crate::errors::app_error::AppResult;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Metadata for the admin task menu.
pub const META: HandlerMeta = HandlerMeta {
    id: "llm_task_sweep",
    display_name: "LLM Async Task Sweep",
    description: "Expires overdue LLM async tasks (video etc.) and refunds their pre-charged quota",
    category: "AI / LLM",
    params_schema: None,
    icon: Some("timer-off"),
};

pub struct LlmTaskSweepHandler {
    pool: crate::db::Pool,
}

impl LlmTaskSweepHandler {
    /// Creates the handler.
    #[must_use]
    pub fn new(pool: crate::db::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobHandler for LlmTaskSweepHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let _ = job;
        let expired = crate::llm::relay::tasks::sweep_expired_tasks(&self.pool).await;
        if expired > 0 {
            tracing::info!(expired, "llm task sweep expired stale tasks");
        }
        Ok(())
    }
}

crate::register_cron_handler!(&META, |deps| {
    Box::new(LlmTaskSweepHandler::new(deps.pool.clone()))
});
