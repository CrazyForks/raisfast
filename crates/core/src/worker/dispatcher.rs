//! Plugin Cron dispatch dispatcher
//!
//! When the built-in Handler registry has no matching Handler,
//! `WorkerRunner` falls back to this dispatcher, passing job data to the plugin system.
//!
//! Plugins receive cron jobs by declaring `[hooks.on-cron-tick]` in their `plugin.toml`.

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::plugins::PluginManager;

use super::Job;

/// Plugin Cron dispatcher
///
/// Serializes Job data and sends it via `PluginManager::dispatch_action` to
/// plugins that declared the `on_cron_tick` hook.
pub struct PluginCronDispatcher {
    plugins: Arc<PluginManager>,
}

impl PluginCronDispatcher {
    /// Creates a new dispatcher
    pub fn new(plugins: Arc<PluginManager>) -> Self {
        Self { plugins }
    }

    /// Dispatches a Job to plugins
    pub async fn dispatch(&self, job: &Job) -> AppResult<()> {
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
