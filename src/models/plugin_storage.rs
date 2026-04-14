//! 插件 KV 存储模型
//!
//! 插件通过 Host API (`setData`/`getData`) 存取持久化数据。
//! 每个插件只能访问自己 plugin_id 下的键值对。

use sqlx::FromRow;

use crate::db::Pool;
use crate::errors::app_error::AppResult;

/// 插件存储行
#[derive(Debug, FromRow)]
pub struct PluginStorageRow {
    pub plugin_id: String,
    pub key: String,
    pub value: String,
    pub expires_at: Option<String>,
    pub updated_at: String,
}

/// 获取插件的 KV 数据
pub async fn get(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<Option<String>> {
    let row = sqlx::query_as::<_, PluginStorageRow>(
        "SELECT * FROM plugin_storage WHERE plugin_id = ? AND key = ?",
    )
    .bind(plugin_id)
    .bind(key)
    .fetch_optional(pool)
    .await?;

    match row {
        Some(r) => {
            if let Some(exp) = &r.expires_at {
                let now = chrono::Utc::now().to_rfc3339();
                if exp < &now {
                    let _ =
                        sqlx::query("DELETE FROM plugin_storage WHERE plugin_id = ? AND key = ?")
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
        chrono::Utc::now()
            .checked_add_signed(chrono::Duration::seconds(t))
            .unwrap_or_else(chrono::Utc::now)
            .to_rfc3339()
    });

    sqlx::query(
        "INSERT INTO plugin_storage (plugin_id, key, value, expires_at, updated_at) \
         VALUES (?, ?, ?, ?, datetime('now')) \
         ON CONFLICT(plugin_id, key) DO UPDATE SET value = excluded.value, expires_at = excluded.expires_at, updated_at = datetime('now')",
    )
    .bind(plugin_id)
    .bind(key)
    .bind(value)
    .bind(&expires_at)
    .execute(pool)
    .await?;

    Ok(())
}

/// 删除插件的某个 key
pub async fn delete(pool: &Pool, plugin_id: &str, key: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM plugin_storage WHERE plugin_id = ? AND key = ?")
        .bind(plugin_id)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除插件的所有数据
pub async fn delete_all(pool: &Pool, plugin_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM plugin_storage WHERE plugin_id = ?")
        .bind(plugin_id)
        .execute(pool)
        .await?;
    Ok(())
}
