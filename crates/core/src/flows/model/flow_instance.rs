//! flow_instance model + queries (dev-docs/workflow db-schema.md).
//! One run (wf_trace root = id); waiting fields denormalized.
use serde::Serialize;
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const FLOW_INSTANCE_COLS: &str = "id, tenant_id, flow_id, flow_version_id, status, \
     has_exceptions, trigger_kind, trigger_payload, inputs_summary, outputs, error, started_by, \
     started_at, finished_at, waiting_kind, waiting_needed, waiting_received, resume_until, \
     created_at";

pub(crate) fn row_not_found(table: &str) -> AppError {
    AppError::not_found(table)
}
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct FlowInstance {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub flow_id: SnowflakeId,
    pub flow_version_id: SnowflakeId,
    pub status: String,
    pub has_exceptions: bool,
    pub trigger_kind: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub trigger_payload: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub inputs_summary: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub outputs: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub error: Option<Value>,
    pub started_by: Option<SnowflakeId>,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
    pub waiting_kind: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub waiting_needed: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub waiting_received: i64,
    pub resume_until: Option<Timestamp>,
    pub created_at: Timestamp,
}

pub async fn insert_flow_instance(pool: &crate::db::Pool, i: &FlowInstance) -> AppResult<()> {
    let ph = (1..=19).map(Driver::ph).collect::<Vec<_>>().join(", ");
    let sql = format!("INSERT INTO flow_instance ({FLOW_INSTANCE_COLS}) VALUES ({ph})");
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*i.id)
        .bind(&i.tenant_id)
        .bind(*i.flow_id)
        .bind(*i.flow_version_id)
        .bind(&i.status)
        .bind(i.has_exceptions)
        .bind(&i.trigger_kind)
        .bind(&i.trigger_payload)
        .bind(&i.inputs_summary)
        .bind(&i.outputs)
        .bind(&i.error)
        .bind(i.started_by)
        .bind(i.started_at)
        .bind(i.finished_at)
        .bind(&i.waiting_kind)
        .bind(i.waiting_needed)
        .bind(i.waiting_received)
        .bind(i.resume_until)
        .bind(i.created_at)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_instance_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<FlowInstance> {
    let sql = format!(
        "SELECT {FLOW_INSTANCE_COLS} FROM flow_instance WHERE id = {}",
        Driver::ph(1)
    );
    sqlx::query_as::<crate::db::pool::Db, FlowInstance>(crate::db::safe_sql(&sql))
        .bind(*id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| row_not_found("flow_instance"))
}

/// Update instance terminal/running status + optional finished_at.
#[allow(clippy::too_many_arguments)]
pub async fn update_instance_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: &str,
    has_exceptions: bool,
    error: Option<&Value>,
    finished_at: Option<Timestamp>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_instance SET status = {}, has_exceptions = {}, error = {}, \
         finished_at = {}, waiting_kind = NULL WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(status)
        .bind(has_exceptions)
        .bind(error)
        .bind(finished_at)
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Finalize a finished instance (success/failed): outputs + error + finished_at.
pub async fn finalize_instance(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: &str,
    has_exceptions: bool,
    outputs: Option<&Value>,
    error: Option<&Value>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_instance SET status = {}, has_exceptions = {}, outputs = {}, error = {}, \
         finished_at = {}, waiting_kind = NULL WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(status)
        .bind(has_exceptions)
        .bind(outputs)
        .bind(error)
        .bind(crate::utils::tz::now_utc())
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}
