//! SMS verification code sending Handler

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::notifier::{SmsMessage, SmsSender};
use crate::worker::{Job, JobHandler};

/// SMS verification code sending handler
pub struct SendSmsCodeHandler {
    sms_sender: Arc<dyn SmsSender>,
}

impl SendSmsCodeHandler {
    #[must_use]
    pub fn new(sms_sender: Arc<dyn SmsSender>) -> Self {
        Self { sms_sender }
    }
}

#[async_trait::async_trait]
impl JobHandler for SendSmsCodeHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::SendSmsCode {
            phone,
            code,
            purpose,
        } = job
        else {
            return Ok(());
        };

        let msg = SmsMessage {
            to: phone.clone(),
            content: format!("Your verification code is: {code}. Valid for 5 minutes."),
            template_id: None,
            template_params: Some(serde_json::json!({ "code": code })),
        };

        let _ = purpose;
        if let Err(e) = self.sms_sender.send(&msg).await {
            tracing::error!("[sms] send failed to {phone}: {e}");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifier::sms::LogSender;

    #[tokio::test]
    async fn logs_sms_code() {
        let handler = SendSmsCodeHandler::new(Arc::new(LogSender));
        let job = Job::SendSmsCode {
            phone: "+8613800138000".into(),
            code: "123456".into(),
            purpose: "register".into(),
        };
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let handler = SendSmsCodeHandler::new(Arc::new(LogSender));
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}
