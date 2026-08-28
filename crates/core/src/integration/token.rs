//! Dynamic OAuth client-credentials tokens (integration.md §10.1).
//!
//! Third parties with expiring API tokens (Feishu `tenant_access_token`,
//! WeChat `access_token`, generic OAuth2 client-credentials) share one
//! provider: credentials carry the grant config, tokens are fetched from
//! `token_url` and cached until shortly before expiry. Both the stream
//! connectors (ws handshake) and the egress plane (bearer auth) resolve
//! through here, so one refresh serves every consumer.
//!
//! Credentials JSON shape (sealed in the vault like all credentials):
//! ```json
//! {
//!   "kind": "oauth-cc",
//!   "token_url": "https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal",
//!   "grant": { "app_id": "cli_x", "app_secret": "…" },
//!   "token_path": "tenant_access_token",
//!   "expire_path": "expire",
//!   "expire_default_secs": 7200
//! }
//! ```

use std::time::Duration;

use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};

/// token → expires_at (cache key = channel/client key).
static CACHE: std::sync::LazyLock<dashmap::DashMap<String, (String, std::time::Instant)>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

/// Refresh when less than this remains (whichever is larger of the two).
const MIN_MARGIN: Duration = Duration::from_secs(60);
const RATIO_MARGIN: f64 = 0.1;

/// Whether the credentials JSON describes an oauth-cc grant.
#[must_use]
pub fn is_oauth_cc(creds: &Value) -> bool {
    creds.get("kind").and_then(Value::as_str) == Some("oauth-cc")
}

/// Resolve a token for `cache_key`, fetching (or re-fetching) as needed.
///
/// # Errors
///
/// `AppError::BadRequest` on malformed grant config;
/// `AppError::Internal` on token-endpoint failure or missing token field.
pub async fn resolve_token(cache_key: &str, creds: &Value) -> AppResult<String> {
    if !is_oauth_cc(creds) {
        return Err(AppError::BadRequest(
            "oauth-cc credentials require {\"kind\":\"oauth-cc\", token_url, grant{...}}".into(),
        ));
    }
    let token_url = creds
        .get("token_url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("oauth-cc: missing token_url".into()))?;
    let grant = creds
        .get("grant")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    // Serve from cache while comfortably inside the validity window.
    if let Some(entry) = CACHE.get(cache_key) {
        let (token, expires_at) = entry.value();
        if std::time::Instant::now() < *expires_at {
            return Ok(token.clone());
        }
    }

    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth-cc http client: {e}")))?
        .post(token_url)
        .json(&grant)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth-cc token fetch: {e}")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth-cc token body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "oauth-cc token endpoint returned {status}: {body}"
        )));
    }
    let token = creds
        .get("token_path")
        .and_then(Value::as_str)
        .unwrap_or("access_token");
    let Some(token) = body.get(token).and_then(Value::as_str).map(str::to_string) else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "oauth-cc: response has no '{token}' field: {body}"
        )));
    };
    let expire_secs = match creds.get("expire_path").and_then(Value::as_str) {
        Some(path) => body.get(path).and_then(Value::as_i64).unwrap_or(0),
        None => body.get("expires_in").and_then(Value::as_i64).unwrap_or(0),
    };
    let expire_secs = if expire_secs <= 0 {
        creds
            .get("expire_default_secs")
            .and_then(Value::as_i64)
            .unwrap_or(3600)
    } else {
        expire_secs
    };
    let margin = MIN_MARGIN.max(Duration::from_secs(
        (expire_secs as f64 * RATIO_MARGIN) as u64,
    ));
    let ttl = Duration::from_secs(expire_secs.max(5) as u64).saturating_sub(margin);
    CACHE.insert(
        cache_key.to_string(),
        (token.clone(), std::time::Instant::now() + ttl),
    );
    tracing::debug!(cache_key, expire_secs, "oauth-cc token cached");
    Ok(token)
}

/// Render `{{var}}` placeholders in a protocol frame template from a
/// context map. Generic for any JSON-message protocol: dynamic token,
/// grant fields (app_id, …), or endpoint fragments.
///
/// Unknown placeholders are left as-is so misconfiguration is visible on
/// the wire (the peer rejects it) instead of silently empty.
#[must_use]
pub fn render_template(template: &str, vars: &serde_json::Map<String, Value>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        let rendered = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out = out.replace(&format!("{{{{{k}}}}}"), &rendered);
    }
    out
}

/// Flatten credentials into a template var map: resolved `token` plus every
/// `grant.*` field (app_id, app_secret, …) and top-level string fields.
#[must_use]
pub fn template_vars(token: Option<String>, creds: &Value) -> serde_json::Map<String, Value> {
    let mut vars = serde_json::Map::new();
    if let Some(token) = token {
        vars.insert("token".into(), Value::String(token));
    }
    if let Some(grant) = creds.get("grant").and_then(Value::as_object) {
        for (k, v) in grant {
            vars.insert(k.clone(), v.clone());
        }
    }
    vars
}

/// Drop the cached token for `cache_key` (e.g. after a 401 forces re-auth).
pub fn invalidate(cache_key: &str) {
    CACHE.remove(cache_key);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_oauth_cc_kind() {
        assert!(is_oauth_cc(&json!({"kind":"oauth-cc"})));
        assert!(!is_oauth_cc(&json!({"kind":"bearer"})));
        assert!(!is_oauth_cc(&json!({})));
    }

    #[tokio::test]
    async fn rejects_missing_token_url() {
        let err = resolve_token("t", &json!({"kind":"oauth-cc"})).await;
        assert!(err.is_err());
    }

    #[test]
    fn renders_template_vars() {
        let vars: serde_json::Map<String, Value> =
            [("token", json!("t-123")), ("app_id", json!("cli_x"))]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect();
        assert_eq!(
            render_template(
                r#"{"command":"connect","token":"{{token}}","app":"{{app_id}}","keep":"{{unknown}}"}"#,
                &vars
            ),
            r#"{"command":"connect","token":"t-123","app":"cli_x","keep":"{{unknown}}"}"#
        );
    }

    #[test]
    fn template_vars_flatten_grant() {
        let creds = json!({"kind":"oauth-cc","grant":{"app_id":"cli_1","app_secret":"s"}});
        let vars = template_vars(Some("t".into()), &creds);
        assert_eq!(vars["token"], "t");
        assert_eq!(vars["app_id"], "cli_1");
        assert_eq!(vars["app_secret"], "s");
    }
}
