//! 审计日志数据模型与数据库查询

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// 审计日志完整数据库行
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct AuditEntry {
    pub id: String,
    pub tenant_id: String,
    pub actor_id: Option<String>,
    pub actor_role: Option<String>,
    pub action: String,
    pub subject: String,
    pub subject_id: Option<String>,
    pub detail: Option<String>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: String,
}

/// 插入一条审计日志
pub async fn insert(pool: &crate::db::Pool, entry: &AuditEntry) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "INSERT INTO audit_log (id, tenant_id, actor_id, actor_role, action, subject, subject_id, detail, ip_address, user_agent, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&entry.id)
        .bind(&entry.tenant_id)
        .bind(&entry.actor_id)
        .bind(&entry.actor_role)
        .bind(&entry.action)
        .bind(&entry.subject)
        .bind(&entry.subject_id)
        .bind(&entry.detail)
        .bind(&entry.ip_address)
        .bind(&entry.user_agent)
        .bind(&entry.created_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// 分页查询审计日志
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    action: Option<&str>,
    actor_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<AuditEntry>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let mut where_clauses = vec!["1=1".to_string()];
    let mut count_sql = "SELECT COUNT(*) FROM audit_log WHERE 1=1".to_string();
    let mut data_sql = "SELECT * FROM audit_log WHERE 1=1".to_string();

    if tenant_id.is_some() {
        where_clauses.push("tenant_id = ?".to_string());
    }
    if action.is_some() {
        where_clauses.push("action = ?".to_string());
    }
    if actor_id.is_some() {
        where_clauses.push("actor_id = ?".to_string());
    }

    let where_str = where_clauses.join(" AND ");
    count_sql = format!("{count_sql} AND {where_str}");
    data_sql = format!("{data_sql} AND {where_str} ORDER BY created_at DESC LIMIT ? OFFSET ?");

    let count_sql = crate::db::dialect::translate(&count_sql);
    let data_sql = crate::db::dialect::translate(&data_sql);

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
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<AuditEntry> {
    let sql = crate::db::dialect::translate("SELECT * FROM audit_log WHERE id = ?");
    sqlx::query_as::<_, AuditEntry>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}
