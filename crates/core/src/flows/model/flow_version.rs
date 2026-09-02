//! flow_version model + queries (dev-docs/workflow db-schema.md).
//! Immutable definition snapshots; publish = append.
use serde::Serialize;
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const FLOW_VERSION_COLS: &str = "id, flow_id, version_number, definition, created_by, created_at";

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct FlowVersion {
    pub id: SnowflakeId,
    pub flow_id: SnowflakeId,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version_number: i64,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Value,
    pub created_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
}

pub async fn insert_flow_version(pool: &crate::db::Pool, v: &FlowVersion) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO flow_version ({FLOW_VERSION_COLS}) VALUES ({}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*v.id)
        .bind(*v.flow_id)
        .bind(v.version_number)
        .bind(&v.definition)
        .bind(v.created_by)
        .bind(v.created_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// Latest published version of a flow (None = not published yet).
pub async fn latest_version(
    pool: &crate::db::Pool,
    flow_id: SnowflakeId,
) -> AppResult<Option<FlowVersion>> {
    let sql = format!(
        "SELECT {FLOW_VERSION_COLS} FROM flow_version WHERE flow_id = {} \
         ORDER BY version_number DESC LIMIT 1",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowVersion>(crate::db::safe_sql(&sql))
            .bind(*flow_id)
            .fetch_optional(pool)
            .await?,
    )
}

pub async fn find_version_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<FlowVersion>> {
    let sql = format!(
        "SELECT {FLOW_VERSION_COLS} FROM flow_version WHERE id = {}",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowVersion>(crate::db::safe_sql(&sql))
            .bind(*id)
            .fetch_optional(pool)
            .await?,
    )
}
