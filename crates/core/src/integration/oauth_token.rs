//! OAuth2 authorization-code token store (oauth2-egress.md §2).
//!
//! One row per (api-client, tenant); `access_token`/`refresh_token` are sealed
//! with the vault (never plaintext at rest). Resolution + refresh live in
//! [`super::token`]; this module only persists.

use serde::Deserialize;

use crate::db::DbDriver;
use crate::db::Driver;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// Decrypted (or to-be-sealed) token row — never echoes tokens over the API.
#[derive(Debug, Clone, Deserialize)]
pub struct OauthToken {
    pub client_key: String,
    pub tenant_id: String,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub expires_at: Option<Timestamp>,
    pub scope: Option<String>,
}

pub async fn find(
    pool: &crate::db::Pool,
    client_key: &str,
    tenant_id: &str,
) -> AppResult<Option<OauthToken>> {
    let sql = format!(
        "SELECT client_key, tenant_id, access_token, refresh_token, expires_at, scope \
         FROM itg_oauth_tokens WHERE client_key = {} AND tenant_id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    let row = sqlx::query_as::<
        crate::db::pool::Db,
        (
            String,
            String,
            Option<String>,
            Option<String>,
            Option<Timestamp>,
            Option<String>,
        ),
    >(crate::db::safe_sql(&sql))
    .bind(client_key)
    .bind(tenant_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(client_key, tenant_id, access_token, refresh_token, expires_at, scope)| OauthToken {
            client_key,
            tenant_id,
            access_token,
            refresh_token,
            expires_at,
            scope,
        },
    ))
}

pub async fn upsert(pool: &crate::db::Pool, row: &OauthToken) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let assignments = format!(
        "access_token = {}, refresh_token = {}, expires_at = {}, scope = {}, updated_at = {}",
        Driver::excluded_col("access_token"),
        Driver::excluded_col("refresh_token"),
        Driver::excluded_col("expires_at"),
        Driver::excluded_col("scope"),
        Driver::excluded_col("updated_at"),
    );
    let sql = format!(
        "INSERT INTO itg_oauth_tokens \
         (id, client_key, tenant_id, access_token, refresh_token, expires_at, scope, created_at, updated_at) \
         VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}) {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8),
        Driver::ph(9),
        Driver::upsert_clause("client_key, tenant_id", &assignments),
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(crate::utils::id::new_id())
        .bind(&row.client_key)
        .bind(&row.tenant_id)
        .bind(row.access_token.as_deref())
        .bind(row.refresh_token.as_deref())
        .bind(row.expires_at)
        .bind(row.scope.as_deref())
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &crate::db::Pool, client_key: &str, tenant_id: &str) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM itg_oauth_tokens WHERE client_key = {} AND tenant_id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(client_key)
        .bind(tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}
