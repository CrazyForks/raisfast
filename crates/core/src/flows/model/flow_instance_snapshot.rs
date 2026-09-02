//! flow_instance_snapshot model + queries (dev-docs/workflow db-schema.md).
//! Durable whole-snapshot (1:1 per instance, single writer).
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;

pub async fn upsert_snapshot(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    snapshot: &Value,
) -> AppResult<()> {
    let assignments = format!(
        "snapshot = {}, snapshot_version = 1, updated_at = {}",
        Driver::excluded_col("snapshot"),
        Driver::excluded_col("updated_at"),
    );
    let sql = format!(
        "INSERT INTO flow_instance_snapshot (instance_id, snapshot, snapshot_version, updated_at) \
         VALUES ({}, {}, 1, {}) {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::upsert_clause("instance_id", &assignments),
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*instance_id)
        .bind(snapshot)
        .bind(crate::utils::tz::now_utc())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_snapshot(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
) -> AppResult<Option<Value>> {
    let sql = format!(
        "SELECT snapshot FROM flow_instance_snapshot WHERE instance_id = {}",
        Driver::ph(1)
    );
    let row: Option<Value> = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .bind(*instance_id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// Delete the snapshot row (instance terminal / cleanup).
pub async fn delete_snapshot(pool: &crate::db::Pool, instance_id: SnowflakeId) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM flow_instance_snapshot WHERE instance_id = {}",
        Driver::ph(1)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*instance_id)
        .execute(pool)
        .await?;
    Ok(())
}
