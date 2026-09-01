//! Short-session widget tokens (platform-signed JWTs for anonymous visitor
//! channels — widget.md §2/§3). Generic: any "anonymous short-session channel"
//! app (support widget, ticket receipts, feedback) can reuse this.
//!
//! Claims are minimal by design:
//!   `{ typ: "widget", sub: <contact_id>, ch: <channel_key>, iat, exp }`

use serde::{Deserialize, Serialize};

/// Minimal widget session claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetClaims {
    /// `"widget"` — token kind marker (distinguishes from login JWTs).
    pub typ: String,
    /// Contact id (numeric string; ID_ENCODING-agnostic).
    pub sub: String,
    /// Channel key the token is scoped to.
    pub ch: String,
    pub iat: usize,
    pub exp: usize,
}

/// Sign a widget session token with the platform JWT secret.
pub fn issue_widget_token(
    secret: &str,
    channel_key: &str,
    contact_id: &str,
    ttl_secs: u64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = crate::utils::tz::now_utc().timestamp();
    let claims = WidgetClaims {
        typ: "widget".into(),
        sub: contact_id.to_string(),
        ch: channel_key.to_string(),
        iat: now.max(0) as usize,
        exp: (now + ttl_secs as i64).max(0) as usize,
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
}

/// Verify a widget session token. Returns `None` for malformed / expired /
/// non-widget tokens (caller decides 401 vs 403).
pub fn verify_widget_token(secret: &str, token: &str) -> Option<WidgetClaims> {
    let data = jsonwebtoken::decode::<WidgetClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    )
    .ok()?;
    if data.claims.typ != "widget" {
        return None;
    }
    Some(data.claims)
}

/// Extract a `Bearer <token>` from the Authorization header (case-insensitive).
pub fn bearer_token(header: &str) -> Option<&str> {
    let lower = header.to_ascii_lowercase();
    lower
        .strip_prefix("bearer ")
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|_| {
            // Re-slice from the original (preserve case of the token body).
            let start = "bearer ".len();
            header[start..].trim()
        })
}
