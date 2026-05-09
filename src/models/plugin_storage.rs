//! 插件 KV 存储模型
//!
//! 插件通过 Host API (`setData`/`getData`) 存取持久化数据。
//! 每个插件只能访问自己 `plugin_id` 下的键值对。

use sqlx::FromRow;

use crate::db::Pool;
use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// 插件存储行
#[derive(Debug, FromRow)]
pub struct PluginStorageRow {
    pub plugin_id: String,
    pub storage_key: String,
    pub value: String,
    pub expires_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// 获取插件的 KV 数据
pub async fn get(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<Option<String>> {
    let row = sqlx::query_as::<_, PluginStorageRow>(&format!(
        "SELECT * FROM plugin_storage WHERE plugin_id = {} AND storage_key = {}",
        ph(1),
        ph(2),
    ))
    .bind(plugin_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            if let Some(exp) = &r.expires_at {
                let now = crate::utils::tz::now_utc();
                if exp < &now {
                    let _ = sqlx::query(&format!(
                        "DELETE FROM plugin_storage WHERE plugin_id = {} AND storage_key = {}",
                        ph(1),
                        ph(2),
                    ))
                    .bind(plugin_id)
                    .bind(key)
                    .execute(pool)
                    .await;
                    return Ok(None);
                }
            }
            Ok(Some(r.value))
        }
        None => Ok(None),
    }
}

/// 设置插件的 KV 数据
pub async fn set(
    pool: &Pool,
    plugin_id: &str,
    key: &str,
    value: &str,
    ttl_seconds: Option<i64>,
) -> AppResult<()> {
    let expires_at = ttl_seconds.map(|t| {
        crate::utils::tz::now_utc()
            .checked_add_signed(chrono::Duration::seconds(t))
            .unwrap_or_else(crate::utils::tz::now_utc)
    });

    let now = crate::db::dialect::now_fn();
    let assignments = format!(
        "value = {}, expires_at = {}, updated_at = {now}",
        crate::db::dialect::excluded_col("value"),
        crate::db::dialect::excluded_col("expires_at"),
    );
    let sql = format!(
        "INSERT INTO plugin_storage (plugin_id, storage_key, value, expires_at, updated_at) \
         VALUES ({}, {}, {}, {}, {now}) {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        crate::db::dialect::upsert_clause("plugin_id, storage_key", &assignments)
    );
    sqlx::query(&sql)
        .bind(plugin_id)
        .bind(key)
        .bind(value)
        .bind(expires_at)
        .execute(pool)
        .await?;

    Ok(())
}

/// 删除插件的某个 key
pub async fn delete(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<()> {
    sqlx::query(&format!(
        "DELETE FROM plugin_storage WHERE plugin_id = {} AND storage_key = {}",
        ph(1),
        ph(2),
    ))
    .bind(plugin_id)
    .bind(key)
    .execute(pool)
    .await?;
    Ok(())
}

/// 删除插件的所有数据
pub async fn delete_all(pool: &Pool, plugin_id: &str) -> AppResult<()> {
    sqlx::query(&format!(
        "DELETE FROM plugin_storage WHERE plugin_id = {}",
        ph(1),
    ))
    .bind(plugin_id)
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

    #[tokio::test]
    async fn set_and_get() {
        let pool = setup_pool().await;
        set(&pool, "plugin_set_get", "key1", "hello", None)
            .await
            .unwrap();
        let val = get(&pool, "plugin_set_get", "key1").await.unwrap();
        assert_eq!(val, Some("hello".to_string()));
    }

    #[tokio::test]
    async fn get_missing_returns_none() {
        let pool = setup_pool().await;
        let val = get(&pool, "plugin_missing", "nope").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn set_overwrites() {
        let pool = setup_pool().await;
        set(&pool, "plugin_overwrite", "key1", "v1", None)
            .await
            .unwrap();
        set(&pool, "plugin_overwrite", "key1", "v2", None)
            .await
            .unwrap();
        let val = get(&pool, "plugin_overwrite", "key1").await.unwrap();
        assert_eq!(val, Some("v2".to_string()));
    }

    #[tokio::test]
    async fn delete_key() {
        let pool = setup_pool().await;
        set(&pool, "plugin_delete", "key1", "val", None)
            .await
            .unwrap();
        delete(&pool, "plugin_delete", "key1").await.unwrap();
        let val = get(&pool, "plugin_delete", "key1").await.unwrap();
        assert!(val.is_none());
    }

    #[tokio::test]
    async fn delete_all_removes_all() {
        let pool = setup_pool().await;
        let pid = "plugin_delete_all";
        set(&pool, pid, "k1", "v1", None).await.unwrap();
        set(&pool, pid, "k2", "v2", None).await.unwrap();
        set(&pool, pid, "k3", "v3", None).await.unwrap();
        delete_all(&pool, pid).await.unwrap();
        assert!(get(&pool, pid, "k1").await.unwrap().is_none());
        assert!(get(&pool, pid, "k2").await.unwrap().is_none());
        assert!(get(&pool, pid, "k3").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_with_ttl() {
        let pool = setup_pool().await;
        set(&pool, "plugin_ttl", "key1", "expires", Some(1))
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let val = get(&pool, "plugin_ttl", "key1").await.unwrap();
        assert!(val.is_none());
    }
}
