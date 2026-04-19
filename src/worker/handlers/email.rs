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
        Arc::new(AppConfig::test_defaults())
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
