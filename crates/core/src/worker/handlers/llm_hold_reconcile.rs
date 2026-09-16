//! Cron handler: llm_hold_reconcile — refund orphaned LLM wallet holds.
//!
//! Streaming disconnects (or any crash between hold and settle) can leave a
//! `llm_hold` debit with no matching settle row — real money frozen forever.
//! This sweep refunds holds that stayed unsettled past a grace window
//! (user-favoring: actual usage is unknowable at that point). Refunds reuse
//! the `llm_settle` `-r` idempotency suffix, so re-runs never double-refund.

use crate::errors::app_error::AppResult;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

/// Holds younger than this are assumed in-flight and left alone.
const GRACE_MINS: i64 = 60;
/// Per-run cap so a large backlog drains gradually instead of one huge burst.
const MAX_REFUNDS_PER_RUN: i64 = 200;

/// Metadata for the admin task menu.
pub const META: HandlerMeta = HandlerMeta {
    id: "llm_hold_reconcile",
    display_name: "LLM Hold Reconcile Sweep",
    description: "Refunds LLM wallet holds left unsettled past the grace window (stream disconnects / crashes)",
    category: "AI / LLM",
    params_schema: None,
    icon: Some("shield-check"),
};

pub struct LlmHoldReconcileHandler {
    pool: crate::db::Pool,
}

impl LlmHoldReconcileHandler {
    /// Creates the handler.
    #[must_use]
    pub fn new(pool: crate::db::Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl JobHandler for LlmHoldReconcileHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let _ = job;
        let refunded = crate::services::wallet::reconcile_orphan_llm_holds(
            &self.pool,
            chrono::Duration::minutes(GRACE_MINS),
            MAX_REFUNDS_PER_RUN,
        )
        .await;
        match refunded {
            Ok(n) if n > 0 => tracing::info!(refunded = n, "llm hold reconcile refunded orphans"),
            Ok(_) => {}
            Err(err) => tracing::warn!(%err, "llm hold reconcile sweep failed"),
        }
        Ok(())
    }
}

crate::register_cron_handler!(&META, |deps| {
    Box::new(LlmHoldReconcileHandler::new(deps.pool.clone()))
});
