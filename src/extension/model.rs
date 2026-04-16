//! Extension 数据模型与数据库查询

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// extensions 表完整行
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct ExtensionRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub enabled: i64,
    pub config: Option<String>,
    pub installed_at: String,
    pub updated_at: String,
    pub tenant_id: Option<String>,
}

/// 插入一条 Extension 记录
pub async fn insert(pool: &crate::db::Pool, record: &ExtensionRecord) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "INSERT INTO extensions (id, name, version, enabled, config, installed_at, updated_at, tenant_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&record.id)
        .bind(&record.name)
        .bind(&record.version)
        .bind(record.enabled)
        .bind(&record.config)
        .bind(&record.installed_at)
        .bind(&record.updated_at)
        .bind(&record.tenant_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 按 ID 查询
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<ExtensionRecord>> {
    let sql = crate::db::dialect::translate("SELECT * FROM extensions WHERE id = ?");
    let row = sqlx::query_as::<_, ExtensionRecord>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 查询所有已安装 Extension
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<ExtensionRecord>> {
    let sql = crate::db::dialect::translate("SELECT * FROM extensions ORDER BY installed_at ASC");
    let rows = sqlx::query_as::<_, ExtensionRecord>(&sql)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 更新启用状态
pub async fn set_enabled(
    pool: &crate::db::Pool,
    id: &str,
    enabled: bool,
    now: &str,
) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "UPDATE extensions SET enabled = ?, updated_at = ? WHERE id = ?",
    );
    sqlx::query(&sql)
        .bind(if enabled { 1i64 } else { 0i64 })
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 更新版本号
pub async fn update_version(
    pool: &crate::db::Pool,
    id: &str,
    version: &str,
    name: &str,
    now: &str,
) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "UPDATE extensions SET version = ?, name = ?, updated_at = ? WHERE id = ?",
    );
    sqlx::query(&sql)
        .bind(version)
        .bind(name)
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除记录
pub async fn delete(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let sql = crate::db::dialect::translate("DELETE FROM extensions WHERE id = ?");
    sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/014_extensions.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn sample_record(id: &str) -> ExtensionRecord {
        ExtensionRecord {
            id: id.to_string(),
            name: format!("Test {id}"),
            version: "1.0.0".to_string(),
            enabled: 1,
            config: None,
            installed_at: "2026-01-01T00:00:00+00:00".to_string(),
            updated_at: "2026-01-01T00:00:00+00:00".to_string(),
            tenant_id: None,
        }
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let record = sample_record("test-ext");
        insert(&pool, &record).await.unwrap();

        let found = find_by_id(&pool, "test-ext").await.unwrap().unwrap();
        assert_eq!(found.id, "test-ext");
        assert_eq!(found.name, "Test test-ext");
        assert_eq!(found.version, "1.0.0");
        assert_eq!(found.enabled, 1);
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        let result = find_by_id(&pool, "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn find_all_returns_all() {
        let pool = setup_pool().await;
        insert(&pool, &sample_record("ext-a")).await.unwrap();
        insert(&pool, &sample_record("ext-b")).await.unwrap();

        let all = find_all(&pool).await.unwrap();
        assert_eq!(all.len(), 2);
        let ids: Vec<&str> = all.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"ext-a"));
        assert!(ids.contains(&"ext-b"));
    }

    #[tokio::test]
    async fn set_enabled_toggle() {
        let pool = setup_pool().await;
        insert(&pool, &sample_record("toggle-ext")).await.unwrap();

        set_enabled(&pool, "toggle-ext", false, "2026-02-01T00:00:00+00:00")
            .await
            .unwrap();
        let found = find_by_id(&pool, "toggle-ext").await.unwrap().unwrap();
        assert_eq!(found.enabled, 0);
        assert_eq!(found.updated_at, "2026-02-01T00:00:00+00:00");

        set_enabled(&pool, "toggle-ext", true, "2026-03-01T00:00:00+00:00")
            .await
            .unwrap();
        let found = find_by_id(&pool, "toggle-ext").await.unwrap().unwrap();
        assert_eq!(found.enabled, 1);
    }

    #[tokio::test]
    async fn update_version_changes_name_and_version() {
        let pool = setup_pool().await;
        insert(&pool, &sample_record("update-ext")).await.unwrap();

        update_version(
            &pool,
            "update-ext",
            "2.0.0",
            "Updated Name",
            "2026-06-01T00:00:00+00:00",
        )
        .await
        .unwrap();

        let found = find_by_id(&pool, "update-ext").await.unwrap().unwrap();
        assert_eq!(found.version, "2.0.0");
        assert_eq!(found.name, "Updated Name");
    }

    #[tokio::test]
    async fn delete_removes_record() {
        let pool = setup_pool().await;
        insert(&pool, &sample_record("del-ext")).await.unwrap();
        assert!(find_by_id(&pool, "del-ext").await.unwrap().is_some());

        delete(&pool, "del-ext").await.unwrap();
        assert!(find_by_id(&pool, "del-ext").await.unwrap().is_none());
    }
}
