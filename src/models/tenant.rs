//! 租户模型与数据库查询
//!
//! 定义 `tenants` 表的数据结构及全部 CRUD 操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::{AppError, AppResult};

/// tenants 表行模型
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub config: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 查询所有租户
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Tenant>> {
    let tenants = sqlx::query_as::<_, Tenant>("SELECT * FROM tenants ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(tenants)
}

/// 根据 ID 查找租户
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<Tenant>> {
    let sql = crate::db::dialect::translate("SELECT * FROM tenants WHERE id = ?");
    let tenant = sqlx::query_as::<_, Tenant>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(tenant)
}

/// 根据域名查找租户
pub async fn find_by_domain(pool: &crate::db::Pool, domain: &str) -> AppResult<Option<Tenant>> {
    let sql = crate::db::dialect::translate("SELECT * FROM tenants WHERE domain = ?");
    let tenant = sqlx::query_as::<_, Tenant>(&sql)
        .bind(domain)
        .fetch_optional(pool)
        .await?;
    Ok(tenant)
}

/// 创建租户
pub async fn create(
    pool: &crate::db::Pool,
    id: &str,
    name: &str,
    domain: Option<&str>,
    config: &str,
    created_at: &str,
) -> AppResult<Tenant> {
    let sql = crate::db::dialect::translate(
        "INSERT INTO tenants (id, name, domain, config, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'active', ?, ?)",
    );
    sqlx::query(&sql)
        .bind(id)
        .bind(name)
        .bind(domain)
        .bind(config)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .map_err(|e| AppError::Conflict(format!("create tenant failed: {e}")))?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("tenant"))
}

/// 更新租户
pub async fn update(
    pool: &crate::db::Pool,
    id: &str,
    name: Option<&str>,
    domain: Option<&str>,
    config: Option<&str>,
    status: Option<&str>,
    updated_at: &str,
) -> AppResult<Tenant> {
    let mut sets = Vec::new();
    if name.is_some() {
        sets.push("name = ?");
    }
    if domain.is_some() {
        sets.push("domain = ?");
    }
    if config.is_some() {
        sets.push("config = ?");
    }
    if status.is_some() {
        sets.push("status = ?");
    }
    sets.push("updated_at = ?");

    let sql = format!("UPDATE tenants SET {} WHERE id = ?", sets.join(", "));
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(sql.as_ref());
    if let Some(n) = name {
        q = q.bind(n);
    }
    if let Some(d) = domain {
        q = q.bind(d);
    }
    if let Some(c) = config {
        q = q.bind(c);
    }
    if let Some(s) = status {
        q = q.bind(s);
    }
    q = q.bind(updated_at).bind(id);
    q.execute(pool).await?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("tenant/{id}")))
}

/// 删除租户
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let sql = crate::db::dialect::translate("DELETE FROM tenants WHERE id = ?");
    sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(())
}
