//! 审计日志数据模型与数据库查询

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// 审计日志完整数据库行
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

/// 插入一条审计日志
pub async fn insert(pool: &crate::db::Pool, entry: &AuditEntry) -> AppResult<()> {
    match &entry.tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO audit_log (document_id, tenant_id, actor_id, actor_role, action, subject, subject_id, detail, ip_address, user_agent, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::db::dialect::ph(1),
                crate::db::dialect::ph(2),
                crate::db::dialect::ph(3),
                crate::db::dialect::ph(4),
                crate::db::dialect::ph(5),
                crate::db::dialect::ph(6),
                crate::db::dialect::ph(7),
                crate::db::dialect::ph(8),
                crate::db::dialect::ph(9),
                crate::db::dialect::ph(10),
                crate::db::dialect::ph(11)
            );
            sqlx::query(&sql)
                .bind(&entry.document_id)
                .bind(tid)
                .bind(entry.actor_id)
                .bind(&entry.actor_role)
                .bind(&entry.action)
                .bind(&entry.subject)
                .bind(&entry.subject_id)
                .bind(&entry.detail)
                .bind(&entry.ip_address)
                .bind(&entry.user_agent)
                .bind(entry.created_at)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO audit_log (document_id, actor_id, actor_role, action, subject, subject_id, detail, ip_address, user_agent, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::db::dialect::ph(1),
                crate::db::dialect::ph(2),
                crate::db::dialect::ph(3),
                crate::db::dialect::ph(4),
                crate::db::dialect::ph(5),
                crate::db::dialect::ph(6),
                crate::db::dialect::ph(7),
                crate::db::dialect::ph(8),
                crate::db::dialect::ph(9),
                crate::db::dialect::ph(10)
            );
            sqlx::query(&sql)
                .bind(&entry.document_id)
                .bind(entry.actor_id)
                .bind(&entry.actor_role)
                .bind(&entry.action)
                .bind(&entry.subject)
                .bind(&entry.subject_id)
                .bind(&entry.detail)
                .bind(&entry.ip_address)
                .bind(&entry.user_agent)
                .bind(entry.created_at)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// 分页查询审计日志
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    action: Option<&str>,
    actor_id: Option<i64>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<AuditEntry>, i64)> {
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

/// 根据 ID 查找审计日志
pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<AuditEntry> {
    let sql = format!(
        "SELECT * FROM audit_log WHERE id = {}",
        crate::db::dialect::ph(1)
    );
    sqlx::query_as::<_, AuditEntry>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
) -> AppResult<AuditEntry> {
    let sql = format!(
        "SELECT * FROM audit_log WHERE document_id = {}",
        crate::db::dialect::ph(1)
    );
    sqlx::query_as::<_, AuditEntry>(&sql)
        .bind(document_id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}
