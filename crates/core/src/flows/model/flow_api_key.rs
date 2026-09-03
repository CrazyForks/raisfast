//! flow_api_key model — dedicated public-API keys for flows.
//!
//! Token is stored AES-GCM encrypted (`token_enc`); lookup goes through a
//! unique sha256 (`token_hash`) so bearer strings never appear in queries/logs.
//! The public path uses a separate random `slug` instead of the flow's id.

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;

/// One API key row.
pub struct FlowApiKey {
    pub id: i64,
    pub flow_id: SnowflakeId,
    pub token_hash: String,
    pub token_enc: String,
    pub slug: String,
    pub enabled: bool,
    pub require_auth: bool,
}

type Row = (i64, i64, String, String, String, bool, bool);

const COLS: &str = "id, flow_id, token_hash, token_enc, slug, enabled, require_auth";

fn map_row(r: Row) -> FlowApiKey {
    let (id, flow_id, token_hash, token_enc, slug, enabled, require_auth) = r;
    FlowApiKey {
        id,
        flow_id: SnowflakeId(flow_id),
        token_hash,
        token_enc,
        slug,
        enabled,
        require_auth,
    }
}

pub async fn find_by_flow(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
) -> AppResult<Option<FlowApiKey>> {
    let sql = format!(
        "SELECT {COLS} FROM flow_api_key WHERE flow_id = {} ORDER BY id DESC LIMIT 1",
        Driver::ph(1)
    );
    let row = sqlx::query_as::<crate::db::pool::Db, Row>(crate::db::safe_sql(&sql))
        .bind(*flow_id)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(map_row))
}

pub async fn find_by_hash(pool: &crate::db::Pool, hash: &str) -> AppResult<Option<FlowApiKey>> {
    let sql = format!(
        "SELECT {COLS} FROM flow_api_key WHERE token_hash = {} LIMIT 1",
        Driver::ph(1)
    );
    let row = sqlx::query_as::<crate::db::pool::Db, Row>(crate::db::safe_sql(&sql))
        .bind(hash)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(map_row))
}

pub async fn find_by_slug(pool: &crate::db::Pool, slug: &str) -> AppResult<Option<FlowApiKey>> {
    let sql = format!(
        "SELECT {COLS} FROM flow_api_key WHERE slug = {} LIMIT 1",
        Driver::ph(1)
    );
    let row = sqlx::query_as::<crate::db::pool::Db, Row>(crate::db::safe_sql(&sql))
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(map_row))
}

pub async fn create(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    token_hash: &str,
    token_enc: &str,
    slug: &str,
    require_auth: bool,
) -> AppResult<i64> {
    let sql = format!(
        "INSERT INTO flow_api_key (id, flow_id, token_hash, token_enc, slug, enabled, require_auth, created_at) \
         VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8)
    );
    let id = *crate::utils::id::new_snowflake_id();
    let created = crate::utils::tz::now_utc();
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(id)
        .bind(*flow_id)
        .bind(token_hash)
        .bind(token_enc)
        .bind(slug)
        .bind(true)
        .bind(require_auth)
        .bind(created)
        .execute(pool)
        .await?;
    Ok(id)
}

pub async fn update_token(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    token_hash: &str,
    token_enc: &str,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_api_key SET token_hash = {}, token_enc = {} WHERE flow_id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(token_hash)
        .bind(token_enc)
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_slug(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    slug: &str,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_api_key SET slug = {} WHERE flow_id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(slug)
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_enabled(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    enabled: bool,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_api_key SET enabled = {} WHERE flow_id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(enabled)
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_require_auth(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    require_auth: bool,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_api_key SET require_auth = {} WHERE flow_id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(require_auth)
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn touch(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_api_key SET last_used_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(crate::utils::tz::now_utc())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_by_flow(pool: &crate::db::Pool, flow_id: SnowflakeId) -> AppResult<()> {
    let sql = format!("DELETE FROM flow_api_key WHERE flow_id = {}", Driver::ph(1));
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}
