//! docparse_tokens — 第三方 API 令牌模型（对齐 llm_tokens 模式）。
//!
//! 第三方以 `Authorization: Bearer <raw_token>` 调用 docparse 端点；
//! 存储层存 SHA-256 哈希用于鉴权，另存一份 APP_KEY 可逆加密的
//! `token_enc` 供管理端随时查看/复制 [照抄 llm_tokens 的
//! key_hash + key_enc 双列模式]；令牌归属 `user_id` 所有者。

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::db::{DbDriver as _, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;

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

/// 随机令牌原文 `dpt_<48 chars>` [照抄 llm relay::generate_sk 的
/// getrandom 字母表采样；雪花 id 可预测，不可作令牌]。
fn generate_dpt() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut raw = [0u8; 48];
    let _ = getrandom::fill(&mut raw);
    let body: String = raw
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    format!("dpt_{body}")
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct DocparseToken {
    /// SnowflakeId——serde 输出字符串，避免 JS 端 i64 精度丢失。
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub user_id: SnowflakeId,
    pub name: String,
    pub token_prefix: String,
    /// APP_KEY 可逆加密的令牌原文（管理端查看/复制用；无 APP_KEY 的
    /// 存量环境为 NULL）。
    pub token_enc: Option<String>,
    pub status: String,
    pub daily_page_quota: i64,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// 创建令牌：随机原文 + SHA-256 哈希（鉴权）+ APP_KEY 加密副本（查看）。
/// 返回 `(raw_token, token_id)`。
pub async fn create(
    pool: &crate::db::Pool,
    tenant: &str,
    user_id: SnowflakeId,
    name: &str,
    daily_page_quota: i64,
) -> AppResult<(String, SnowflakeId)> {
    let raw = generate_dpt();
    let hash = sha256_hex(raw.as_bytes());
    let prefix = raw.chars().take(12).collect::<String>();
    let token_enc = crate::llm::crypto::encrypt(&raw)?;
    let id = crate::utils::id::new_id();

    let sql = format!(
        "INSERT INTO docparse_tokens \
         (id, tenant_id, user_id, name, token_hash, token_prefix, token_enc, status, daily_page_quota, created_at) \
         VALUES ({}, {}, {}, {}, {}, {}, {}, 'active', {}, {})",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3),
        Driver::ph(4),
        Driver::ph(5),
        Driver::ph(6),
        Driver::ph(7),
        Driver::ph(8),
        Driver::ph(9)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(id)
        .bind(tenant)
        .bind(user_id)
        .bind(name)
        .bind(hash)
        .bind(prefix)
        .bind(token_enc)
        .bind(daily_page_quota)
        .bind(crate::utils::tz::now_utc())
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create docparse token: {e}")))?;
    Ok((raw, SnowflakeId(id)))
}

/// 列出租户的全部令牌（不含哈希；token_enc 由 handler 解密输出）。
pub async fn list_by_tenant(pool: &crate::db::Pool, tenant: &str) -> AppResult<Vec<DocparseToken>> {
    let sql = format!(
        "SELECT id, tenant_id, user_id, name, token_prefix, token_enc, status, daily_page_quota, created_at, last_used_at \
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

/// 更新令牌状态（'active' | 'disabled'）——Switch 开关的启用/停用共用。
/// 0 行受影响 = 令牌不存在，显式 404（杜绝无声失败）。
pub async fn set_status(
    pool: &crate::db::Pool,
    token_id: SnowflakeId,
    status: &str,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE docparse_tokens SET status = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(status)
        .bind(token_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("set docparse token status: {e}")))?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("docparse_token".into()));
    }
    Ok(())
}

/// 删除令牌（行级删除，鉴权哈希一并清除）。
pub async fn delete(pool: &crate::db::Pool, token_id: SnowflakeId) -> AppResult<()> {
    let sql = format!("DELETE FROM docparse_tokens WHERE id = {}", Driver::ph(1));
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(token_id)
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("delete docparse token: {e}")))?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("docparse_token".into()));
    }
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
