//! `ingress.retry` job handler — internal backoff retry for failed inbound
//! routes (integration.md §6.4). Re-runs ONLY the route from the receipt's
//! envelope snapshot; the pipeline owns the state machine.

use crate::errors::app_error::AppResult;
use crate::integration::pipeline::RetryResult;
use crate::worker::Job;
use crate::worker::handler::{HandlerMeta, JobHandler};

static META: HandlerMeta = HandlerMeta {
    id: "ingress.retry",
    display_name: "集成入站重试",
    description: "对路由失败的入站信封按退避策略重跑路由（内部自动调度，无需手动创建）",
    category: "集成",
    params_schema: Some(
        r#"{"type":"object","properties":{"trace_id":{"type":"integer","description":"回执/追踪 ID"},"attempt":{"type":"integer","description":"当前重试序号"}},"required":["trace_id"]}"#,
    ),
    icon: None,
};

/// Retry handler — resolves the shared [`Pipeline`] at execution time (the
/// plane is assembled after the worker registry in startup order).
pub struct IngressRetryHandler;

#[async_trait::async_trait]
impl JobHandler for IngressRetryHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let Some(trace_id) = payload.get("trace_id").and_then(|v| v.as_i64()) else {
            tracing::warn!("ingress.retry without trace_id: {payload}");
            return Ok(());
        };
        let Some(pipeline) = crate::integration::shared_pipeline() else {
            // Plane disabled/absent — nothing to retry through.
            tracing::warn!(trace_id, "ingress.retry: integration plane not initialized");
            return Ok(());
        };
        match pipeline.run_retry(trace_id).await? {
            RetryResult::Delivered | RetryResult::Dead | RetryResult::Skipped => Ok(()),
            RetryResult::Rescheduled => {
                // Re-enqueue happened inside run_retry via next_retry_at? No —
                // the handler path enqueues here so the queue owns scheduling.
                let attempt = payload.get("attempt").and_then(|v| v.as_i64()).unwrap_or(1);
                pipeline.schedule_retry_public(trace_id, attempt + 1).await;
                Ok(())
            }
        }
    }
}

crate::register_cron_handler!(&META, |_deps| { Box::new(IngressRetryHandler) });
