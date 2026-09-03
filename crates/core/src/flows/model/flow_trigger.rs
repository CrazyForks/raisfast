//! flow_trigger model — internal automation triggers (event/cron) that point
//! at a flow. Decoupled: a trigger knows which flow it starts; flows never
//! declare triggers. (Public API exposure is a separate concern — flow_api_key.)

use serde::Serialize;
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

const COLS: &str = "id, tenant_id, flow_id, kind, name, event_type, filter, cron_expr, \
     inputs_map, enabled, last_triggered_at, created_at";

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct FlowTrigger {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub flow_id: SnowflakeId,
    pub kind: String,
    pub name: String,
    pub event_type: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub filter: Option<Value>,
    pub cron_expr: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub inputs_map: Option<Value>,
    pub enabled: bool,
    pub last_triggered_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

pub async fn create(pool: &crate::db::Pool, trigger: &FlowTrigger) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO flow_trigger ({COLS}) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8),
        Driver::ph(9),
        Driver::ph(10),
        Driver::ph(11),
        Driver::ph(12)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*trigger.id)
        .bind(&trigger.tenant_id)
        .bind(*trigger.flow_id)
        .bind(&trigger.kind)
        .bind(&trigger.name)
        .bind(&trigger.event_type)
        .bind(&trigger.filter)
        .bind(&trigger.cron_expr)
        .bind(&trigger.inputs_map)
        .bind(trigger.enabled)
        .bind(trigger.last_triggered_at)
        .bind(trigger.created_at)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<FlowTrigger> {
    let sql = format!(
        "SELECT {COLS} FROM flow_trigger WHERE id = {}",
        Driver::ph(1)
    );
    sqlx::query_as::<crate::db::pool::Db, FlowTrigger>(crate::db::safe_sql(&sql))
        .bind(*id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::not_found("flow_trigger"))
}

pub async fn list(
    pool: &crate::db::Pool,
    tenant_id: &str,
    kind: Option<&str>,
) -> AppResult<Vec<FlowTrigger>> {
    let where_sql = match kind {
        Some(_) => format!("tenant_id = {} AND kind = {}", Driver::ph(1), Driver::ph(2)),
        None => format!("tenant_id = {}", Driver::ph(1)),
    };
    let sql = format!("SELECT {COLS} FROM flow_trigger WHERE {where_sql} ORDER BY id DESC");
    let mut q = sqlx::query_as::<crate::db::pool::Db, FlowTrigger>(crate::db::safe_sql(&sql));
    q = q.bind(tenant_id);
    if let Some(k) = kind {
        q = q.bind(k);
    }
    Ok(q.fetch_all(pool).await?)
}

/// Enabled triggers of a given kind+event type (for the eventbus subscriber).
pub async fn list_enabled_by_event(
    pool: &crate::db::Pool,
    tenant_id: &str,
    event_type: &str,
) -> AppResult<Vec<FlowTrigger>> {
    let sql = format!(
        "SELECT {COLS} FROM flow_trigger WHERE tenant_id = {} AND kind = 'event' AND enabled = TRUE \
         AND event_type = {} ORDER BY id ASC",
        Driver::ph(1),
        Driver::ph(2)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowTrigger>(crate::db::safe_sql(&sql))
            .bind(tenant_id)
            .bind(event_type)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn set_enabled(pool: &crate::db::Pool, id: SnowflakeId, enabled: bool) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_trigger SET enabled = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(enabled)
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_enabled_cron(
    pool: &crate::db::Pool,
    tenant_id: &str,
) -> AppResult<Vec<FlowTrigger>> {
    let sql = format!(
        "SELECT {COLS} FROM flow_trigger WHERE tenant_id = {} AND kind = 'cron' AND enabled = TRUE \
         ORDER BY id ASC",
        Driver::ph(1)
    );
    Ok(
        sqlx::query_as::<crate::db::pool::Db, FlowTrigger>(crate::db::safe_sql(&sql))
            .bind(tenant_id)
            .fetch_all(pool)
            .await?,
    )
}

pub async fn set_last_triggered(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    at: crate::utils::tz::Timestamp,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE flow_trigger SET last_triggered_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(at)
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    let sql = format!("DELETE FROM flow_trigger WHERE id = {}", Driver::ph(1));
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}
