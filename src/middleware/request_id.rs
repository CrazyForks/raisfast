//! Request ID 中间件
//!
//! 为每个入站请求生成唯一 ID（UUID v7），注入到：
//!
//! * 响应头 `X-Request-ID`
//! * tracing span 的 `request_id` 字段
//!
//! 同时将 method/uri 记录到 span 中，便于日志关联。

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

/// 响应头名称
pub const HEADER_NAME: &str = "X-Request-ID";

/// 为请求注入 Request ID 的中间件
pub async fn inject_request_id(req: Request, next: Next) -> Response {
    let request_id = uuid::Uuid::now_v7().to_string();

    tracing::Span::current().record("request_id", &request_id);

    let mut response = next.run(req).await;

    if let Ok(val) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(HEADER_NAME, val);
    }

    response
}
