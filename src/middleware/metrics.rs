//! Prometheus metrics middleware
//!
//! For each HTTP request, records:
//! - `http_requests_total` — Counter (grouped by method + status + path)
//! - `http_request_duration_seconds` — Histogram (grouped by method + path)
//!
//! Initializes the Prometheus recorder at startup, rendered via a global handle.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use metrics::{counter, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::sync::OnceLock;
use std::time::Instant;

static HANDLE: OnceLock<metrics_exporter_prometheus::PrometheusHandle> = OnceLock::new();

/// Initialize the Prometheus recorder (call once)
pub fn init() {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .unwrap_or_else(|e| panic!("prometheus recorder install failed: {e}"));
    HANDLE
        .set(handle)
        .unwrap_or_else(|_| panic!("metrics::init called more than once"));
}

/// Render Prometheus text output
pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// Metrics middleware: records request counts and latency histograms
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

/// Prometheus metrics endpoint handler
pub async fn metrics_endpoint() -> impl IntoResponse {
    let body = render();
    (
        StatusCode::OK,
        [("Content-Type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// Normalize a path by replacing dynamic segments with placeholders to reduce cardinality
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

/// Determine if a path segment looks like an ID (UUID, numeric, long hex)
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
