//! 站点配置模型与数据库查询
//!
//! 定义 `options` 表的数据结构及 KV 存储的全部 CRUD 操作。
//! `value` 为 JSON 字符串，`autoload` 标记启动时预加载。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// options 表行模型
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct OptionRow {
    pub key: String,
    pub value: String,
    pub autoload: bool,
    pub updated_at: String,
}

/// 查询所有 autoload 的配置（启动时预加载）
pub async fn find_autoload(pool: &crate::db::Pool) -> AppResult<Vec<(String, String)>> {
    let rows =
        sqlx::query_as::<_, (String, String)>("SELECT key, value FROM options WHERE autoload = 1")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}

/// 根据 key 查询单条配置
pub async fn find_by_key(pool: &crate::db::Pool, key: &str) -> AppResult<Option<String>> {
    let sql = crate::db::dialect::translate("SELECT value FROM options WHERE key = ?");
    let row = sqlx::query_as::<_, (String,)>(&sql)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(v,)| v))
}

/// 查询所有配置
pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT key, value FROM options ORDER BY key")
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 插入或更新配置（UPSERT）
pub async fn upsert(
    pool: &crate::db::Pool,
    key: &str,
    value: &str,
    updated_at: &str,
) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "INSERT INTO options (key, value, autoload, updated_at) VALUES (?, ?, 1, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    );
    sqlx::query(&sql)
        .bind(key)
        .bind(value)
        .bind(updated_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// 根据 key 删除配置
pub async fn delete_by_key(pool: &crate::db::Pool, key: &str) -> AppResult<()> {
    let sql = crate::db::dialect::translate("DELETE FROM options WHERE key = ?");
    sqlx::query(&sql).bind(key).execute(pool).await?;
    Ok(())
}
