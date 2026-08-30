//! Plugin job dispatch
//!
//! When the built-in handler registry has no match, `WorkerRunner` falls back
//! to this dispatcher. Two dispatch modes:
//!
//! 1. **Targeted** — plugins that declare `[[jobs]]` in `manifest.toml` answer
//!    their declared `job_type`s directly: the handler receives the job
//!    payload and errors propagate to the worker retry/dead-letter machinery.
//! 2. **Legacy broadcast** — undeclared job types are fanned out to all
//!    plugins hooking `on-cron-tick` (fire-and-forget, errors swallowed).

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::plugins::PluginManager;

use super::Job;

/// Build the ambient trace context from a job payload (`trace_id` accepts
/// string or number form — pipeline injects strings, see pipeline post_route).
fn build_trace_ctx(payload: &serde_json::Value) -> Option<crate::integration::trace::TraceCtx> {
    let trace_id = crate::types::snowflake_id::parse_id_value(payload.get("trace_id")?)?;
    let channel_key = payload
        .get("channel_key")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Some(crate::integration::trace::TraceCtx {
        trace_id,
        channel_key,
    })
}

/// Plugin job dispatcher
pub struct PluginCronDispatcher {
    plugins: Arc<PluginManager>,
}

impl PluginCronDispatcher {
    /// Creates a new dispatcher
    pub fn new(plugins: Arc<PluginManager>) -> Self {
        Self { plugins }
    }

    /// Dispatches a Job: targeted `[[jobs]]` route first, legacy broadcast
    /// otherwise. Targeted jobs run inside the ambient TRACE_CTX when the
    /// payload carries `trace_id` — `emit_event` / `call_api` / plugin host
    /// APIs attach it automatically (§10.7).
    pub async fn dispatch(&self, job: &Job) -> AppResult<()> {
        // Targeted route: error semantics are the worker's (retry/dead).
        if let Job::Custom { job_type, payload } = job
            && self.plugins.resolve_job(job_type).is_some()
        {
            tracing::debug!("dispatching job to plugin handler: job_type={job_type}");
            let ctx = build_trace_ctx(payload);
            return match ctx {
                Some(ctx) => {
                    crate::integration::trace::with_trace(
                        ctx,
                        self.plugins.call_job(job_type, payload),
                    )
                    .await
                }
                None => self.plugins.call_job(job_type, payload).await,
            };
        }

        // Legacy broadcast (on-cron-tick), fire-and-forget.
        let payload = match job {
            Job::Custom { job_type, payload } => serde_json::json!({
                "job_type": job_type,
                "payload": payload,
                "timestamp": crate::utils::tz::now_utc(),
            }),
            _ => serde_json::json!({
                "job_type": job.job_type(),
                "payload": serde_json::to_value(job).unwrap_or_default(),
                "timestamp": crate::utils::tz::now_utc(),
            }),
        };

        tracing::info!(
            "dispatching cron job to plugins: job_type={}",
            job.job_type()
        );

        self.plugins.dispatch_action("on_cron_tick", &payload).await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatch_does_not_panic_without_plugins() {
        let config = Arc::new(crate::config::app::AppConfig::test_defaults());
        let mgr = PluginManager::new_with_options(
            config,
            crate::plugins::PluginManagerOptions {
                pool: None,
                event_bus: None,
                content_registry: None,
            },
        )
        .await;
        let dispatcher = PluginCronDispatcher::new(mgr);

        let job = Job::Custom {
            job_type: "test_task".into(),
            payload: serde_json::json!({"key": "value"}),
        };
        let result = dispatcher.dispatch(&job).await;
        assert!(result.is_ok());
    }
}
