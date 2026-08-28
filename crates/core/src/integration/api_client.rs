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

    /// Auth kind (`bearer` | `api-key-header` | `none`).
    #[must_use]
    pub fn auth_kind(&self) -> &str {
        self.auth
            .as_ref()
            .and_then(|a| a.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("none")
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
            "bearer" => {
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
            other => {
                return Err(AppError::BadRequest(format!(
                    "auth kind '{other}' not supported (bearer | api-key-header | none)"
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
        }
    }
    Ok(())
}
