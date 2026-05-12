//! Locale detection middleware
//!
//! This module detects and binds a locale for each HTTP request, used for subsequent
//! i18n message translation (e.g. error messages). Uses [`tokio::task_local!`] to pass
//! the locale within the request lifecycle, avoiding explicit injection in handler signatures.
//!
//! # Language detection priority
//!
//! 1. URL query parameter `?lang=` (e.g. `?lang=zh-CN`)
//! 2. `Accept-Language` request header (follows RFC 7231 q-value weighting)
//! 3. Default value `"en"`

use std::cmp::Ordering;

use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

// The locale for the current request.
//
// Set by [`locale_middleware`] at the request entry point, readable via [`current_locale`].
// Implemented with `tokio::task_local!` to ensure the locale is always available
// within the same request's async call chain without manual parameter passing.
tokio::task_local! {
    static CURRENT_LOCALE: String;
}

/// Get the locale for the current request.
///
/// Reads the locale from the task-local context; if called outside a request scope
/// (e.g. background tasks, tests), falls back to the default value `"en"`.
#[must_use]
pub fn current_locale() -> String {
    CURRENT_LOCALE
        .try_with(std::clone::Clone::clone)
        .unwrap_or_else(|_| "en".to_string())
}

/// Detect locale based on request information.
///
/// Detection priority:
/// 1. `?lang=` query parameter — if the value is in the supported list, use it directly
/// 2. `Accept-Language` request header — select the best match by q-value weighting
/// 3. Default `"en"`
pub fn detect_locale(req: &Request) -> String {
    if let Some(lang) = req.uri().query().and_then(|q| {
        q.split('&')
            .filter_map(|pair| pair.split_once('='))
            .find(|(k, _)| *k == "lang")
            .map(|(_, v)| v.to_string())
    }) {
        let lang = lang.to_lowercase();
        if ["zh-cn", "zh-tw", "zh", "en", "ja", "ko"].contains(&lang.as_str()) {
            return normalize_locale(&lang);
        }
    }

    req.headers()
        .get("accept-language")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_accept_language)
        .unwrap_or_else(|| "en".to_string())
}

/// Parse RFC 7231 `Accept-Language` request header.
///
/// Splits the header value by comma into multiple entries, parsing each entry's
/// language tag and optional `q` quality value (default `q=1.0`),
/// returning the highest-quality language tag (normalized via [`normalize_locale`]).
///
/// # Example
///
/// Input `"zh-CN,zh;q=0.9,en;q=0.8"` returns `"zh-CN"`.
fn parse_accept_language(header: &str) -> Option<String> {
    header
        .split(',')
        .filter_map(|part| {
            let (lang, quality) = if let Some((l, q)) = part.trim().split_once(';') {
                let quality = q
                    .trim()
                    .strip_prefix("q=")
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(1.0);
                (l.trim().to_lowercase(), quality)
            } else {
                (part.trim().to_lowercase(), 1.0)
            };
            if lang.is_empty() {
                None
            } else {
                Some((normalize_locale(&lang), quality))
            }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
        .map(|(lang, _)| lang)
}

/// Normalize a language tag to the project's standard form.
///
/// # Mapping rules
///
/// | Input | Output |
/// |---|---|
/// | `zh`, `zh-cn`, `zh-hans` | `zh-CN` |
/// | `zh-tw`, `zh-hant` | `zh-TW` |
/// | `en`, `en-us`, `en-gb` | `en` |
/// | `ja` | `ja` |
/// | `ko` | `ko` |
/// | Other | Returned as-is |
fn normalize_locale(lang: &str) -> String {
    match lang {
        "zh" | "zh-cn" | "zh-hans" => "zh-CN".to_string(),
        "zh-tw" | "zh-hant" => "zh-TW".to_string(),
        "en" | "en-us" | "en-gb" => "en".to_string(),
        "ja" => "ja".to_string(),
        "ko" => "ko".to_string(),
        other => other.to_string(),
    }
}

/// Axum locale detection middleware.
///
/// Calls [`detect_locale`] for each request to determine the language, then binds it
/// to the task-local context via [`CURRENT_LOCALE::scope`], so that subsequent handler
/// and service layers can retrieve the current locale via [`current_locale`].
pub async fn locale_middleware(req: Request, next: Next) -> Response {
    let locale = detect_locale(&req);
    rust_i18n::set_locale(&locale);
    CURRENT_LOCALE.scope(locale, next.run(req)).await
}
