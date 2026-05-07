//! 邮箱验证令牌模型与数据库查询

use chrono::Utc;
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::id;

/// 邮箱验证令牌数据库行模型
#[derive(Debug, FromRow)]
#[non_exhaustive]
pub struct EmailVerificationToken {
    pub id: String,
    pub user_id: String,
    pub token: String,
    pub email: String,
    pub expires_at: String,
    pub verified_at: Option<String>,
    pub created_at: String,
}

/// 创建新的邮箱验证令牌
pub async fn create(
    pool: &crate::db::Pool,
    user_id: &str,
    email: &str,
    expires_in_secs: i64,
) -> AppResult<EmailVerificationToken> {
    let (id, now) = id::new_id_and_timestamp();

    let mut token_bytes = [0u8; 32];
    getrandom::getrandom(&mut token_bytes).map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "verification token generation failed: {e}"
        ))
    })?;
    let token = hex::encode(token_bytes);

    let expires_at = (Utc::now() + chrono::Duration::seconds(expires_in_secs)).to_rfc3339();

    let sql = format!(
        "INSERT INTO email_verification_tokens (id, user_id, token, email, expires_at, created_at) VALUES ({}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(user_id)
        .bind(&token)
        .bind(email)
        .bind(&expires_at)
        .bind(&now)
        .execute(pool)
        .await?;

    find_by_token(pool, &token).await?.ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "failed to fetch verification token"
        ))
    })
}

/// 根据令牌查找未验证的记录
pub async fn find_by_token(
    pool: &crate::db::Pool,
    token: &str,
) -> AppResult<Option<EmailVerificationToken>> {
    let sql = format!(
        "SELECT * FROM email_verification_tokens WHERE token = {} AND verified_at IS NULL",
        ph(1),
    );
    let row = sqlx::query_as::<_, EmailVerificationToken>(&sql)
        .bind(token)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 标记令牌为已验证
pub async fn mark_verified(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE email_verification_tokens SET verified_at = {} WHERE id = {}",
        ph(1),
        ph(2),
    );
    sqlx::query(&sql).bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// 删除用户所有未使用的验证令牌
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: &str) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
        ph(1),
    );
    sqlx::query(&sql).bind(user_id).execute(pool).await?;
    Ok(())
}

/// 清理过期的验证令牌
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "DELETE FROM email_verification_tokens WHERE expires_at < {} AND verified_at IS NULL",
        ph(1),
    );
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
}
