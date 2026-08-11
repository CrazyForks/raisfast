//! Security response headers middleware.
//!
//! Injects security-related HTTP headers into every HTTP response to prevent common web attacks:
//!
//! - `X-Content-Type-Options: nosniff` — Prevents MIME sniffing
//! - `X-Frame-Options: DENY` — Prevents clickjacking
//! - `X-XSS-Protection: 0` — Disables browser XSS filter (modern best practice)
//! - `Referrer-Policy: strict-origin-when-cross-origin` — Limits Referer leakage
//! - `Permissions-Policy` — Disables unnecessary browser APIs
//! - HSTS (HTTPS only) — Enforces HTTPS connections

use std::sync::OnceLock;

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

static X_CONTENT_TYPE_OPTIONS: HeaderName = HeaderName::from_static("x-content-type-options");
static X_FRAME_OPTIONS: HeaderName = HeaderName::from_static("x-frame-options");
static X_XSS_PROTECTION: HeaderName = HeaderName::from_static("x-xss-protection");
static REFERRER_POLICY: HeaderName = HeaderName::from_static("referrer-policy");
static PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
static STRICT_TRANSPORT_SECURITY: HeaderName = HeaderName::from_static("strict-transport-security");

static CONTENT_SECURITY_POLICY: HeaderName = HeaderName::from_static("content-security-policy");

/// Build the Content-Security-Policy value, merging in optional extra sources from:
/// - `CSP_SCRIPT_SRC`  — additional `script-src` origins (e.g. Cloudflare Insights)
/// - `CSP_CONNECT_SRC` — additional `connect-src` origins
/// - `CSP_STYLE_SRC`   — additional `style-src` origins
///
/// Each is a space-separated list of origins. Cached for the process lifetime.
fn csp_value() -> &'static HeaderValue {
    static CSP: OnceLock<HeaderValue> = OnceLock::new();
    CSP.get_or_init(|| {
        let script_extra = std::env::var("CSP_SCRIPT_SRC").unwrap_or_default();
        let connect_extra = std::env::var("CSP_CONNECT_SRC").unwrap_or_default();
        let style_extra = std::env::var("CSP_STYLE_SRC").unwrap_or_default();

        if script_extra.is_empty() && connect_extra.is_empty() && style_extra.is_empty() {
            return HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
            );
        }

        let script_src = if script_extra.is_empty() {
            "'self'".to_string()
        } else {
            format!("'self' {script_extra}")
        };
        let connect_src = if connect_extra.is_empty() {
            "'self'".to_string()
        } else {
            format!("'self' {connect_extra}")
        };
        let style_src = if style_extra.is_empty() {
            "'self' 'unsafe-inline'".to_string()
        } else {
            format!("'self' 'unsafe-inline' {style_extra}")
        };

        let policy = format!(
            "default-src 'self'; script-src {script_src}; style-src {style_src}; img-src 'self' data: blob:; font-src 'self'; connect-src {connect_src}; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
        );
        HeaderValue::from_str(&policy).expect("CSP header contains invalid characters")
    })
}

/// Security response headers middleware.
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
    headers.insert(CONTENT_SECURITY_POLICY.clone(), csp_value().clone());

    response
}

/// HTTPS security response headers middleware (adds HSTS additionally).
pub async fn security_headers_with_hsts(request: Request, next: Next) -> Response {
    let mut response = security_headers(request, next).await;
    response.headers_mut().insert(
        STRICT_TRANSPORT_SECURITY.clone(),
        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );
    response
}
