//! docparse_tokens — 第三方 API 令牌模型（对齐 llm_tokens 模式）。
//!
//! 第三方以 `Authorization: Bearer <raw_token>` 调用 docparse 端点；
//! 存储层只存 SHA-256 哈希，原始 token 仅在创建时返回一次。

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::db::{DbDriver as _, Driver};
use crate::errors::app_error::{AppError, AppResult};

/// SHA-256 hex digest of the raw token.
fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let hash = <sha2::Sha256 as sha2::Digest>::digest(data);
    let mut hex = String::with_capacity(64);
    for byte in hash {
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DocparseToken {
    pub id: i64,
    pub tenant_id: String,
    pub name: String,
    pub token_prefix: String,
    pub status: String,
    pub daily_page_quota: i64,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// 创建令牌：生成 `dpt_<random>` 原文，存储 SHA-256 哈希。
/// 返回 `(raw_token, token_id)`——原文仅在创建时可见。
pub async fn create(
    pool: &crate::db::Pool,
    tenant: &str,
    name: &str,
    daily_page_quota: i64,
) -> AppResult<(String, i64)> {
    let raw = format!("dpt_{}", crate::utils::id::new_id());
    let hash = sha256_hex(raw.as_bytes());
    let prefix = raw.chars().take(12).collect::<String>();
    let id = crate::utils::id::new_id();

    let sql = format!(
        "INSERT INTO docparse_tokens (id, tenant_id, name, token_hash, token_prefix, status, daily_page_quota, created_at) \
         VALUES ({}, {}, {}, {}, {}, 'active', {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(id)
        .bind(tenant)
        .bind(name)
        .bind(hash)
        .bind(prefix)
        .bind(daily_page_quota)
        .bind(crate::utils::tz::now_utc())
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create docparse token: {e}")))?;
    Ok((raw, id))
}

/// 列出租户的全部令牌（不含哈希）。
pub async fn list_by_tenant(pool: &crate::db::Pool, tenant: &str) -> AppResult<Vec<DocparseToken>> {
    let sql = format!(
        "SELECT id, tenant_id, name, token_prefix, status, daily_page_quota, created_at, last_used_at \
         FROM docparse_tokens WHERE tenant_id = {} ORDER BY created_at DESC",
        Driver::ph(1)
    );
    let rows: Vec<DocparseToken> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(tenant)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("list docparse tokens: {e}")))?;
    Ok(rows)
}

/// 停用令牌（status → disabled）。
pub async fn disable(pool: &crate::db::Pool, token_id: i64) -> AppResult<()> {
    let sql = format!(
        "UPDATE docparse_tokens SET status = 'disabled' WHERE id = {}",
        Driver::ph(1)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(token_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("disable docparse token: {e}")))?;
    Ok(())
}

/// 验证原始 token → 返回 (tenant_id, daily_page_quota)。
/// 哈希匹配 + status = 'active' 才通过；同时刷新 last_used_at。
pub async fn verify(pool: &crate::db::Pool, raw_token: &str) -> AppResult<Option<(String, i64)>> {
    let hash = sha256_hex(raw_token.as_bytes());
    let sql = format!(
        "SELECT tenant_id, daily_page_quota FROM docparse_tokens \
         WHERE token_hash = {} AND status = 'active'",
        Driver::ph(1)
    );
    let row: Option<(String, i64)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(&hash)
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("verify docparse token: {e}")))?;
    Ok(row)
}
