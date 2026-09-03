//! flow model + queries (dev-docs/workflow db-schema.md).
//! Flow row = workflow metadata; `current_version` → published snapshot.
use serde::Serialize;
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const FLOW_COLS: &str = "id, tenant_id, name, description, enabled, current_version, extra, \
     created_at, updated_at";

pub(crate) fn row_not_found(table: &str) -> AppError {
    AppError::not_found(table)
}
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Flow {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub current_version: Option<SnowflakeId>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub extra: Option<Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn insert_flow(pool: &crate::db::Pool, f: &Flow) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "INSERT INTO flow ({FLOW_COLS}) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8),
        Driver::ph(9)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*f.id)
        .bind(&f.tenant_id)
        .bind(&f.name)
        .bind(&f.description)
        .bind(f.enabled)
        .bind(f.current_version)
        .bind(&f.extra)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_flow_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<Flow> {
    let sql = format!("SELECT {FLOW_COLS} FROM flow WHERE id = {}", Driver::ph(1));
    sqlx::query_as::<crate::db::pool::Db, Flow>(crate::db::safe_sql(&sql))
        .bind(*id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| row_not_found("flow"))
}

pub async fn find_flows_by_tenant(pool: &crate::db::Pool, tenant_id: &str) -> AppResult<Vec<Flow>> {
    let sql = format!(
        "SELECT {FLOW_COLS} FROM flow WHERE tenant_id = {} ORDER BY id",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, Flow>(crate::db::safe_sql(&sql))
            .bind(tenant_id)
            .fetch_all(pool)
            .await?,
    )
}

/// Read a key from the `flow.extra` JSON object.
pub fn extra_get(flow: &Flow, key: &str) -> Option<Value> {
    let obj = flow.extra.as_ref()?.as_object()?;
    obj.get(key).cloned()
}

/// Set/clear a key inside the `flow.extra` JSON object (preserves other keys).
pub async fn extra_set(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    key: &str,
    value: Option<Value>,
) -> AppResult<()> {
    let flow = find_flow_by_id(pool, flow_id).await?;
    let mut obj = match flow.extra {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    match value {
        Some(v) => {
            obj.insert(key.to_string(), v);
        }
        None => {
            obj.remove(key);
        }
    }
    let extra = if obj.is_empty() {
        None
    } else {
        Some(Value::Object(obj))
    };
    let sql = format!(
        "UPDATE flow SET extra = {}, updated_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(&extra)
        .bind(crate::utils::tz::now_utc())
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The working-draft definition lives inside `flow.extra` under `_draft`
/// (schema-free, avoids a migration); publish promotes it to a version.
pub fn flow_draft(flow: &Flow) -> Option<Value> {
    extra_get(flow, "_draft")
}

/// The public API token (Dify-style external invocation).
pub fn flow_api_token(flow: &Flow) -> Option<String> {
    extra_get(flow, "_api_token").and_then(|v| v.as_str().map(str::to_string))
}

/// Store (`Some`) or clear (`None`) the working draft in `flow.extra`.
pub async fn set_flow_draft(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    draft: Option<Value>,
) -> AppResult<()> {
    extra_set(pool, flow_id, "_draft", draft).await
}

/// Whether another flow in the tenant already uses `name` (case-insensitive).
pub async fn flow_name_taken(
    pool: &crate::db::Pool,
    tenant_id: &str,
    name: &str,
    exclude_id: Option<SnowflakeId>,
) -> AppResult<bool> {
    let where_sql = match exclude_id {
        Some(_) => format!(
            "tenant_id = {} AND LOWER(name) = LOWER({}) AND id <> {}",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3)
        ),
        None => format!(
            "tenant_id = {} AND LOWER(name) = LOWER({})",
            Driver::ph(1),
            Driver::ph(2)
        ),
    };
    let sql = format!("SELECT 1 FROM flow WHERE {where_sql} LIMIT 1");
    let mut q = sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&sql));
    q = q.bind(tenant_id).bind(name);
    if let Some(id) = exclude_id {
        q = q.bind(*id);
    }
    Ok(q.fetch_optional(pool).await?.is_some())
}

/// Find a flow by its unique (case-insensitive) name within a tenant.
pub async fn find_flow_by_name(
    pool: &crate::db::Pool,
    tenant_id: &str,
    name: &str,
) -> AppResult<Option<Flow>> {
    let sql = format!(
        "SELECT {FLOW_COLS} FROM flow WHERE tenant_id = {} AND LOWER(name) = LOWER({})          LIMIT 1",
        Driver::ph(1),
        Driver::ph(2)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, Flow>(crate::db::safe_sql(&sql))
            .bind(tenant_id)
            .bind(name)
            .fetch_optional(pool)
            .await?,
    )
}

/// Paged flow rows (newest first) with the total count.
pub async fn find_flows_page(
    pool: &crate::db::Pool,
    tenant_id: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Flow>, i64)> {
    let total_sql = format!(
        "SELECT {} FROM flow WHERE tenant_id = {}",
        Driver::cast_int("COUNT(*)"),
        Driver::ph(1)
    );
    let total = sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&total_sql))
        .bind(tenant_id)
        .fetch_one(pool)
        .await?;
    let rows_sql = format!(
        "SELECT {FLOW_COLS} FROM flow WHERE tenant_id = {} \
         ORDER BY id DESC LIMIT {} OFFSET {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    let offset = (page - 1).max(0) * page_size;
    let rows = sqlx::query_as::<crate::db::pool::Db, Flow>(crate::db::safe_sql(&rows_sql))
        .bind(tenant_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

/// Update the mutable metadata columns of a flow row (name/description).
pub async fn update_flow_meta(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    name: &str,
    description: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow SET name = {}, description = {}, updated_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(name)
        .bind(description)
        .bind(crate::utils::tz::now_utc())
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Point `flow.current_version` at the newly published version.
pub async fn set_flow_current_version(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
    version_id: SnowflakeId,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow SET current_version = {}, updated_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*version_id)
        .bind(crate::utils::tz::now_utc())
        .bind(*flow_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Cascade-delete a flow and everything it owns: versions, instances plus
/// their durable snapshots and per-node run history. Cross-database safe
/// (subselect on SQLite/PG/MySQL).
pub async fn delete_flow(pool: &crate::db::Pool, flow_id: SnowflakeId) -> AppResult<()> {
    let p1 = Driver::ph(1);
    let cascade = [
        format!(
            "DELETE FROM flow_instance_snapshot WHERE instance_id IN \
             (SELECT id FROM flow_instance WHERE flow_id = {p1})"
        ),
        format!(
            "DELETE FROM flow_node_run WHERE instance_id IN (SELECT id FROM flow_instance WHERE flow_id = {p1})"
        ),
        format!("DELETE FROM flow_trigger WHERE flow_id = {p1}"),
        format!("DELETE FROM flow_api_key WHERE flow_id = {p1}"),
        format!("DELETE FROM flow_instance WHERE flow_id = {p1}"),
        format!("DELETE FROM flow_version WHERE flow_id = {p1}"),
        format!("DELETE FROM flow WHERE id = {p1}"),
    ];
    for sql in cascade {
        sqlx::query(crate::db::safe_sql(&sql))
            .bind(*flow_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}
