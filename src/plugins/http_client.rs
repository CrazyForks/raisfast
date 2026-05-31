//! Plugin HTTP client
//!
//! Provides restricted HTTP request capabilities for plugins.
//! Permission checking (domain whitelist) is handled by the Host layer; this module only executes HTTP requests.

use std::time::Duration;

use crate::errors::app_error::AppResult;

/// Single HTTP request timeout (seconds)
const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Maximum response body size (1 MB)
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// Execute an HTTP GET request
pub async fn http_get(url: &str) -> AppResult<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!("build http client: {e}"))
        })?;

    let response = client.get(url).send().await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("http get {url}: {e}"))
    })?;

    let status = response.status();
    let body = response.bytes().await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("read response: {e}"))
    })?;

    if body.len() > MAX_RESPONSE_BYTES {
        return Err(crate::errors::app_error::AppError::Internal(
            anyhow::anyhow!(
                "response too large: {} bytes (max {MAX_RESPONSE_BYTES})",
                body.len()
            ),
        ));
    }

    let body_str = String::from_utf8_lossy(&body).to_string();
    Ok(format!(
        "{{\"status\":{},\"body\":{}}}",
        status.as_u16(),
        serde_json::to_string(&body_str).unwrap_or_default()
    ))
}

/// Execute an HTTP POST request
pub async fn http_post(url: &str, body: &str, content_type: Option<&str>) -> AppResult<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
        .build()
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!("build http client: {e}"))
        })?;

    let ct = content_type.unwrap_or("application/json");
    let request = client
        .post(url)
        .header("Content-Type", ct)
        .body(body.to_string());

    let response = request.send().await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("http post {url}: {e}"))
    })?;

    let status = response.status();
    let resp_body = response.bytes().await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("read response: {e}"))
    })?;

    if resp_body.len() > MAX_RESPONSE_BYTES {
        return Err(crate::errors::app_error::AppError::Internal(
            anyhow::anyhow!(
                "response too large: {} bytes (max {MAX_RESPONSE_BYTES})",
                resp_body.len()
            ),
        ));
    }

    let body_str = String::from_utf8_lossy(&resp_body).to_string();
    Ok(format!(
        "{{\"status\":{},\"body\":{}}}",
        status.as_u16(),
        serde_json::to_string(&body_str).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn http_get_real_request() {
        let result = http_get("https://httpbin.org/get").await;
        assert!(result.is_ok());
        let body = result.unwrap();
        assert!(body.contains("\"status\":200"));
        assert!(body.contains("httpbin.org"));
    }

    #[tokio::test]
    #[ignore]
    async fn http_post_real_request() {
        let result = http_post(
            "https://httpbin.org/post",
            r#"{"hello":"world"}"#,
            Some("application/json"),
        )
        .await;
        assert!(result.is_ok());
        let body = result.unwrap();
        assert!(body.contains("\"status\":200"));
    }

    #[tokio::test]
    async fn http_get_invalid_url() {
        let result = http_get("http://192.0.2.1:1/impossible").await;
        assert!(result.is_err());
    }
}
