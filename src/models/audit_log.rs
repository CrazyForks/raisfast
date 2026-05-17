//! Audit log data model and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// Full database row for an audit log entry
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub actor_id: Option<i64>,
    pub actor_role: Option<String>,
    pub action: String,
    pub subject: String,
    pub subject_id: Option<String>,
    pub detail: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(AuditEntry {
    required { id, document_id, action, subject, created_at }
    optional { actor_id, actor_role, subject_id, detail, ip_address, user_agent }
});

/// Insert an audit log entry
pub async fn insert(pool: &crate::db::Pool, entry: &AuditEntry) -> AppResult<()> {
    raisfast_derive::tenant_insert!(
        pool,
        "audit_log",
        [
            "document_id" => &entry.document_id,
            "actor_id" => entry.actor_id,
            "actor_role" => &entry.actor_role,
            "action" => &entry.action,
            "subject" => &entry.subject,
            "subject_id" => &entry.subject_id,
            "detail" => &entry.detail,
            "ip_address" => &entry.ip_address,
            "user_agent" => &entry.user_agent,
            "created_at" => entry.created_at
        ],
        entry.tenant_id.as_deref()
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
    raisfast_derive::check_schema!("audit_log", "action", "actor_id", "created_at");
    let offset = (page - 1).max(0) * page_size;

    let mut ph_idx = 1usize;
    let mut where_clauses = vec!["1=1".to_string()];

    if tenant_id.is_some() {
        where_clauses.push(format!("tenant_id = {}", crate::db::dialect::ph(ph_idx)));
        ph_idx += 1;
    }
    if action.is_some() {
        where_clauses.push(format!("action = {}", crate::db::dialect::ph(ph_idx)));
        ph_idx += 1;
    }
    if actor_id.is_some() {
        where_clauses.push(format!("actor_id = {}", crate::db::dialect::ph(ph_idx)));
        ph_idx += 1;
    }

    let where_str = where_clauses.join(" AND ");
    let count_sql = format!("SELECT COUNT(*) FROM audit_log WHERE {where_str}");
    let data_sql = format!(
        "SELECT * FROM audit_log WHERE {where_str} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        crate::db::dialect::ph(ph_idx),
        crate::db::dialect::ph(ph_idx + 1)
    );

    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    let mut dq = sqlx::query_as::<_, AuditEntry>(&data_sql);

    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
        dq = dq.bind(tid);
    }
    if let Some(a) = action {
        cq = cq.bind(a);
        dq = dq.bind(a);
    }
    if let Some(aid) = actor_id {
        cq = cq.bind(aid);
        dq = dq.bind(aid);
    }

    let total = cq.fetch_one(pool).await?;
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;

    Ok((items, total))
}

/// Find an audit log entry by ID
pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<AuditEntry> {
    raisfast_derive::crud_find_one!(pool, "audit_log", AuditEntry, "id" => id).map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
) -> AppResult<AuditEntry> {
    raisfast_derive::crud_find_one!(pool, "audit_log", AuditEntry, "document_id" => document_id)
        .map_err(Into::into)
}
