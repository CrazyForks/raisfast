//! Email Handlers
//!
//! Uses the [`EmailSender`] trait to send emails, rendering HTML content via the Tera template engine.

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::notifier::{EmailMessage, EmailSender};
use crate::worker::{Job, JobHandler};

/// Welcome email handler
pub struct SendWelcomeEmailHandler {
    config: Arc<AppConfig>,
    email_sender: Arc<dyn EmailSender>,
}

impl SendWelcomeEmailHandler {
    #[must_use]
    pub fn new(config: Arc<AppConfig>, email_sender: Arc<dyn EmailSender>) -> Self {
        Self {
            config,
            email_sender,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for SendWelcomeEmailHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::SendWelcomeEmail {
            user_id: _,
            email,
            username,
        } = job
        else {
            return Ok(());
        };

        let verify_url = String::new();
        let html = render_template(
            "welcome",
            tera::Context::from_serialize(serde_json::json!({
                "username": username,
                "site_name": &self.config.base_url,
                "verify_url": verify_url,
            }))
            .unwrap_or_default(),
        )
        .unwrap_or_default();

        let msg = EmailMessage {
            to: email.clone(),
            subject: format!("Welcome to {}!", self.config.base_url),
            html_body: if html.is_empty() {
                format!("<p>Welcome, {username}!</p>")
            } else {
                html
            },
            text_body: None,
        };

        if let Err(e) = self.email_sender.send(&msg).await {
            tracing::error!("[email] welcome email failed: {e}");
        }

        Ok(())
    }
}

/// Password reset email handler
pub struct SendPasswordResetEmailHandler {
    config: Arc<AppConfig>,
    email_sender: Arc<dyn EmailSender>,
}

impl SendPasswordResetEmailHandler {
    #[must_use]
    pub fn new(config: Arc<AppConfig>, email_sender: Arc<dyn EmailSender>) -> Self {
        Self {
            config,
            email_sender,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for SendPasswordResetEmailHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::SendPasswordResetEmail {
            user_id: _,
            email,
            reset_token,
        } = job
        else {
            return Ok(());
        };

        let reset_url = format!(
            "{}/auth/reset-password?token={reset_token}",
            self.config.base_url,
        );

        let html = render_template(
            "password-reset",
            tera::Context::from_serialize(serde_json::json!({
                "reset_url": &reset_url,
            }))
            .unwrap_or_default(),
        )
        .unwrap_or_default();

        let msg = EmailMessage {
            to: email.clone(),
            subject: "Reset Your Password".into(),
            html_body: if html.is_empty() {
                format!("<p>Reset: <a href=\"{reset_url}\">{reset_url}</a></p>")
            } else {
                html
            },
            text_body: None,
        };

        if let Err(e) = self.email_sender.send(&msg).await {
            tracing::error!("[email] password reset email failed: {e}");
        }

        Ok(())
    }
}

fn render_template(template_name: &str, ctx: tera::Context) -> Option<String> {
    let source = match template_name {
        "welcome" => include_str!("../../../templates/email/welcome.html"),
        "password-reset" => include_str!("../../../templates/email/password-reset.html"),
        "email-verification" => include_str!("../../../templates/email/email-verification.html"),
        _ => return None,
    };
    let mut tera = tera::Tera::default();
    tera.add_raw_template(template_name, source).ok()?;
    tera.render(template_name, &ctx).ok()
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
    async fn logs_welcome_email() {
        let (config, sender) = test_deps();
        let handler = SendWelcomeEmailHandler::new(config, sender);
        let job = Job::SendWelcomeEmail {
            user_id: SnowflakeId(1),
            email: "alice@example.com".into(),
            username: "alice".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn logs_password_reset_email() {
        let (config, sender) = test_deps();
        let handler = SendPasswordResetEmailHandler::new(config, sender);
        let job = Job::SendPasswordResetEmail {
            user_id: SnowflakeId(1),
            email: "alice@example.com".into(),
            reset_token: "abc123".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }
}
