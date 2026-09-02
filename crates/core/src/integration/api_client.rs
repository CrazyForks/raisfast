//! ItgApiClient — declarative outbound API client (`itg_api_clients`,
//! integration.md §9.2). "Outbound channel is a data row": ops describe
//! method/path, auth + sealed credentials give trust, every call is logged
//! to `itg_egress_log` with the ambient trace id.

use serde::Serialize;
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// API client row (`itg_api_clients`).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ItgApiClient {
    pub id: SnowflakeId,
    pub tenant_id: String,
    /// Outbound routing key — callers reference `plane.send(client_key, op)`.
    pub client_key: String,
    pub display_name: String,
    /// Base URL prepended to every op path (`https://dify.internal/v1`).
    pub base_url: String,
    /// Auth injection config: `{kind: bearer|api-key-header|none, header?, prefix?}`.
    pub auth: Option<Value>,
    /// Sealed credentials (AES-256-GCM, base64). Never returned via API.
    #[serde(skip_serializing)]
    pub credentials: Option<String>,
    /// Fixed-window limit: `{per_minute}` (default 120).
    pub rate_limit: Option<Value>,
    /// Op templates: `{op: {method, path, output?}}`.
    pub ops: Option<Value>,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ItgApiClient {
    /// One op template, resolved from `ops` by name.
    #[must_use]
    pub fn op(&self, op: &str) -> Option<&serde_json::Map<String, Value>> {
        self.ops.as_ref()?.get(op)?.as_object()
    }

    /// Auth kind (`bearer` | `api-key-header` | `url-path-token` | `oauth2-auth-code` | `none`).
    #[must_use]
    pub fn auth_kind(&self) -> &str {
        self.auth
            .as_ref()
            .and_then(|a| a.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("none")
    }

    /// OAuth2 authorization-code redirect URI (validated at create/update).
    ///
    /// # Errors
    ///
    /// `BadRequest` when the client isn't an `oauth2-auth-code` client.
    pub fn redirect_uri(&self) -> AppResult<String> {
        self.auth
            .as_ref()
            .and_then(|a| a.get("redirect_uri"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AppError::BadRequest("auth requires a 'redirect_uri'".into()))
    }
}

/// Model-level CRUD (pool-based; admin-facing).
pub mod model {
    use super::ItgApiClient;
    use crate::errors::app_error::AppResult;
    use crate::types::snowflake_id::SnowflakeId;

    pub async fn insert(pool: &crate::db::Pool, c: &ItgApiClient) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        raisfast_derive::crud_insert!(
            pool,
            "itg_api_clients",
            [
                "id" => c.id,
                "client_key" => &c.client_key,
                "display_name" => &c.display_name,
                "base_url" => &c.base_url,
                "auth" => c.auth.as_ref(),
                "credentials" => c.credentials.as_deref(),
                "rate_limit" => c.rate_limit.as_ref(),
                "ops" => c.ops.as_ref(),
                "enabled" => c.enabled,
                "created_at" => now,
                "updated_at" => now
            ],
            tenant: Some(c.tenant_id.as_str())
        )?;
        Ok(())
    }

    pub async fn find_by_key(
        pool: &crate::db::Pool,
        tenant_id: &str,
        client_key: &str,
    ) -> AppResult<Option<ItgApiClient>> {
        Ok(
            raisfast_derive::crud_find!(pool, "itg_api_clients", ItgApiClient,
                where: ("client_key", client_key),
                tenant: Some(tenant_id))?,
        )
    }

    pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<ItgApiClient> {
        Ok(
            raisfast_derive::crud_find_one!(pool, "itg_api_clients", ItgApiClient, where: ("id", id))?,
        )
    }

    pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<ItgApiClient>> {
        // Hand-written SQL: `crud_find_all!` requires a `where:` section and we
        // genuinely want the full table (clients are few).
        const CLIENT_COLS: &str = "id, tenant_id, client_key, display_name, base_url, auth, \
             credentials, rate_limit, ops, enabled, created_at, updated_at";
        let sql = format!("SELECT {CLIENT_COLS} FROM itg_api_clients ORDER BY id");
        let rows: Vec<ItgApiClient> =
            sqlx::query_as::<crate::db::pool::Db, ItgApiClient>(crate::db::safe_sql(&sql))
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    pub async fn delete_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
        let result = raisfast_derive::crud_delete!(pool, "itg_api_clients", where: ("id", id))?;
        crate::errors::app_error::AppError::expect_affected(&result, "itg_api_client")?;
        Ok(())
    }
}

/// Validate the client config at save time (cross-field, no DB).
///
/// # Errors
///
/// `AppError::BadRequest` on malformed base_url / auth / ops.
pub fn validate(base_url: &str, auth: Option<&Value>, ops: Option<&Value>) -> AppResult<()> {
    let url = reqwest::Url::parse(base_url)
        .map_err(|e| AppError::BadRequest(format!("invalid base_url: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::BadRequest("base_url must be http(s)".into()));
    }
    if let Some(auth) = auth {
        let kind = auth.get("kind").and_then(Value::as_str).unwrap_or("none");
        match kind {
            "none" => {}
            "bearer" | "basic" => {
                if auth.get("header").is_some() {
                    return Err(AppError::BadRequest(
                        "auth kind 'bearer' does not take a 'header'".into(),
                    ));
                }
            }
            "api-key-header" => {
                let header = auth
                    .get("header")
                    .and_then(Value::as_str)
                    .unwrap_or("X-API-Key");
                if header.is_empty() || header.contains(':') {
                    return Err(AppError::BadRequest(
                        "auth 'header' must be a non-empty header name".into(),
                    ));
                }
            }
            "url-path-token" => {
                // The sealed secret is injected into the URL path (e.g. Telegram
                // `/bot<token>/sendMessage`). Requires a non-empty `path_prefix`
                // starting with `/`; the secret is appended right after it.
                let prefix = auth
                    .get("path_prefix")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if prefix.is_empty() || !prefix.starts_with('/') || prefix.ends_with('/') {
                    return Err(AppError::BadRequest(
                        "auth 'path_prefix' must start with '/' and not end with '/' \
                         (e.g. \"/bot\")"
                            .into(),
                    ));
                }
            }
            "oauth2-auth-code" => {
                // 3-legged OAuth: the static config lives here (sealed with the
                // client's credentials); the dynamic token is persisted per
                // (client, tenant) via itg_oauth_tokens (oauth2-egress.md §2).
                // `auth` here only declares the header kind for injection.
                let url = auth
                    .get("redirect_uri")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if url.is_empty() {
                    return Err(AppError::BadRequest(
                        "auth kind 'oauth2-auth-code' requires a 'redirect_uri'".into(),
                    ));
                }
            }
            other => {
                return Err(AppError::BadRequest(format!(
                    "auth kind '{other}' not supported (bearer | basic | api-key-header | url-path-token | oauth2-auth-code | none)"
                )));
            }
        }
    }
    if let Some(ops) = ops {
        let Some(map) = ops.as_object() else {
            return Err(AppError::BadRequest(
                "ops must be an object of {op: {method, path}}".into(),
            ));
        };
        for (name, op) in map {
            let method = op.get("method").and_then(Value::as_str).unwrap_or("GET");
            if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
                return Err(AppError::BadRequest(format!(
                    "op '{name}': method '{method}' not supported"
                )));
            }
            let path = op
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::BadRequest(format!("op '{name}' requires a 'path'")))?;
            if !path.starts_with('/') {
                return Err(AppError::BadRequest(format!(
                    "op '{name}': path must start with '/'"
                )));
            }
            if let Some(query) = op.get("query") {
                let Some(qobj) = query.as_object() else {
                    return Err(AppError::BadRequest(format!(
                        "op '{name}': query must be an object"
                    )));
                };
                for (k, v) in qobj {
                    if !v.is_string() && !v.is_number() && !v.is_boolean() {
                        return Err(AppError::BadRequest(format!(
                            "op '{name}': query '{k}' must be a scalar"
                        )));
                    }
                }
            }
            if let Some(headers) = op.get("headers") {
                let Some(hobj) = headers.as_object() else {
                    return Err(AppError::BadRequest(format!(
                        "op '{name}': headers must be an object"
                    )));
                };
                for (k, v) in hobj {
                    reqwest::header::HeaderName::try_from(k.as_str()).map_err(|_| {
                        AppError::BadRequest(format!("op '{name}': invalid header name '{k}'"))
                    })?;
                    if !v.is_string() {
                        return Err(AppError::BadRequest(format!(
                            "op '{name}': header '{k}' must be a string"
                        )));
                    }
                }
            }
            let body_shapes = ["body", "form", "multipart"]
                .iter()
                .filter(|k| op.get(*k).is_some())
                .count();
            if body_shapes > 1 {
                return Err(AppError::BadRequest(format!(
                    "op '{name}': only one of body/form/multipart may be set"
                )));
            }
            if let Some(form) = op.get("form") {
                let Some(fobj) = form.as_object() else {
                    return Err(AppError::BadRequest(format!(
                        "op '{name}': form must be an object"
                    )));
                };
                for (k, v) in fobj {
                    if !v.is_string() && !v.is_number() && !v.is_boolean() {
                        return Err(AppError::BadRequest(format!(
                            "op '{name}': form '{k}' must be a scalar"
                        )));
                    }
                }
            }
            if let Some(mp) = op.get("multipart") {
                let Some(mobj) = mp.as_object() else {
                    return Err(AppError::BadRequest(format!(
                        "op '{name}': multipart must be an object"
                    )));
                };
                if let Some(text) = mobj.get("text") {
                    let Some(tobj) = text.as_object() else {
                        return Err(AppError::BadRequest(format!(
                            "op '{name}': multipart.text must be an object"
                        )));
                    };
                    for (k, v) in tobj {
                        if !v.is_string() && !v.is_number() && !v.is_boolean() {
                            return Err(AppError::BadRequest(format!(
                                "op '{name}': multipart.text '{k}' must be a scalar"
                            )));
                        }
                    }
                }
                if let Some(files) = mobj.get("files") {
                    let Some(fobj) = files.as_object() else {
                        return Err(AppError::BadRequest(format!(
                            "op '{name}': multipart.files must be an object"
                        )));
                    };
                    for (k, v) in fobj {
                        let Some(ff) = v.as_object() else {
                            return Err(AppError::BadRequest(format!(
                                "op '{name}': multipart file '{k}' must be an object"
                            )));
                        };
                        for field in ["filename", "content_type", "content"] {
                            if !ff
                                .get(field)
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty())
                            {
                                return Err(AppError::BadRequest(format!(
                                    "op '{name}': multipart file '{k}' requires a string '{field}'"
                                )));
                            }
                        }
                    }
                }
            }
            if let Some(sig) = op.get("signature") {
                validate_signature(name, sig)?;
            }
        }
    }
    Ok(())
}

/// Validate an `op.signature` recipe (egress-signature.md §3). Only the
/// structural shape — actual template variables resolve at call time.
fn validate_signature(op_name: &str, sig: &Value) -> AppResult<()> {
    let Some(obj) = sig.as_object() else {
        return Err(AppError::BadRequest(format!(
            "op '{op_name}': signature must be an object"
        )));
    };
    for required in ["canonical_template", "string_to_sign_template", "inject"] {
        if !obj.contains_key(required) {
            return Err(AppError::BadRequest(format!(
                "op '{op_name}': signature requires '{required}'"
            )));
        }
    }
    if let Some(alg) = obj
        .get("algorithm")
        .and_then(Value::as_str)
        .filter(|a| !matches!(*a, "hmac-sha256" | "hmac-sha1"))
    {
        return Err(AppError::BadRequest(format!(
            "op '{op_name}': signature algorithm '{alg}' not supported (hmac-sha256 | hmac-sha1)"
        )));
    }
    if let Some(enc) = obj
        .get("encoding")
        .and_then(Value::as_str)
        .filter(|e| !matches!(*e, "hex" | "base64"))
    {
        return Err(AppError::BadRequest(format!(
            "op '{op_name}': signature encoding '{enc}' not supported (hex | base64)"
        )));
    }
    if let Some(key) = obj.get("key") {
        let Some(kind) = key.get("type").and_then(Value::as_str) else {
            return Err(AppError::BadRequest(format!(
                "op '{op_name}': signature.key requires a 'type' (hmac_chain | secret)"
            )));
        };
        match kind {
            "hmac_chain" => {
                if !key
                    .get("steps")
                    .and_then(Value::as_array)
                    .is_some_and(|s| !s.is_empty())
                {
                    return Err(AppError::BadRequest(format!(
                        "op '{op_name}': signature.key.hmac_chain requires non-empty 'steps'"
                    )));
                }
            }
            "secret" => {}
            other => {
                return Err(AppError::BadRequest(format!(
                    "op '{op_name}': signature.key type '{other}' not supported (hmac_chain | secret)"
                )));
            }
        }
    }
    let Some(inject) = obj.get("inject").and_then(Value::as_object) else {
        return Err(AppError::BadRequest(format!(
            "op '{op_name}': signature.inject must be an object"
        )));
    };
    let into = inject
        .get("into")
        .and_then(Value::as_str)
        .unwrap_or("header");
    match into {
        "header" => {
            if !inject.contains_key("template") {
                return Err(AppError::BadRequest(format!(
                    "op '{op_name}': signature.inject.header requires a 'template'"
                )));
            }
        }
        "query" => {
            if !inject.contains_key("query_param") {
                return Err(AppError::BadRequest(format!(
                    "op '{op_name}': signature.inject.query requires a 'query_param'"
                )));
            }
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "op '{op_name}': signature.inject.into '{other}' not supported (header | query)"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_accepts_url_path_token_auth() {
        let auth = serde_json::json!({ "kind": "url-path-token", "path_prefix": "/bot" });
        assert!(validate("https://api.telegram.org", Some(&auth), None).is_ok());
    }

    #[test]
    fn validate_rejects_bad_path_prefix() {
        for bad in [
            serde_json::json!({ "kind": "url-path-token", "path_prefix": "" }),
            serde_json::json!({ "kind": "url-path-token", "path_prefix": "bot" }),
            serde_json::json!({ "kind": "url-path-token", "path_prefix": "/bot/" }),
        ] {
            assert!(
                validate("https://api.telegram.org", Some(&bad), None).is_err(),
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn validate_rejects_unknown_auth_kind() {
        let auth = serde_json::json!({ "kind": "query-token" });
        assert!(validate("https://api.telegram.org", Some(&auth), None).is_err());
    }

    #[test]
    fn validate_accepts_basic_auth() {
        let auth = serde_json::json!({ "kind": "basic" });
        assert!(validate("https://api.example.org", Some(&auth), None).is_ok());
    }

    #[test]
    fn validate_op_query_headers_shapes() {
        let good = serde_json::json!({
            "search": {
                "method": "GET",
                "path": "/search",
                "query": {"q": "{q}", "limit": 10},
                "headers": {"X-Trace": "t-{trace}"}
            }
        });
        assert!(validate("https://api.example.org", None, Some(&good)).is_ok());

        let bad_query = serde_json::json!({
            "op": {"method": "GET", "path": "/x", "query": {"nested": {"a": 1}}}
        });
        assert!(validate("https://api.example.org", None, Some(&bad_query)).is_err());

        let bad_header = serde_json::json!({
            "op": {"method": "GET", "path": "/x", "headers": {"bad name": "v"}}
        });
        assert!(validate("https://api.example.org", None, Some(&bad_header)).is_err());
    }

    #[test]
    fn validate_op_form_and_multipart_shapes() {
        let form = serde_json::json!({
            "op": {"method": "POST", "path": "/token",
                   "form": {"grant_type": "authorization_code", "code": "{code}"}}
        });
        assert!(validate("https://api.example.org", None, Some(&form)).is_ok());

        let multipart = serde_json::json!({
            "op": {"method": "POST", "path": "/upload",
                   "multipart": {
                       "text": {"caption": "hi {user}"},
                       "files": {"file": {"filename": "{name}.png", "content_type": "image/png", "content": "{b64}"}}
                   }}
        });
        assert!(validate("https://api.example.org", None, Some(&multipart)).is_ok());

        // body+form mutually exclusive
        let both = serde_json::json!({
            "op": {"method": "POST", "path": "/x", "body": {"a": 1}, "form": {"b": "2"}}
        });
        assert!(validate("https://api.example.org", None, Some(&both)).is_err());

        // multipart file missing content
        let bad_file = serde_json::json!({
            "op": {"method": "POST", "path": "/upload",
                   "multipart": {"files": {"f": {"filename": "a.txt"}}}}
        });
        assert!(validate("https://api.example.org", None, Some(&bad_file)).is_err());
    }

    #[test]
    fn validate_oauth2_auth_code_requires_redirect_uri() {
        let ok = serde_json::json!({ "kind": "oauth2-auth-code", "redirect_uri": "https://h/cb" });
        assert!(validate("https://s", Some(&ok), None).is_ok());
        let bad = serde_json::json!({ "kind": "oauth2-auth-code" });
        assert!(validate("https://s", Some(&bad), None).is_err());
    }
}
