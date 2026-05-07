//! RBAC 模型与数据库查询
//!
//! 定义 `roles` / `permissions` 表的数据结构及全部 CRUD 操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};

/// roles 表行模型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// permissions 表行模型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Permission {
    pub id: String,
    pub role_id: String,
    pub action: String,
    pub subject: String,
    pub fields: Option<String>,
    pub conditions: Option<String>,
    pub created_at: String,
}

/// 查询所有角色
pub async fn list_roles(pool: &crate::db::Pool) -> AppResult<Vec<Role>> {
    let roles = sqlx::query_as::<_, Role>(
        "SELECT id, name, description, is_system, created_at, updated_at FROM roles ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    Ok(roles)
}

/// 根据 ID 查找角色
pub async fn find_role_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<Role>> {
    let sql = format!(
        "SELECT id, name, description, is_system, created_at, updated_at FROM roles WHERE id = {}",
        ph(1)
    );
    let role = sqlx::query_as::<_, Role>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(role)
}

/// 根据角色名查找角色 ID
pub async fn find_role_id_by_name(pool: &crate::db::Pool, name: &str) -> AppResult<Option<String>> {
    let sql = format!(
        "SELECT id FROM roles WHERE name = {}",
        ph(1)
    );
    let row = sqlx::query_as::<_, (String,)>(&sql)
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(id,)| id))
}

/// 创建角色
pub async fn create_role(
    pool: &crate::db::Pool,
    id: &str,
    name: &str,
    description: Option<&str>,
    created_at: &str,
) -> AppResult<Role> {
    let sql = format!(
        "INSERT INTO roles (id, name, description, is_system, created_at, updated_at) VALUES ({}, {}, {}, 0, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    );
    sqlx::query(&sql)
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .map_err(|e| AppError::Conflict(format!("create role failed: {e}")))?;

    find_role_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found("role"))
}

/// 更新角色（动态 SET 子句）
pub async fn update_role(
    pool: &crate::db::Pool,
    id: &str,
    name: Option<&str>,
    description: Option<&str>,
    updated_at: &str,
) -> AppResult<Role> {
    let mut sets = Vec::new();
    let mut idx = 1;
    if name.is_some() {
        sets.push(format!("name = {}", ph(idx)));
        idx += 1;
    }
    if description.is_some() {
        sets.push(format!("description = {}", ph(idx)));
        idx += 1;
    }
    sets.push(format!("updated_at = {}", ph(idx)));
    idx += 1;

    let sql = format!(
        "UPDATE roles SET {} WHERE id = {}",
        sets.join(", "),
        ph(idx)
    );
    let mut q = sqlx::query(sql.as_ref());
    if let Some(n) = name {
        q = q.bind(n);
    }
    if let Some(d) = description {
        q = q.bind(d);
    }
    q = q.bind(updated_at).bind(id);
    q.execute(pool).await?;

    find_role_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("role/{id}")))
}

/// 删除角色
pub async fn delete_role(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let sql = format!("DELETE FROM roles WHERE id = {}", ph(1));
    sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(())
}

/// 查询角色的所有权限
pub async fn find_permissions_by_role_id(
    pool: &crate::db::Pool,
    role_id: &str,
) -> AppResult<Vec<Permission>> {
    let sql = format!(
        "SELECT id, role_id, action, subject, fields, conditions, created_at FROM permissions WHERE role_id = {} ORDER BY action",
        ph(1)
    );
    let perms = sqlx::query_as::<_, Permission>(&sql)
        .bind(role_id)
        .fetch_all(pool)
        .await?;
    Ok(perms)
}

/// 删除角色的所有权限
pub async fn delete_permissions_by_role_id(pool: &crate::db::Pool, role_id: &str) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM permissions WHERE role_id = {}",
        ph(1)
    );
    sqlx::query(&sql).bind(role_id).execute(pool).await?;
    Ok(())
}

/// 插入单条权限
#[allow(clippy::too_many_arguments)]
pub async fn insert_permission(
    pool: &crate::db::Pool,
    id: &str,
    role_id: &str,
    action: &str,
    subject: &str,
    fields: Option<&str>,
    conditions: Option<&str>,
    created_at: &str,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO permissions (id, role_id, action, subject, fields, conditions, created_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7)
    );
    sqlx::query(&sql)
        .bind(id)
        .bind(role_id)
        .bind(action)
        .bind(subject)
        .bind(fields)
        .bind(conditions)
        .bind(created_at)
        .execute(pool)
        .await?;
    Ok(())
}
