//! 插件 Cron 调度分发器
//!
//! 当内置 Handler 注册表没有匹配的 Handler 时，
//! `WorkerRunner` 会 fallback 到此分发器，将任务数据传递给插件系统。
//!
//! 插件通过在 `plugin.toml` 中声明 `[hooks.on-cron-tick]` 来接收定时任务。

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::plugins::HookPoint;
use crate::plugins::PluginManager;

use super::Job;

/// 插件 Cron 分发器
///
/// 将 Job 数据序列化后通过 `PluginManager::dispatch_action` 发送给
/// 声明了 `on_cron_tick` hook 的插件。
pub struct PluginCronDispatcher {
    plugins: Arc<PluginManager>,
}

impl PluginCronDispatcher {
    /// 创建分发器
    pub fn new(plugins: Arc<PluginManager>) -> Self {
        Self { plugins }
    }

    /// 将 Job 分发给插件
    pub async fn dispatch(&self, job: &Job) -> AppResult<()> {
        let payload = match job {
            Job::Custom { job_type, payload } => serde_json::json!({
                "job_type": job_type,
                "payload": payload,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }),
            _ => serde_json::json!({
                "job_type": job.job_type(),
                "payload": serde_json::to_value(job).unwrap_or_default(),
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }),
        };

        tracing::info!(
            "dispatching cron job to plugins: job_type={}",
            job.job_type()
        );

        self.plugins
            .dispatch_action(HookPoint::CronTick, &payload)
            .await;

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
