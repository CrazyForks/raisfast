//! 密码重置令牌模型与数据库查询
//!
//! 管理密码重置令牌的创建、查找、标记已用和过期清理。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::id;

/// 密码重置令牌完整数据库行模型
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct PasswordResetToken {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub token: String,
    pub expires_at: String,
    pub used_at: Option<String>,
    pub created_at: String,
}

/// 创建新的密码重置令牌
///
/// 生成 document_id 和 32 字节随机令牌，有效期由 `expires_in_secs` 控制。
pub async fn create(
    pool: &crate::db::Pool,
    user_id: i64,
    expires_in_secs: i64,
) -> AppResult<PasswordResetToken> {
    let (document_id, now) = id::new_document_id_and_timestamp();

    let mut token_bytes = [0u8; 32];
    getrandom::getrandom(&mut token_bytes).map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "reset token generation failed: {e}"
        ))
    })?;
    let token = hex::encode(token_bytes);

    let expires_at = (Utc::now() + chrono::Duration::seconds(expires_in_secs)).to_rfc3339();

    let sql = format!(
        "INSERT INTO password_reset_tokens (document_id, user_id, token, expires_at, created_at) VALUES ({}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
    );
    sqlx::query(&sql)
        .bind(&document_id)
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
        ph(1),
    );
    let row = sqlx::query_as::<_, PasswordResetToken>(&sql)
        .bind(token)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 标记令牌为已使用
pub async fn mark_used(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE password_reset_tokens SET used_at = {} WHERE id = {}",
        ph(1),
        ph(2),
    );
    sqlx::query(&sql).bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// 删除用户所有未使用的重置令牌（在创建新令牌前调用，防止令牌堆积）
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM password_reset_tokens WHERE user_id = {} AND used_at IS NULL",
        ph(1),
    );
    sqlx::query(&sql).bind(user_id).execute(pool).await?;
    Ok(())
}

/// 清理已过期且未使用的令牌
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "DELETE FROM password_reset_tokens WHERE expires_at < {} AND used_at IS NULL",
        ph(1),
    );
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
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

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = crate::utils::id::new_document_id();
        let sql = format!(
            "INSERT INTO users (document_id, email, username, password_hash, role) VALUES ({}, {}, {}, {}, 'admin') RETURNING id",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3),
            crate::db::dialect::ph(4)
        );
        let (id,): (i64,) = sqlx::query_as(&sql)
            .bind(&doc_id)
            .bind("pr-test@test.com")
            .bind("pruser")
            .bind("$argon2id$v=19$m=19456,t=2,p=1$test$test")
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(&pool, user_id, 3600).await.unwrap();
        assert!(row.id > 0);
        assert_eq!(row.user_id, user_id);
        assert!(!row.token.is_empty());
        assert!(row.used_at.is_none());

        let found = find_by_token(&pool, &row.token).await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
        assert_eq!(found.token, row.token);
    }

    #[tokio::test]
    async fn test_mark_used() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(&pool, user_id, 3600).await.unwrap();
        assert!(row.used_at.is_none());

        super::mark_used(&pool, row.id).await.unwrap();

        let found = find_by_token(&pool, &row.token).await.unwrap();
        assert!(
            found.is_none(),
            "used token should not be found by find_by_token"
        );

        let sql = format!(
            "SELECT used_at FROM password_reset_tokens WHERE id = {}",
            crate::db::dialect::ph(1),
        );
        let (used_at,): (Option<String>,) = sqlx::query_as(&sql)
            .bind(row.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(used_at.is_some(), "used_at should be set after mark_used");
    }

    #[tokio::test]
    async fn test_delete_unused_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row1 = create(&pool, user_id, 3600).await.unwrap();
        let row2 = create(&pool, user_id, 3600).await.unwrap();

        super::delete_unused_by_user(&pool, user_id).await.unwrap();

        let found1 = find_by_token(&pool, &row1.token).await.unwrap();
        let found2 = find_by_token(&pool, &row2.token).await.unwrap();
        assert!(found1.is_none());
        assert!(found2.is_none());
    }

    #[tokio::test]
    async fn test_cleanup_expired() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let _row = create(&pool, user_id, 1).await.unwrap();

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let removed = super::cleanup_expired(&pool).await.unwrap();
        assert_eq!(removed, 1);
    }
}
