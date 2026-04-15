//! 站点配置模型与数据库查询
//!
//! `options` 表每行含完整元数据（类型、分组、标签、校验规则），
//! 读取时可直接返回给前端渲染分组表单。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// options 表行模型（含完整元数据）
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct OptionRow {
    pub id: String,
    pub tenant_id: String,
    pub key: String,
    pub value: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
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

/// 查询所有 autoload 的配置（启动时预加载）
pub async fn find_autoload(pool: &crate::db::Pool) -> AppResult<Vec<OptionRow>> {
    let sql = crate::db::dialect::translate(
        "SELECT id, tenant_id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at FROM options WHERE autoload = 1 AND tenant_id = 'default'",
    );
    let rows = sqlx::query_as::<_, OptionRow>(&sql)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 根据 key 查询单条配置
pub async fn find_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: &str,
) -> AppResult<Option<OptionRow>> {
    let sql = crate::db::dialect::translate(
        "SELECT id, tenant_id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at FROM options WHERE tenant_id = ? AND key = ?",
    );
    let row = sqlx::query_as::<_, OptionRow>(&sql)
        .bind(tenant_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 查询所有配置
pub async fn find_all(pool: &crate::db::Pool, tenant_id: &str) -> AppResult<Vec<OptionRow>> {
    let sql = crate::db::dialect::translate(
        "SELECT id, tenant_id, key, value, type, group_name, label, description, validation, is_public, autoload, sort_order, updated_at FROM options WHERE tenant_id = ? ORDER BY sort_order, key",
    );
    let rows = sqlx::query_as::<_, OptionRow>(&sql)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// 插入或更新配置 value（UPSERT by tenant_id + key）
pub async fn upsert_value(
    pool: &crate::db::Pool,
    key: &str,
    value: &str,
    tenant_id: &str,
    updated_at: &str,
) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "UPDATE options SET value = ?, updated_at = ? WHERE tenant_id = ? AND key = ?",
    );
    sqlx::query(&sql)
        .bind(value)
        .bind(updated_at)
        .bind(tenant_id)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}

/// 根据 key 删除配置
pub async fn delete_by_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: &str,
) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "DELETE FROM options WHERE tenant_id = ? AND key = ?",
    );
    sqlx::query(&sql)
        .bind(tenant_id)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(())
}
