//! 密码重置令牌模型与数据库查询
//!
//! 管理密码重置令牌的创建、查找、标记已用和过期清理。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::utils::id;

/// 密码重置令牌完整数据库行模型
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct PasswordResetToken {
    pub id: String,
    pub user_id: String,
    pub token: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub created_at: String,
}

/// 创建新的密码重置令牌
///
/// 生成 UUID v7 主键和 32 字节随机令牌，有效期由 `expires_in_secs` 控制。
pub async fn create(
    pool: &crate::db::Pool,
    user_id: &str,
    expires_in_secs: i64,
) -> AppResult<PasswordResetToken> {
    let (id, now) = id::new_id_and_timestamp();

    let mut token_bytes = [0u8; 32];
    getrandom::getrandom(&mut token_bytes).map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "reset token generation failed: {e}"
        ))
    })?;
    let token = hex::encode(token_bytes);

    let expires_at = (Utc::now() + chrono::Duration::seconds(expires_in_secs)).to_rfc3339();

    let sql = format!(
        "INSERT INTO password_reset_tokens (id, user_id, token, expires_at, created_at) VALUES ({}, {}, {}, {}, {})",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3),
        crate::db::dialect::ph(4),
        crate::db::dialect::ph(5),
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(user_id)
        .bind(&token)
        .bind(&expires_at)
        .bind(&now)
        .execute(pool)
        .await?;

    find_by_token(pool, &token).await?.ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "failed to fetch newly created password reset token"
        ))
    })
}

/// 根据令牌查找未使用的重置记录
pub async fn find_by_token(
    pool: &crate::db::Pool,
    token: &str,
) -> AppResult<Option<PasswordResetToken>> {
    let sql = format!(
        "SELECT * FROM password_reset_tokens WHERE token = {} AND used_at IS NULL",
        crate::db::dialect::ph(1),
    );
    let row = sqlx::query_as::<_, PasswordResetToken>(&sql)
        .bind(token)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 标记令牌为已使用
pub async fn mark_used(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE password_reset_tokens SET used_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
    );
    sqlx::query(&sql).bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// 删除用户所有未使用的重置令牌（在创建新令牌前调用，防止令牌堆积）
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: &str) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM password_reset_tokens WHERE user_id = {} AND used_at IS NULL",
        crate::db::dialect::ph(1),
    );
    sqlx::query(&sql).bind(user_id).execute(pool).await?;
    Ok(())
}

/// 清理已过期且未使用的令牌
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "DELETE FROM password_reset_tokens WHERE expires_at < {} AND used_at IS NULL",
        crate::db::dialect::ph(1),
    );
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
}
