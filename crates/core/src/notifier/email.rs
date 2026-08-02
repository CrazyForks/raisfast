//! Email sending implementations
//!
//! Supports 6 providers:
//!
//! | Provider | EMAIL_PROVIDER | Description |
//! |----------|---------------|-------------|
//! | `log` | Log placeholder (development) |
//! | `smtp` | lettre SMTP relay |
//! | `sendgrid` | SendGrid HTTP API |
//! | `resend` | Resend HTTP API |
//! | `aliyun` | Alibaba Cloud DirectMail HTTP API |
//! | `tencent` | Tencent Cloud SES HTTP API |

use crate::notifier::{EmailMessage, EmailSender};

// ── Log ──────────────────────────────────────────────────────

/// Log email sender (for development)
pub struct LogSender;

#[async_trait::async_trait]
impl EmailSender for LogSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        tracing::info!(
            "[email/log] to={} subject=\"{}\" html_len={}",
            msg.to,
            msg.subject,
            msg.html_body.len(),
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "log"
    }
}

// ── SMTP (lettre) ────────────────────────────────────────────

pub struct SmtpSender {
    host: String,
    port: u16,
    user: String,
    pass: String,
}

impl SmtpSender {
    #[must_use]
    pub fn new(host: &str, port: u16, user: &str, pass: &str) -> Self {
        Self {
            host: host.to_string(),
            port,
            user: user.to_string(),
            pass: pass.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl EmailSender for SmtpSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        use lettre::message::header::ContentType;
        use lettre::transport::smtp::authentication::Credentials;
        use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

        let email = Message::builder()
            .from(
                self.user
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid from address: {e}"))?,
            )
            .to(msg
                .to
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid to address: {e}"))?)
            .subject(&msg.subject)
            .header(ContentType::TEXT_HTML)
            .body(msg.html_body.clone())
            .map_err(|e| anyhow::anyhow!("failed to build email: {e}"))?;

        let creds = Credentials::new(self.user.clone(), self.pass.clone());

        let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.host)?
            .port(self.port)
            .credentials(creds)
            .timeout(Some(std::time::Duration::from_secs(30)))
            .build();

        mailer.send(email).await.map_err(|e| {
            tracing::error!("[email/smtp] send failed: {e}");
            anyhow::anyhow!("smtp send failed: {e}")
        })?;

        tracing::info!(
            "[email/smtp] sent to={} subject=\"{}\"",
            msg.to,
            msg.subject
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "smtp"
    }
}

// ── SendGrid ─────────────────────────────────────────────────

pub struct SendGridSender {
    api_key: String,
    from: String,
    from_name: Option<String>,
}

impl SendGridSender {
    #[must_use]
    pub fn new(api_key: String, from: String, from_name: Option<String>) -> Self {
        Self {
            api_key,
            from,
            from_name,
        }
    }
}

#[async_trait::async_trait]
impl EmailSender for SendGridSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        let client = crate::notifier::http_client();

        let mut payload = serde_json::json!({
            "personalizations": [{
                "to": [{ "email": msg.to }],
            }],
            "subject": msg.subject,
            "content": [{
                "type": "text/html",
                "value": msg.html_body,
            }],
            "from": {
                "email": self.from,
            },
        });

        if let Some(name) = &self.from_name {
            payload["from"]["name"] = serde_json::Value::String(name.clone());
        }

        let resp = client
            .post("https://api.sendgrid.com/v3/mail/send")
            .header(
                crate::constants::HEADER_AUTHORIZATION,
                format!("{}{}", crate::constants::AUTH_BEARER_PREFIX, self.api_key),
            )
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("[email/sendgrid] failed: status={status} body={body}");
            return Err(anyhow::anyhow!("sendgrid failed: status={status}"));
        }

        tracing::info!(
            "[email/sendgrid] sent to={} subject=\"{}\"",
            msg.to,
            msg.subject
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "sendgrid"
    }
}

// ── Resend ───────────────────────────────────────────────────

pub struct ResendSender {
    api_key: String,
    from: String,
    from_name: Option<String>,
}

impl ResendSender {
    #[must_use]
    pub fn new(api_key: String, from: String, from_name: Option<String>) -> Self {
        Self {
            api_key,
            from,
            from_name,
        }
    }
}

#[async_trait::async_trait]
impl EmailSender for ResendSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        let client = crate::notifier::http_client();

        let mut from_field = serde_json::json!({ "email": self.from });
        if let Some(name) = &self.from_name {
            from_field["name"] = serde_json::Value::String(name.clone());
        }

        let payload = serde_json::json!({
            "from": from_field,
            "to": [msg.to],
            "subject": msg.subject,
            "html": msg.html_body,
        });

        let resp = client
            .post("https://api.resend.com/emails")
            .header(
                crate::constants::HEADER_AUTHORIZATION,
                format!("{}{}", crate::constants::AUTH_BEARER_PREFIX, self.api_key),
            )
            .json(&payload)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("[email/resend] failed: status={status} body={body}");
            return Err(anyhow::anyhow!("resend failed: status={status}"));
        }

        tracing::info!(
            "[email/resend] sent to={} subject=\"{}\"",
            msg.to,
            msg.subject
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "resend"
    }
}

// ── Alibaba Cloud DirectMail ─────────────────────────────────

pub struct AliyunDirectMailSender {
    access_key_id: String,
    access_key_secret: String,
    from: String,
    from_name: Option<String>,
    region: String,
}

impl AliyunDirectMailSender {
    #[must_use]
    pub fn new(
        access_key_id: String,
        access_key_secret: String,
        from: String,
        from_name: Option<String>,
        region: String,
    ) -> Self {
        Self {
            access_key_id,
            access_key_secret,
            from,
            from_name,
            region,
        }
    }
}

#[async_trait::async_trait]
impl EmailSender for AliyunDirectMailSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        let client = crate::notifier::http_client();

        let endpoint = format!("https://dm.{}.aliyuncs.com", self.region);

        let mut params = vec![
            ("Action", "SingleSendMail".to_string()),
            ("Format", "JSON".to_string()),
            ("Version", "2015-11-23".to_string()),
            ("AccessKeyId", self.access_key_id.clone()),
            ("SignatureMethod", "HMAC-SHA1".to_string()),
            ("SignatureVersion", "1.0".to_string()),
            ("SignatureNonce", uuid::Uuid::new_v4().to_string()),
            (
                "Timestamp",
                chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            ),
            ("AccountName", self.from.clone()),
            ("AddressType", "1".to_string()),
            ("ReplyToAddress", "false".to_string()),
            ("ToAddress", msg.to.clone()),
            ("Subject", msg.subject.clone()),
            ("HtmlBody", msg.html_body.clone()),
        ];

        if let Some(name) = &self.from_name {
            params.push(("FromAlias", name.clone()));
        }

        params.sort_by(|a, b| a.0.cmp(b.0));

        let canonicalized = params
            .iter()
            .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let string_to_sign = format!("GET&%2F&{}", percent_encode(&canonicalized));
        let signature = hmac_sha1_sign(&self.access_key_secret, &string_to_sign);

        let url = format!(
            "{}?Signature={}&{}",
            endpoint,
            percent_encode(&signature),
            canonicalized,
        );

        let resp = client.get(&url).send().await?;
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            tracing::error!(
                "[email/aliyun] failed: status={status} body={}",
                &body[..body.len().min(200)]
            );
            return Err(anyhow::anyhow!("aliyun directmail failed: status={status}"));
        }

        tracing::info!(
            "[email/aliyun] sent to={} subject=\"{}\"",
            msg.to,
            msg.subject
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "aliyun"
    }
}

// ── Tencent Cloud SES ────────────────────────────────────────

pub struct TencentSesSender {
    secret_id: String,
    secret_key: String,
    from: String,
    from_name: Option<String>,
    region: String,
}

impl TencentSesSender {
    #[must_use]
    pub fn new(
        secret_id: String,
        secret_key: String,
        from: String,
        from_name: Option<String>,
        region: String,
    ) -> Self {
        Self {
            secret_id,
            secret_key,
            from,
            from_name,
            region,
        }
    }
}

#[async_trait::async_trait]
impl EmailSender for TencentSesSender {
    async fn send(&self, msg: &EmailMessage) -> anyhow::Result<()> {
        let client = crate::notifier::http_client();

        let host = format!("ses.{}.tencentcloudapi.com", self.region);
        let url = format!("https://{host}");

        let timestamp = chrono::Utc::now().timestamp().to_string();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();

        let payload = serde_json::json!({
            "FromEmailAddress": if let Some(name) = &self.from_name {
                format!("{name} <{}>", self.from)
            } else {
                self.from.clone()
            },
            "Destination": [msg.to],
            "Subject": msg.subject,
            "Html": msg.html_body,
        });

        let payload_str = serde_json::to_string(&payload)?;

        let content_type = "application/json; charset=utf-8";
        let host_header = host.as_str();
        let action = "SendEmail";
        let service = "ses";

        let canonical_request = format!(
            "POST\n/\n\ncontent-type:{content_type}\nhost:{host_header}\nx-tc-action:{action}\n\ncontent-type;host;x-tc-action\n{}",
            sha256_hex(payload_str.as_bytes()),
        );

        let credential_scope = format!("{date}/{service}/tc3_request");
        let signed_headers = "content-type;host;x-tc-action";
        let string_to_sign = format!(
            "TC3-HMAC-SHA256\n{timestamp}\n{credential_scope}\n{}",
            sha256_hex(canonical_request.as_bytes()),
        );

        let secret_date = hmac_sha256_raw(
            format!("TC3{}", self.secret_key).as_bytes(),
            date.as_bytes(),
        );
        let secret_service = hmac_sha256_raw(&secret_date, service.as_bytes());
        let secret_signing = hmac_sha256_raw(&secret_service, b"tc3_request");
        let signature = hmac_sha256_hex(&secret_signing, string_to_sign.as_bytes());

        let authorization = format!(
            "TC3-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            self.secret_id, credential_scope, signed_headers, signature,
        );

        let resp = client
            .post(&url)
            .header("Content-Type", content_type)
            .header("Host", host_header)
            .header("X-TC-Action", action)
            .header("X-TC-Version", "2020-12-29")
            .header("X-TC-Region", &self.region)
            .header("X-TC-Timestamp", &timestamp)
            .header("Authorization", authorization)
            .body(payload_str)
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            tracing::error!(
                "[email/tencent] failed: status={status} body={}",
                &body[..body.len().min(200)]
            );
            return Err(anyhow::anyhow!("tencent ses failed: status={status}"));
        }

        tracing::info!(
            "[email/tencent] sent to={} subject=\"{}\"",
            msg.to,
            msg.subject
        );
        Ok(())
    }

    fn name(&self) -> &'static str {
        "tencent"
    }
}

// ── Utility functions ────────────────────────────────────────

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

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256_raw(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(key)
        .unwrap_or_else(|_| panic!("HMAC-SHA256 accepts any key size"));
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    hex::encode(hmac_sha256_raw(key, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_msg() -> EmailMessage {
        EmailMessage {
            to: "test@example.com".into(),
            subject: "Test".into(),
            html_body: "<p>Hello</p>".into(),
            text_body: None,
        }
    }

    #[tokio::test]
    async fn log_sender_succeeds() {
        let sender = LogSender;
        assert!(sender.send(&test_msg()).await.is_ok());
        assert_eq!(sender.name(), "log");
    }

    #[test]
    fn sha256_hex_works() {
        let result = sha256_hex(b"hello");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hmac_sha256_hex_works() {
        let result = hmac_sha256_hex(b"key", b"data");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn percent_encode_works() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }
}
