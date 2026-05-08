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
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub token: String,
    pub email: String,
    pub expires_at: String,
    pub verified_at: Option<String>,
    pub created_at: String,
}

/// 创建新的邮箱验证令牌
pub async fn create(
    pool: &crate::db::Pool,
    user_id: i64,
    email: &str,
    expires_in_secs: i64,
) -> AppResult<EmailVerificationToken> {
    let (document_id, now) = id::new_document_id_and_timestamp();

    let mut token_bytes = [0u8; 32];
    getrandom::getrandom(&mut token_bytes).map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "verification token generation failed: {e}"
        ))
    })?;
    let token = hex::encode(token_bytes);

    let expires_at = (Utc::now() + chrono::Duration::seconds(expires_in_secs)).to_rfc3339();

    let sql = format!(
        "INSERT INTO email_verification_tokens (document_id, user_id, token, email, expires_at, created_at) VALUES ({}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
    );
    sqlx::query(&sql)
        .bind(&document_id)
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
pub async fn mark_verified(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
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
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<()> {
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

    async fn insert_user(pool: &crate::db::Pool, document_id: &str) -> i64 {
        let hash = "$argon2id$v=19$m=19456,t=2,p=1$test$test".to_string();
        let sql = format!(
            "INSERT INTO users (document_id, email, username, password_hash, role) VALUES ({}, {}, {}, {}, 'admin') RETURNING id",
            ph(1),
            ph(2),
            ph(3),
            ph(4)
        );
        let (id,): (i64,) = sqlx::query_as(&sql)
            .bind(document_id)
            .bind("ev-test@test.com")
            .bind("evtestuser")
            .bind(&hash)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool, "ev-user-1-doc").await;
        let row = create(&pool, user_id, "ev1@test.com", 3600).await.unwrap();
        let found = find_by_token(&pool, &row.token).await.unwrap().unwrap();
        assert_eq!(found.id, row.id);
        assert_eq!(found.token, row.token);
        assert_eq!(found.email, "ev1@test.com");
        assert!(found.verified_at.is_none());
    }

    #[tokio::test]
    async fn mark_verified() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool, "ev-user-2-doc").await;
        let row = create(&pool, user_id, "ev2@test.com", 3600).await.unwrap();
        assert!(row.verified_at.is_none());
        super::mark_verified(&pool, row.id).await.unwrap();
        let found = find_by_token(&pool, &row.token).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_unused_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool, "ev-user-3-doc").await;
        create(&pool, user_id, "ev3a@test.com", 3600).await.unwrap();
        create(&pool, user_id, "ev3b@test.com", 3600).await.unwrap();
        super::delete_unused_by_user(&pool, user_id).await.unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {}",
            ph(1),
        );
        let (count,): (i64,) = sqlx::query_as(&sql)
            .bind(user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn cleanup_expired() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool, "ev-user-4-doc").await;
        create(&pool, user_id, "ev4@test.com", -1).await.unwrap();
        let removed = super::cleanup_expired(&pool).await.unwrap();
        assert_eq!(removed, 1);
    }
}
