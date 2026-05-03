//! 安全响应头中间件。
//!
//! 为每个 HTTP 响应注入安全相关的 HTTP 头，防止常见的 Web 攻击：
//!
//! - `X-Content-Type-Options: nosniff` — 阻止 MIME 嗅探
//! - `X-Frame-Options: DENY` — 阻止点击劫持
//! - `X-XSS-Protection: 0` — 禁用浏览器 XSS 过滤器（现代最佳实践）
//! - `Referrer-Policy: strict-origin-when-cross-origin` — 限制 Referer 泄露
//! - `Permissions-Policy` — 禁用不必要的浏览器 API
//! - HSTS（仅 HTTPS）— 强制 HTTPS 连接

use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use axum::extract::Request;

static X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
static X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
static X_XSS_PROTECTION: HeaderName = HeaderName::from_static("x-xss-protection");
static REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
static PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
static STRICT_TRANSPORT_SECURITY: HeaderName =
    HeaderName::from_static("strict-transport-security");

/// 安全响应头中间件。
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        X_CONTENT_TYPE_OPTIONS.clone(),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(X_FRAME_OPTIONS.clone(), HeaderValue::from_static("DENY"));
    headers.insert(X_XSS_PROTECTION.clone(), HeaderValue::from_static("0"));
    headers.insert(
        REFERRER_POLICY.clone(),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        PERMISSIONS_POLICY.clone(),
        HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), payment=(), usb=(), magnetometer=(), gyroscope=(), accelerometer=()",
        ),
    );

    response
}

/// HTTPS 安全响应头中间件（额外添加 HSTS）。
pub async fn security_headers_with_hsts(request: Request, next: Next) -> Response {
    let mut response = security_headers(request, next).await;
    response.headers_mut().insert(
        STRICT_TRANSPORT_SECURITY.clone(),
        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );
    response
}
