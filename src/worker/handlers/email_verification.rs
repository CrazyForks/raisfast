//! Email verification Handler

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::notifier::{EmailMessage, EmailSender};
use crate::worker::{Job, JobHandler};

pub struct SendEmailVerificationHandler {
    config: Arc<AppConfig>,
    email_sender: Arc<dyn EmailSender>,
}

impl SendEmailVerificationHandler {
    #[must_use]
    pub fn new(config: Arc<AppConfig>, email_sender: Arc<dyn EmailSender>) -> Self {
        Self {
            config,
            email_sender,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for SendEmailVerificationHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::SendEmailVerification {
            user_id: _,
            email,
            verify_token,
        } = job
        else {
            return Ok(());
        };

        let verify_url = format!(
            "{}/auth/verify-email?token={verify_token}",
            self.config.base_url,
        );

        let mut tera = tera::Tera::default();
        let source = include_str!("../../../templates/email/email-verification.html");
        let html = match tera.add_raw_template("email-verification", source) {
            Ok(_) => {
                let mut ctx = tera::Context::new();
                ctx.insert("verify_url", &verify_url);
                tera.render("email-verification", &ctx).unwrap_or_default()
            }
            Err(_) => {
                format!("<p>Verify your email: <a href=\"{verify_url}\">{verify_url}</a></p>")
            }
        };

        let msg = EmailMessage {
            to: email.clone(),
            subject: "Verify Your Email".into(),
            html_body: html,
            text_body: None,
        };

        if let Err(e) = self.email_sender.send(&msg).await {
            tracing::error!("[email] verification email failed: {e}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;
    use crate::notifier::email::LogSender;
    use crate::types::snowflake_id::SnowflakeId;

    fn test_deps() -> (Arc<AppConfig>, Arc<dyn EmailSender>) {
        (Arc::new(AppConfig::test_defaults()), Arc::new(LogSender))
    }

    #[tokio::test]
    async fn logs_verification_email() {
        let (config, sender) = test_deps();
        let handler = SendEmailVerificationHandler::new(config, sender);
        let job = Job::SendEmailVerification {
            user_id: SnowflakeId(1),
            email: "alice@example.com".into(),
            verify_token: "abc123".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let (config, sender) = test_deps();
        let handler = SendEmailVerificationHandler::new(config, sender);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}
