//! Notification module
//!
//! Generic email/SMS sending abstraction layer, with implementation selected via env config:
//!
//! - Email: `log` (log placeholder) | `smtp` (lettre)
//! - SMS: `log` (log placeholder) | `aliyun` (Alibaba Cloud SMS) | `twilio`

pub mod email;
pub mod sms;

/// HTTP request timeout for notification service calls (seconds)
const NOTIFICATION_TIMEOUT_SECS: u64 = 10;

/// Build a reqwest client with notification-appropriate timeout
pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(NOTIFICATION_TIMEOUT_SECS))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

use std::sync::Arc;

use serde_json::Value;

/// Email message
#[derive(Debug, Clone)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub html_body: String,
    pub text_body: Option<String>,
}

/// SMS message
#[derive(Debug, Clone)]
pub struct SmsMessage {
    pub to: String,
    pub content: String,
    pub template_id: Option<String>,
    pub template_params: Option<Value>,
}

/// Email sender trait
#[async_trait::async_trait]
pub trait EmailSender: Send + Sync {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()>;
    fn name(&self) -> &'static str;
}

/// SMS sender trait
#[async_trait::async_trait]
pub trait SmsSender: Send + Sync {
    async fn send(&self, msg: &SmsMessage) -> anyhow::Result<()>;
    fn name(&self) -> &'static str;
}

/// Build an email sender based on configuration
pub fn build_email_sender(config: &crate::config::app::AppConfig) -> Arc<dyn EmailSender> {
    match config.email_provider.as_str() {
        "smtp" => Arc::new(email::SmtpSender::new(
            config.email_smtp_host.as_deref().unwrap_or(""),
            config.email_smtp_port,
            config.email_smtp_user.as_deref().unwrap_or(""),
            config.email_smtp_pass.as_deref().unwrap_or(""),
        )),
        "sendgrid" => Arc::new(email::SendGridSender::new(
            config.email_sendgrid_api_key.clone().unwrap_or_default(),
            config.email_from.clone().unwrap_or_default(),
            config.email_from_name.clone(),
        )),
        "resend" => Arc::new(email::ResendSender::new(
            config.email_resend_api_key.clone().unwrap_or_default(),
            config.email_from.clone().unwrap_or_default(),
            config.email_from_name.clone(),
        )),
        "aliyun" => Arc::new(email::AliyunDirectMailSender::new(
            config
                .email_aliyun_access_key_id
                .clone()
                .unwrap_or_default(),
            config
                .email_aliyun_access_key_secret
                .clone()
                .unwrap_or_default(),
            config.email_from.clone().unwrap_or_default(),
            config.email_from_name.clone(),
            config
                .email_aliyun_region
                .clone()
                .unwrap_or_else(|| "cn-hangzhou".into()),
        )),
        "tencent" => Arc::new(email::TencentSesSender::new(
            config.email_tencent_secret_id.clone().unwrap_or_default(),
            config.email_tencent_secret_key.clone().unwrap_or_default(),
            config.email_from.clone().unwrap_or_default(),
            config.email_from_name.clone(),
            config
                .email_tencent_region
                .clone()
                .unwrap_or_else(|| "ap-guangzhou".into()),
        )),
        _ => Arc::new(email::LogSender),
    }
}

/// Build an SMS sender based on configuration
pub fn build_sms_sender(config: &crate::config::app::AppConfig) -> Arc<dyn SmsSender> {
    match config.sms_provider.as_str() {
        "aliyun" => Arc::new(sms::AliyunSender::new(
            config.sms_aliyun_access_key_id.clone().unwrap_or_default(),
            config
                .sms_aliyun_access_key_secret
                .clone()
                .unwrap_or_default(),
            config.sms_aliyun_sign_name.clone().unwrap_or_default(),
            config.sms_aliyun_template_code.clone().unwrap_or_default(),
        )),
        "twilio" => Arc::new(sms::TwilioSender::new(
            config.sms_twilio_account_sid.clone().unwrap_or_default(),
            config.sms_twilio_auth_token.clone().unwrap_or_default(),
            config.sms_twilio_from.clone().unwrap_or_default(),
        )),
        _ => Arc::new(sms::LogSender),
    }
}
