//! Prometheus metrics 中间件
//!
//! 为每个 HTTP 请求记录：
//! - `http_requests_total` — 计数器（按 method + status + path 分组）
//! - `http_request_duration_seconds` — 直方图（按 method + path 分组）
//!
//! 初始化时启动 Prometheus recorder，通过全局 handle 渲染输出。

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use metrics::{counter, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::sync::OnceLock;
use std::time::Instant;

static HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();

/// 初始化 Prometheus recorder（调用一次）
pub fn init() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .unwrap_or_else(|e| panic!("prometheus recorder install failed: {e}"));
    HANDLE
        .set(handle)
        .expect("metrics::init called more than once");
}

/// 渲染 Prometheus 文本输出
pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// metrics 中间件：记录请求计数和延迟直方图
pub async fn track_metrics(req: Request, next: Next) -> Response {
    let method = req.method().clone().to_string();
    let path = normalize_path(req.uri().path());

    counter!("http_requests_total", "method" => method.clone(), "status" => "active", "path" => path.clone())
        .increment(1);

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed();

    let status = response.status().as_u16().to_string();

    counter!("http_requests_total", "method" => method.clone(), "status" => status, "path" => path.clone())
        .increment(1);

    histogram!("http_request_duration_seconds", "method" => method, "path" => path)
        .record(elapsed.as_secs_f64());

    response
}

/// Prometheus metrics 端点 handler
pub async fn metrics_endpoint() -> impl IntoResponse {
    let body = render();
    (
        StatusCode::OK,
        [("Content-Type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// 将路径标准化，将动态段替换为占位符以降低基数
fn normalize_path(path: &str) -> String {
    let segments: Vec<&str> = path.split('/').collect();
    let mut normalized: Vec<String> = Vec::with_capacity(segments.len());
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            normalized.push(String::new());
        } else if looks_like_id(seg) {
            if i > 0 {
                let prev = segments[i - 1];
                normalized.push(format!("{{{prev}}}"));
            } else {
                normalized.push("{id}".to_string());
            }
        } else {
            normalized.push((*seg).to_string());
        }
    }
    normalized.join("/")
}

/// 判断一段路径是否看起来像 ID（UUID、数字、长 hex）
fn looks_like_id(s: &str) -> bool {
    if s.len() >= 32 {
        return true;
    }
    if s.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && s.len() >= 8 {
        return true;
    }
    s.chars().all(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_plain_path() {
        assert_eq!(normalize_path("/api/v1/posts"), "/api/v1/posts");
    }

    #[test]
    fn normalize_uuid_segment() {
        assert_eq!(
            normalize_path("/api/v1/posts/019abc12-def4-7abc-8def-0123456789ab"),
            "/api/v1/posts/{posts}"
        );
    }

    #[test]
    fn normalize_numeric_segment() {
        assert_eq!(normalize_path("/api/v1/posts/42"), "/api/v1/posts/{posts}");
    }

    #[test]
    fn normalize_hex_segment() {
        assert_eq!(
            normalize_path("/api/v1/media/a1b2c3d4"),
            "/api/v1/media/{media}"
        );
    }

    #[test]
    fn normalize_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn looks_like_id_uuid() {
        assert!(looks_like_id("019abc12-def4-7abc-8def-0123456789ab"));
    }

    #[test]
    fn looks_like_id_numeric() {
        assert!(looks_like_id("42"));
    }

    #[test]
    fn looks_like_id_short_name() {
        assert!(!looks_like_id("posts"));
    }
}
