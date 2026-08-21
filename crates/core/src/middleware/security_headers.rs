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

/// Domains of raisfast's own infrastructure (docs site & template gallery),
/// allowed by default because the shipped admin UI loads template data and
/// screenshots from them — first-party features must work out of the box.
const RAISFAST_ORIGINS: &str = "https://raisfast.com https://www.raisfast.com";

/// Build the Content-Security-Policy value. Precedence (simple → expert):
///
/// 1. **Default (nothing set)** — strict policy; only raisfast's own domains
///    are additionally allowed (built-in admin template gallery).
/// 2. **`CSP_ALLOW`** — space-separated extra origins, appended to
///    `script-src`, `connect-src`, `img-src` and `style-src`. Covers most
///    third-party needs (analytics, chat widgets, CDN scripts), e.g.
///    `CSP_ALLOW="https://static.cloudflareinsights.com https://cloudflareinsights.com"`.
/// 3. **`CSP`** — full policy override (verbatim header value, nginx-style)
///    for complete control; wins over everything above.
///
/// Cached for the process lifetime.
fn csp_value() -> &'static HeaderValue {
    static CSP: OnceLock<HeaderValue> = OnceLock::new();
    CSP.get_or_init(|| {
        resolve_csp(
            std::env::var("CSP").ok().as_deref(),
            std::env::var("CSP_ALLOW").ok().as_deref(),
        )
    })
}

/// Pure resolution of the policy header from the `CSP` / `CSP_ALLOW` values.
/// Deterministic and independent of the process environment (testable).
fn resolve_csp(csp: Option<&str>, allow: Option<&str>) -> HeaderValue {
    // 3. Full override (wins over CSP_ALLOW)
    if let Some(policy) = csp.map(str::trim).filter(|p| !p.is_empty()) {
        match HeaderValue::from_str(policy) {
            Ok(v) => return v,
            Err(e) => eprintln!(
                "warning: invalid CSP env var ({e}); falling back to the default policy. \
                 Hint: wrap values containing quotes in double quotes"
            ),
        }
    }

    // 2. Extra origins appended to the default policy, otherwise default.
    let extra = allow.unwrap_or_default().trim();
    HeaderValue::from_str(&build_csp(extra)).unwrap_or_else(|_| default_csp())
}

/// Compose the policy string from optional extra origins (pure; testable).
fn build_csp(extra: &str) -> String {
    if extra.is_empty() {
        return format!(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob: {RAISFAST_ORIGINS}; font-src 'self'; \
             connect-src 'self' {RAISFAST_ORIGINS}; frame-ancestors 'none'; \
             base-uri 'self'; form-action 'self'"
        );
    }
    format!(
        "default-src 'self'; script-src 'self' {extra}; style-src 'self' 'unsafe-inline' {extra}; \
         img-src 'self' data: blob: {RAISFAST_ORIGINS} {extra}; font-src 'self'; \
         connect-src 'self' {RAISFAST_ORIGINS} {extra}; frame-ancestors 'none'; \
         base-uri 'self'; form-action 'self'"
    )
}

fn default_csp() -> HeaderValue {
    // No unwrap/expect: build_csp only interpolates RAISFAST_ORIGINS / extra,
    // both of which must come from env; on invalid header chars fall back to
    // the static baseline below.
    HeaderValue::from_str(&build_csp("")).unwrap_or(HeaderValue::from_static(
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
    ))
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
    fn resolve_default_policy_without_any_env() {
        let v = resolve_csp(None, None);
        assert_eq!(v, default_csp());
    }

    #[test]
    fn default_policy_has_all_directives() {
        let s = build_csp("");
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

    #[test]
    fn default_policy_allows_raisfast_own_domains() {
        // The shipped admin template gallery loads templates.json and
        // screenshots from raisfast.com / www.raisfast.com — must be allowed
        // out of the box (connect-src + img-src).
        let s = build_csp("");
        assert!(
            s.contains("img-src 'self' data: blob: https://raisfast.com https://www.raisfast.com")
        );
        assert!(s.contains("connect-src 'self' https://raisfast.com https://www.raisfast.com"));
        // First-party domains are NOT allowed to run scripts by default.
        assert_eq!(
            s.split("script-src").nth(1).unwrap().split(';').next(),
            Some(" 'self'")
        );
    }

    #[test]
    fn csp_allow_appends_extra_origins_to_all_fetch_directives() {
        let s = build_csp("https://analytics.example.com");
        assert!(s.contains("script-src 'self' https://analytics.example.com"));
        assert!(s.contains("style-src 'self' 'unsafe-inline' https://analytics.example.com"));
        assert!(s.contains("connect-src 'self' https://raisfast.com https://www.raisfast.com https://analytics.example.com"));
        let base = build_csp("");
        assert!(!base.contains("example.com"));
    }

    #[test]
    fn full_csp_override_wins_over_csp_allow() {
        let v = resolve_csp(Some("default-src 'self'"), Some("https://x.example.com"));
        assert_eq!(v.to_str().unwrap(), "default-src 'self'");
    }

    #[test]
    fn empty_or_whitespace_csp_falls_back_to_allow_tier() {
        let v = resolve_csp(Some("   "), Some("https://analytics.example.com"));
        assert!(
            v.to_str()
                .unwrap()
                .contains("script-src 'self' https://analytics.example.com")
        );
    }

    #[test]
    fn invalid_csp_override_falls_back_to_default_policy() {
        // '\n' is not a legal header byte → override rejected, default used.
        let v = resolve_csp(Some("default-src 'self';\nscript-src *"), None);
        assert_eq!(v, default_csp());
    }
}
