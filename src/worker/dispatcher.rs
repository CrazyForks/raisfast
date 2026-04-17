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
        let config = Arc::new(crate::config::app::AppConfig {
            host: "0.0.0.0".into(),
            port: 3000,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters!".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "./uploads".into(),
            max_upload_size: 5242880,
            static_dir: "./static".into(),
            base_url: "http://localhost:3000".into(),
            cors_origins: None,
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 32,
            plugin_default_timeout_ms: 5000,
            plugin_disabled: vec![],
            plugin_vfs_root: "./plugins-data".into(),
            plugin_vfs_max_file_size: 1048576,
            plugin_vfs_max_total_size: 10485760,
            log_dir: "./logs".into(),
            log_max_files: 7,
            rate_limit_global_max: 60,
            rate_limit_global_window: 60,
            rate_limit_register_max: 5,
            rate_limit_register_window: 3600,
            rate_limit_login_max: 10,
            rate_limit_login_window: 60,
            rate_limit_comment_max: 3,
            rate_limit_comment_window: 60,
            worker_enabled: false,
            worker_concurrency: 2,
            worker_poll_interval_ms: 500,
            worker_default_max_attempts: 3,
            worker_cron_tick_ms: 60000,
            cron_seed_enabled: false,
            cron_schedules: vec![],
            cron_log_retention_days: 30,
            search_engine: "none".into(),
            search_index_dir: "./data/search_index".into(),
            content_type_dir: "./content_types".into(),
            timezone: "UTC".into(),
            extension_dir: "./extensions".into(),
            protected_tables: vec![],
        });
        let mgr = PluginManager::new_with_options(
            config,
            crate::plugins::PluginManagerOptions { pool: None },
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
