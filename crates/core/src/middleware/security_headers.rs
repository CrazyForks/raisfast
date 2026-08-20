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

/// Build the Content-Security-Policy value.
///
/// Set the `CSP` env var to override the built-in policy entirely (verbatim
/// header value, nginx-style). Typical additions: Cloudflare Insights beacon
/// (`script-src`/`connect-src`) and the admin template gallery (`connect-src`/
/// `img-src` for `https://raisfast.com`). Cached for the process lifetime.
fn csp_value() -> &'static HeaderValue {
    static CSP: OnceLock<HeaderValue> = OnceLock::new();
    CSP.get_or_init(|| match std::env::var("CSP") {
        Ok(policy) if !policy.trim().is_empty() => match HeaderValue::from_str(policy.trim()) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("invalid CSP env var ({e}), using default policy");
                default_csp()
            }
        },
        _ => default_csp(),
    })
}

fn default_csp() -> HeaderValue {
    HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csp_value_returns_strict_default_without_env() {
        // std::env::var reads the process env; the CSP var is unset in tests,
        // so this exercises the default branch.
        let default = default_csp();
        let v = csp_value();
        assert_eq!(v, default);
        assert!(v.to_str().unwrap().contains("script-src 'self'"));
    }

    #[test]
    fn default_csp_is_valid_directive_list() {
        let default = default_csp();
        let s = default.to_str().unwrap();
        for directive in [
            "default-src",
            "script-src",
            "style-src",
            "img-src",
            "font-src",
            "connect-src",
            "frame-ancestors",
            "base-uri",
            "form-action",
        ] {
            assert!(s.contains(directive), "missing {directive}");
        }
    }
}
