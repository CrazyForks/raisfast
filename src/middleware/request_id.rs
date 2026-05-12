//! Request ID middleware
//!
//! Generates a unique ID (UUID v7) for each incoming request and injects it into:
//!
//! * Response header `X-Request-ID`
//! * tracing span's `request_id` field
//!
//! Also records method/uri in the span for log correlation.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// Response header name
pub const HEADER_NAME: &str = "X-Request-ID";

/// Middleware that injects a Request ID into the request
pub async fn inject_request_id(req: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::now_v7().to_string();

    tracing::Span::current().record("request_id", &request_id);

    let mut response = next.run(req).await;

    if let Ok(val) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(HEADER_NAME, val);
    }

    response
}
