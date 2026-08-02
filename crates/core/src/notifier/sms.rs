//! SMS sending implementations
//!
//! - [`LogSender`]: Log placeholder for development
//! - [`AliyunSender`]: Alibaba Cloud SMS API
//! - [`TwilioSender`]: Twilio SMS API

use crate::notifier::{SmsMessage, SmsSender};

/// Log SMS sender (for development)
pub struct LogSender;

#[async_trait::async_trait]
impl SmsSender for LogSender {
    async fn send(&self, msg: &SmsMessage) -> anyhow::Result<()> {
        tracing::info!("[sms/log] to={} content=\"{}\"", msg.to, msg.content,);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "log"
    }
}

/// Alibaba Cloud SMS sender
pub struct AliyunSender {
    access_key_id: String,
    access_key_secret: String,
    sign_name: String,
    template_code: String,
}

impl AliyunSender {
    #[must_use]
    pub fn new(
        access_key_id: String,
        access_key_secret: String,
        sign_name: String,
        template_code: String,
    ) -> Self {
        Self {
            access_key_id,
            access_key_secret,
            sign_name,
            template_code,
        }
    }
}

#[async_trait::async_trait]
impl SmsSender for AliyunSender {
    async fn send(&self, msg: &SmsMessage) -> anyhow::Result<()> {
        let template_code = msg.template_id.as_deref().unwrap_or(&self.template_code);

        let params = msg
            .template_params
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!(r#"{{"code":"{}"}}"#, msg.content));

        let client = crate::notifier::http_client();

        let mut form = vec![
            ("Action", "SendSms".to_string()),
            ("Format", "JSON".to_string()),
            ("Version", "2017-05-25".to_string()),
            ("AccessKeyId", self.access_key_id.clone()),
            ("SignatureMethod", "HMAC-SHA1".to_string()),
            ("SignatureVersion", "1.0".to_string()),
            ("SignatureNonce", uuid::Uuid::new_v4().to_string()),
            (
                "Timestamp",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
            ("PhoneNumbers", msg.to.clone()),
            ("SignName", self.sign_name.clone()),
            ("TemplateCode", template_code.to_string()),
            ("TemplateParam", params),
        ];

        form.sort_by(|a, b| a.0.cmp(b.0));

        let canonicalized = form
            .iter()
            .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v),))
            .collect::<Vec<_>>()
            .join("&");

        let string_to_sign = format!("GET&%2F&{}", percent_encode(&canonicalized));

        let signature = hmac_sha1_sign(&self.access_key_secret, &string_to_sign);

        let url = format!(
            "https://dysmsapi.aliyuncs.com/?Signature={}&{}",
            percent_encode(&signature),
            canonicalized,
        );

        let resp = client.get(&url).send().await?;
        let body = resp.text().await?;

        tracing::info!(
            "[sms/aliyun] to={} response={}",
            msg.to,
            &body[..body.len().min(200)],
        );

        Ok(())
    }

    fn name(&self) -> &'static str {
        "aliyun"
    }
}

/// Twilio SMS sender
pub struct TwilioSender {
    account_sid: String,
    auth_token: String,
    from: String,
}

impl TwilioSender {
    #[must_use]
    pub fn new(account_sid: String, auth_token: String, from: String) -> Self {
        Self {
            account_sid,
            auth_token,
            from,
        }
    }
}

#[async_trait::async_trait]
impl SmsSender for TwilioSender {
    async fn send(&self, msg: &SmsMessage) -> anyhow::Result<()> {
        let client = crate::notifier::http_client();

        let url = format!(
            "https://api.twilio.com/2010-04-01/Accounts/{}/Messages.json",
            self.account_sid,
        );

        let resp = client
            .post(&url)
            .basic_auth(&self.account_sid, Some(&self.auth_token))
            .form(&[
                ("To", msg.to.as_str()),
                ("From", self.from.as_str()),
                ("Body", msg.content.as_str()),
            ])
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;

        if !status.is_success() {
            tracing::error!("[sms/twilio] send failed: status={status} body={body}");
            return Err(anyhow::anyhow!("twilio send failed: status={status}"));
        }

        tracing::info!("[sms/twilio] sent to={}", msg.to);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "twilio"
    }
}

fn percent_encode(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    result
}

fn hmac_sha1_sign(key: &str, data: &str) -> String {
    crate::utils::crypto::hmac_sha1_sign(key, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn log_sender_succeeds() {
        let sender = LogSender;
        let msg = SmsMessage {
            to: "+8613800138000".into(),
            content: "123456".into(),
            template_id: None,
            template_params: None,
        };
        assert!(sender.send(&msg).await.is_ok());
        assert_eq!(sender.name(), "log");
    }
}
