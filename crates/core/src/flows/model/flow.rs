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
