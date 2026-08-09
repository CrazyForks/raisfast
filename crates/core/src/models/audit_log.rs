//! Audit log data model and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Full database row for an audit log entry
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub actor_id: Option<SnowflakeId>,
    pub actor_role: Option<String>,
    pub action: String,
    pub subject: String,
    pub subject_id: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub detail: Option<serde_json::Value>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: Timestamp,
}

/// Insert an audit log entry
pub async fn insert(pool: &crate::db::Pool, entry: &AuditEntry) -> AppResult<()> {
    raisfast_derive::crud_insert!(
        pool,
        "audit_log",
        [
            "id" => entry.id,
            "actor_id" => entry.actor_id,
            "actor_role" => entry.actor_role.as_deref(),
            "action" => &entry.action,
            "subject" => &entry.subject,
            "subject_id" => entry.subject_id.as_deref(),
            "detail" => entry.detail.as_ref(),
            "ip_address" => entry.ip_address.as_deref(),
            "user_agent" => entry.user_agent.as_deref(),
            "created_at" => entry.created_at
        ],
        tenant: entry.tenant_id.as_deref()
    )?;
    Ok(())
}

/// Paginated query for audit logs
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    action: Option<&str>,
    actor_id: Option<i64>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<AuditEntry>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, AuditEntry,
        table: "audit_log",
        // The list form skips conditions whose Option value is None
        // (the `AND(...)` DSL form would emit `col = NULL` and match nothing).
        where: ["action" => action, "actor_id" => actor_id],
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

/// Find an audit log entry by ID
pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<AuditEntry> {
    raisfast_derive::crud_find_one!(pool, "audit_log", AuditEntry, where: ("id", id))
        .map_err(Into::into)
}
