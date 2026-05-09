//! RBAC 模型与数据库查询
//!
//! 定义 `roles` / `permissions` 表的数据结构及全部 CRUD 操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

/// roles 表行模型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Role {
    pub id: i64,
    pub document_id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_system: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// permissions 表行模型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Permission {
    pub id: i64,
    pub document_id: String,
    pub role_id: i64,
    pub action: String,
    pub subject: String,
    pub fields: Option<String>,
    pub conditions: Option<String>,
    pub created_at: Timestamp,
}

/// 查询所有角色
pub async fn list_roles(pool: &crate::db::Pool) -> AppResult<Vec<Role>> {
    let roles = sqlx::query_as::<_, Role>(
        "SELECT id, document_id, name, description, is_system, created_at, updated_at FROM roles ORDER BY name",
    )
    .fetch_all(pool)
    .await?;
    Ok(roles)
}

/// 根据 document_id 查找角色
pub async fn find_role_by_id(pool: &crate::db::Pool, document_id: &str) -> AppResult<Option<Role>> {
    let sql = format!(
        "SELECT id, document_id, name, description, is_system, created_at, updated_at FROM roles WHERE document_id = {}",
        ph(1)
    );
    let role = sqlx::query_as::<_, Role>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;
    Ok(role)
}

/// 根据角色名查找角色 ID（返回整数 PK）
pub async fn find_role_id_by_name(pool: &crate::db::Pool, name: &str) -> AppResult<Option<i64>> {
    let sql = format!("SELECT id FROM roles WHERE name = {}", ph(1));
    let row = sqlx::query_as::<_, (i64,)>(&sql)
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(id,)| id))
}

/// 创建角色
pub async fn create_role(
    pool: &crate::db::Pool,
    document_id: &str,
    name: &str,
    description: Option<&str>,
    created_at: Timestamp,
) -> AppResult<Role> {
    let sql = format!(
        "INSERT INTO roles (document_id, name, description, is_system, created_at, updated_at) VALUES ({}, {}, {}, 0, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5)
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(name)
        .bind(description)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .map_err(|e| AppError::Conflict(format!("create role failed: {e}")))?;

    find_role_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found("role"))
}

/// 更新角色（动态 SET 子句）
pub async fn update_role(
    pool: &crate::db::Pool,
    document_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    updated_at: Timestamp,
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
        "UPDATE roles SET {} WHERE document_id = {}",
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
    q = q.bind(updated_at).bind(document_id);
    q.execute(pool).await?;

    find_role_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("role/{document_id}")))
}

/// 删除角色
pub async fn delete_role(pool: &crate::db::Pool, document_id: &str) -> AppResult<()> {
    let sql = format!("DELETE FROM roles WHERE document_id = {}", ph(1));
    sqlx::query(&sql).bind(document_id).execute(pool).await?;
    Ok(())
}

/// 查询角色的所有权限
pub async fn find_permissions_by_role_id(
    pool: &crate::db::Pool,
    role_id: i64,
) -> AppResult<Vec<Permission>> {
    let sql = format!(
        "SELECT id, document_id, role_id, action, subject, fields, conditions, created_at FROM permissions WHERE role_id = {} ORDER BY action",
        ph(1)
    );
    let perms = sqlx::query_as::<_, Permission>(&sql)
        .bind(role_id)
        .fetch_all(pool)
        .await?;
    Ok(perms)
}

/// 删除角色的所有权限
pub async fn delete_permissions_by_role_id(pool: &crate::db::Pool, role_id: i64) -> AppResult<()> {
    let sql = format!("DELETE FROM permissions WHERE role_id = {}", ph(1));
    sqlx::query(&sql).bind(role_id).execute(pool).await?;
    Ok(())
}

/// 插入单条权限
#[allow(clippy::too_many_arguments)]
pub async fn insert_permission(
    pool: &crate::db::Pool,
    document_id: &str,
    role_id: i64,
    action: &str,
    subject: &str,
    fields: Option<&str>,
    conditions: Option<&str>,
    created_at: Timestamp,
) -> AppResult<()> {
    let sql = format!(
        "INSERT INTO permissions (document_id, role_id, action, subject, fields, conditions, created_at) VALUES ({}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7)
    );
    sqlx::query(&sql)
        .bind(document_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn now() -> Timestamp {
        crate::utils::tz::now_utc()
    }

    #[sqlx::test]
    async fn create_and_find_role_by_id() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "admin_test", Some("desc"), now())
            .await
            .unwrap();
        assert_eq!(role.document_id, doc_id);
        assert_eq!(role.name, "admin_test");

        let found = find_role_by_id(&pool, &doc_id).await.unwrap().unwrap();
        assert_eq!(found.id, role.id);
    }

    #[sqlx::test]
    async fn list_roles_test() {
        let pool = setup_pool().await;
        for i in 0..3 {
            let doc_id = crate::utils::id::new_document_id();
            create_role(&pool, &doc_id, &format!("role_{i}"), None, now())
                .await
                .unwrap();
        }
        let roles = super::list_roles(&pool).await.unwrap();
        assert!(roles.len() >= 3);
    }

    #[sqlx::test]
    async fn update_role_changes_name() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        create_role(&pool, &doc_id, "original", None, now())
            .await
            .unwrap();

        let updated = update_role(&pool, &doc_id, Some("new_name"), None, now())
            .await
            .unwrap();
        assert_eq!(updated.name, "new_name");
    }

    #[sqlx::test]
    async fn delete_role_test() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        create_role(&pool, &doc_id, "to_delete", None, now())
            .await
            .unwrap();

        super::delete_role(&pool, &doc_id).await.unwrap();
        let found = find_role_by_id(&pool, &doc_id).await.unwrap();
        assert!(found.is_none());
    }

    #[sqlx::test]
    async fn permissions_crud() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "perm_role", None, now())
            .await
            .unwrap();

        let t = now();
        insert_permission(
            &pool,
            &crate::utils::id::new_document_id(),
            role.id,
            "read",
            "posts",
            None,
            None,
            t,
        )
        .await
        .unwrap();
        insert_permission(
            &pool,
            &crate::utils::id::new_document_id(),
            role.id,
            "write",
            "posts",
            None,
            None,
            t,
        )
        .await
        .unwrap();

        let perms = find_permissions_by_role_id(&pool, role.id).await.unwrap();
        assert_eq!(perms.len(), 2);

        delete_permissions_by_role_id(&pool, role.id).await.unwrap();
        let perms = find_permissions_by_role_id(&pool, role.id).await.unwrap();
        assert!(perms.is_empty());
    }

    #[sqlx::test]
    async fn find_role_id_by_name_test() {
        let pool = setup_pool().await;
        let doc_id = crate::utils::id::new_document_id();
        let role = create_role(&pool, &doc_id, "lookup_name", None, now())
            .await
            .unwrap();

        let id = super::find_role_id_by_name(&pool, "lookup_name")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(id, role.id);
    }
}
