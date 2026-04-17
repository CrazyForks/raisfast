//! 欢迎邮件 Handler
//!
//! 开发阶段仅记录日志。生产环境需接入 SMTP（如 `lettre` crate）或外部邮件 API（如 `SendGrid`）。
//!
//! # 配置
//!
//! 启用 SMTP 需在 `AppConfig` 中新增：
//! ```text
//! SMTP_HOST / SMTP_PORT / SMTP_USER / SMTP_PASS / SMTP_FROM
//! ```

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// 欢迎邮件处理器
pub struct SendWelcomeEmailHandler {
    #[allow(dead_code)]
    config: Arc<AppConfig>,
}

impl SendWelcomeEmailHandler {
    /// 创建新的欢迎邮件处理器
    #[must_use]
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl JobHandler for SendWelcomeEmailHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::SendWelcomeEmail {
            user_id,
            email,
            username,
        } = job
        else {
            return Ok(());
        };

        tracing::info!(
            "[email] sending welcome email to user={} email={} username={}",
            user_id,
            email,
            username,
        );

        // TODO: 生产环境替换为真实 SMTP 发送
        // let smtp = &self.config.smtp_host;
        // lettre::SmtpTransport::relay(smtp)?
        //     .build()
        //     .send(message)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "/tmp/test-uploads".into(),
            max_upload_size: 5242880,
            static_dir: "./static".into(),
            base_url: "http://localhost:9000".into(),
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
            rate_limit_api_token_max: 120,
            rate_limit_api_token_window: 60,
            worker_enabled: false,
            worker_concurrency: 1,
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
        })
    }

    #[tokio::test]
    async fn logs_welcome_email() {
        let handler = SendWelcomeEmailHandler::new(test_config());
        let job = Job::SendWelcomeEmail {
            user_id: "u1".into(),
            email: "alice@example.com".into(),
            username: "alice".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let handler = SendWelcomeEmailHandler::new(test_config());
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}
