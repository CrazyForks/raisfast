//! 站点配置模型与数据库查询
//!
//! `options` 表每行含完整元数据（类型、分组、标签、校验规则），
//! 读取时可直接返回给前端渲染分组表单。

use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;

/// options 表行模型（含完整元数据）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OptionRow {
    pub id: String,
    pub tenant_id: Option<String>,
    pub key: String,
    pub value: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub group_name: String,
    pub label: String,
    pub description: Option<String>,
    pub validation: Option<String>,
    pub is_public: bool,
    pub autoload: bool,
    pub sort_order: i64,
    pub updated_at: String,
}

#[cfg(feature = "db-sqlite")]
impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for OptionRow {
    fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            tenant_id: row.try_get("tenant_id").ok(),
            id: row.try_get("id")?,
            key: row.try_get("key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
#[cfg(feature = "db-postgres")]
impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for OptionRow {
    fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            tenant_id: row.try_get("tenant_id").ok(),
            id: row.try_get("id")?,
            key: row.try_get("key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
#[cfg(feature = "db-mysql")]
impl<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> for OptionRow {
    fn from_row(row: &'r sqlx::mysql::MySqlRow) -> Result<Self, sqlx::Error> {
        use sqlx::Row;
        Ok(Self {
            tenant_id: row.try_get("tenant_id").ok(),
            id: row.try_get("id")?,
            key: row.try_get("key")?,
            value: row.try_get("value")?,
            type_: row.try_get("type")?,
            group_name: row.try_get("group_name")?,
            label: row.try_get("label")?,
            description: row.try_get("description")?,
            validation: row.try_get("validation")?,
            is_public: row.try_get("is_public")?,
            autoload: row.try_get("autoload")?,
            sort_order: row.try_get("sort_order")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

/// 查询所有 autoload 的配置（启动时预加载）
pub async fn find_autoload(pool: &crate::db::Pool) -> AppResult<Vec<OptionRow>> {
    let sql = crate::db::dialect::translate(
        "SELECT * FROM options WHERE autoload = 1",
    );
    let rows = sqlx::query_as::<_, OptionRow>(&sql).fetch_all(pool).await?;
    Ok(rows)
}

/// 根据 key 查询单条配置
pub async fn find_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<OptionRow>> {
    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "SELECT * FROM options WHERE tenant_id = ? AND key = ?",
            );
            let row = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(tid)
                .bind(key)
                .fetch_optional(pool)
                .await?;
            Ok(row)
        }
        None => {
            let sql = crate::db::dialect::translate(
                "SELECT * FROM options WHERE key = ?",
            );
            let row = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(key)
                .fetch_optional(pool)
                .await?;
            Ok(row)
        }
    }
}

/// 查询所有配置
pub async fn find_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<OptionRow>> {
    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "SELECT * FROM options WHERE tenant_id = ? ORDER BY sort_order, key",
            );
            let rows = sqlx::query_as::<_, OptionRow>(&sql)
                .bind(tid)
                .fetch_all(pool)
                .await?;
            Ok(rows)
        }
        None => {
            let sql = crate::db::dialect::translate(
                "SELECT * FROM options ORDER BY sort_order, key",
            );
            let rows = sqlx::query_as::<_, OptionRow>(&sql)
                .fetch_all(pool)
                .await?;
            Ok(rows)
        }
    }
}

/// 插入或更新配置 value（UPSERT by key）
pub async fn upsert_value(
    pool: &crate::db::Pool,
    key: &str,
    value: &str,
    tenant_id: Option<&str>,
    updated_at: &str,
) -> AppResult<()> {
    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate(
                "UPDATE options SET value = ?, updated_at = ? WHERE tenant_id = ? AND key = ?",
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(updated_at)
                .bind(tid)
                .bind(key)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate(
                "UPDATE options SET value = ?, updated_at = ? WHERE key = ?",
            );
            sqlx::query(&sql)
                .bind(value)
                .bind(updated_at)
                .bind(key)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// 根据 key 删除配置
pub async fn delete_by_key(pool: &crate::db::Pool, key: &str, tenant_id: Option<&str>) -> AppResult<()> {
    match tenant_id {
        Some(tid) => {
            let sql = crate::db::dialect::translate("DELETE FROM options WHERE tenant_id = ? AND key = ?");
            sqlx::query(&sql)
                .bind(tid)
                .bind(key)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = crate::db::dialect::translate("DELETE FROM options WHERE key = ?");
            sqlx::query(&sql)
                .bind(key)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}
