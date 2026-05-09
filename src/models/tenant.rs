//! 租户模型与数据库查询
//!
//! 定义 `tenants` 表的数据结构及全部 CRUD 操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

/// tenants 表行模型
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct Tenant {
    pub id: i64,
    pub document_id: String,
    pub name: String,
    pub domain: Option<String>,
    pub config: String,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// 查询所有租户
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<Tenant>> {
    let tenants = sqlx::query_as::<_, Tenant>("SELECT * FROM tenants ORDER BY name")
        .fetch_all(pool)
        .await?;
    Ok(tenants)
}

/// 根据 document_id 查找租户
pub async fn find_by_id(pool: &crate::db::Pool, document_id: &str) -> AppResult<Option<Tenant>> {
    let sql = format!("SELECT * FROM tenants WHERE document_id = {}", ph(1));
    let tenant = sqlx::query_as::<_, Tenant>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;
    Ok(tenant)
}

/// 根据域名查找租户
pub async fn find_by_domain(pool: &crate::db::Pool, domain: &str) -> AppResult<Option<Tenant>> {
    let sql = format!("SELECT * FROM tenants WHERE domain = {}", ph(1));
    let tenant = sqlx::query_as::<_, Tenant>(&sql)
        .bind(domain)
        .fetch_optional(pool)
        .await?;
    Ok(tenant)
}

/// 创建租户
pub async fn create(
    pool: &crate::db::Pool,
    document_id: &str,
    name: &str,
    domain: Option<&str>,
    config: &str,
    created_at: Timestamp,
) -> AppResult<Tenant> {
    let sql = format!(
        "INSERT INTO tenants (document_id, name, domain, config, status, created_at, updated_at) VALUES ({}, {}, {}, {}, 'active', {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
    );
    sqlx::query(&sql)
        .bind(document_id)
        .bind(name)
        .bind(domain)
        .bind(config)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .map_err(|e| AppError::Conflict(format!("create tenant failed: {e}")))?;

    find_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found("tenant"))
}

/// 更新租户
pub async fn update(
    pool: &crate::db::Pool,
    document_id: &str,
    name: Option<&str>,
    domain: Option<&str>,
    config: Option<&str>,
    status: Option<&str>,
    updated_at: Timestamp,
) -> AppResult<Tenant> {
    let mut sets = Vec::new();
    let mut idx = 1usize;
    if name.is_some() {
        sets.push(format!("name = {}", ph(idx)));
        idx += 1;
    }
    if domain.is_some() {
        sets.push(format!("domain = {}", ph(idx)));
        idx += 1;
    }
    if config.is_some() {
        sets.push(format!("config = {}", ph(idx)));
        idx += 1;
    }
    if status.is_some() {
        sets.push(format!("status = {}", ph(idx)));
        idx += 1;
    }
    sets.push(format!("updated_at = {}", ph(idx)));
    idx += 1;

    let sql = format!(
        "UPDATE tenants SET {} WHERE document_id = {}",
        sets.join(", "),
        ph(idx),
    );
    let mut q = sqlx::query(&sql);
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
    q = q.bind(updated_at).bind(document_id);
    q.execute(pool).await?;

    find_by_id(pool, document_id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("tenant/{document_id}")))
}

/// 删除租户
pub async fn delete(pool: &crate::db::Pool, document_id: &str) -> AppResult<()> {
    let sql = format!("DELETE FROM tenants WHERE document_id = {}", ph(1));
    sqlx::query(&sql).bind(document_id).execute(pool).await?;
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

    #[tokio::test]
    async fn create_and_find_by_id() {
        let pool = setup_pool().await;
        let doc_id = "tenant-001";
        let now = crate::utils::tz::now_utc();
        let row = create(
            &pool,
            doc_id,
            "Test Tenant",
            Some("test.example.com"),
            "{}",
            now,
        )
        .await
        .unwrap();
        assert_eq!(row.document_id, doc_id);
        assert_eq!(row.name, "Test Tenant");
        assert_eq!(row.domain.unwrap(), "test.example.com");

        let found = find_by_id(&pool, doc_id).await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
        assert_eq!(found.document_id, doc_id);
    }

    #[tokio::test]
    async fn find_by_domain_returns_match() {
        let pool = setup_pool().await;
        let now = crate::utils::tz::now_utc();
        create(
            &pool,
            "tenant-002",
            "Dom Tenant",
            Some("dom.example.com"),
            "{}",
            now,
        )
        .await
        .unwrap();

        let found = find_by_domain(&pool, "dom.example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.document_id, "tenant-002");
        assert_eq!(found.name, "Dom Tenant");

        let missing = find_by_domain(&pool, "no.such.domain").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        let now = crate::utils::tz::now_utc();
        create(&pool, "tenant-a", "Alpha", None, "{}", now)
            .await
            .unwrap();
        create(&pool, "tenant-b", "Bravo", None, "{}", now)
            .await
            .unwrap();
        create(&pool, "tenant-c", "Charlie", None, "{}", now)
            .await
            .unwrap();

        let all = find_all(&pool).await.unwrap();
        assert!(all.len() >= 3);
    }

    #[tokio::test]
    async fn update_changes_name() {
        let pool = setup_pool().await;
        let doc_id = "tenant-003";
        let now = crate::utils::tz::now_utc();
        create(
            &pool,
            doc_id,
            "Original",
            Some("orig.example.com"),
            "{}",
            now,
        )
        .await
        .unwrap();

        let later = crate::utils::tz::now_utc();
        let updated = update(&pool, doc_id, Some("Updated Name"), None, None, None, later)
            .await
            .unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert_eq!(updated.domain.unwrap(), "orig.example.com");
    }

    #[tokio::test]
    async fn delete_removes_tenant() {
        let pool = setup_pool().await;
        let doc_id = "tenant-004";
        let now = crate::utils::tz::now_utc();
        create(&pool, doc_id, "ToDelete", None, "{}", now)
            .await
            .unwrap();

        delete(&pool, doc_id).await.unwrap();
        let found = find_by_id(&pool, doc_id).await.unwrap();
        assert!(found.is_none());
    }
}
