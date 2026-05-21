//! Plugin KV storage model
//!
//! Plugins persist data via the Host API (`setData`/`getData`).
//! Each plugin can only access key-value pairs under its own `plugin_id`.

use sqlx::FromRow;

use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// Plugin storage row
#[derive(Debug, FromRow)]
pub struct PluginStorageRow {
    pub plugin_id: String,
    pub storage_key: String,
    pub value: String,
    pub expires_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// Get a plugin's KV data
pub async fn get(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<Option<String>> {
    let row = sqlx::query_as::<_, PluginStorageRow>(&format!(
        "SELECT * FROM plugin_storage WHERE plugin_id = {} AND storage_key = {}",
        Driver::ph(1),
        Driver::ph(2),
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
                    let _ = raisfast_derive::crud_delete!(
                        pool, "plugin_storage", where: AND(("plugin_id", plugin_id), ("storage_key", key))
                    );
                    return Ok(None);
                }
            }
            Ok(Some(r.value))
        }
        None => Ok(None),
    }
}

/// Set a plugin's KV data
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
    let now = crate::utils::tz::now_utc();

    raisfast_derive::crud_upsert!(
        pool, "plugin_storage",
        key: ["plugin_id", "storage_key"],
        bind: ["plugin_id" => plugin_id, "storage_key" => key, "value" => value, "expires_at" => expires_at, "updated_at" => now],
        update: ["value", "expires_at", "updated_at"]
    )?;

    Ok(())
}

/// Delete a specific key for a plugin
pub async fn delete(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "plugin_storage",
        where: AND(("plugin_id", plugin_id), ("storage_key", key))
    )?;
    Ok(())
}

/// Delete all data for a plugin
pub async fn delete_all(pool: &Pool, plugin_id: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "plugin_storage", where: ("plugin_id", plugin_id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
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
