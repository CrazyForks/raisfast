//! Webhook 通知 Handler
//!
//! 向外部 URL 发送 HTTP POST JSON 请求。
//! 超时 10 秒，不跟踪重定向，记录非 2xx 响应为错误（触发重试）。

use crate::errors::app_error::{AppError, AppResult};
use crate::worker::{Job, JobHandler};

/// Webhook 通知处理器
pub struct WebhookNotifyHandler;

impl Default for WebhookNotifyHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookNotifyHandler {
    /// 创建新的 Webhook 通知处理器
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl JobHandler for WebhookNotifyHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::WebhookNotify { url, payload } = job else {
            return Ok(());
        };

        let body = serde_json::to_string(payload).unwrap_or_default();

        tracing::info!("[webhook] POST {} body_len={}", url, body.len());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("build http client: {e}")))?;

        let resp = client
            .post(url)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("webhook POST {url}: {e}")))?;

        let status = resp.status();
        if status.is_success() {
            tracing::info!("[webhook] {} responded {}", url, status);
            Ok(())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(AppError::Internal(anyhow::anyhow!(
                "webhook {} returned {}: {}",
                url,
                status,
                text.chars().take(200).collect::<String>()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let handler = WebhookNotifyHandler::new();
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn fails_on_invalid_url() {
        let handler = WebhookNotifyHandler::new();
        let job = Job::WebhookNotify {
            url: "http://0.0.0.0:1/impossible".into(),
            payload: json!({"test": true}),
        };
        assert!(handler.handle(&job).await.is_err());
    }
}
