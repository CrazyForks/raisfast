//! Email verification token model and database queries

use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::id;
use crate::utils::tz::Timestamp;

/// Email verification token database row model
#[derive(Debug, FromRow)]
#[non_exhaustive]
pub struct EmailVerificationToken {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub token: String,
    pub email: String,
    pub expires_at: Timestamp,
    pub verified_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// Create a new email verification token
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

    let expires_at = crate::utils::tz::now_utc() + chrono::Duration::seconds(expires_in_secs);

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
        .bind(expires_at)
        .bind(now)
        .execute(pool)
        .await?;

    find_by_token(pool, &token).await?.ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "failed to fetch verification token"
        ))
    })
}

/// Find an unverified token record by token string
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

/// Mark a token as verified
pub async fn mark_verified(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE email_verification_tokens SET verified_at = {} WHERE id = {}",
        ph(1),
        ph(2),
    );
    sqlx::query(&sql).bind(now).bind(id).execute(pool).await?;
    Ok(())
}

/// Delete all unused verification tokens for a user
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM email_verification_tokens WHERE user_id = {} AND verified_at IS NULL",
        ph(1),
    );
    sqlx::query(&sql).bind(user_id).execute(pool).await?;
    Ok(())
}

/// Clean up expired verification tokens
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    let now = crate::utils::tz::now_utc();
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

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            None,
        )
        .await
        .unwrap();
        user.id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
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
        let user_id = insert_user(&pool).await;
        let row = create(&pool, user_id, "ev2@test.com", 3600).await.unwrap();
        assert!(row.verified_at.is_none());
        super::mark_verified(&pool, row.id).await.unwrap();
        let found = find_by_token(&pool, &row.token).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_unused_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
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
        let user_id = insert_user(&pool).await;
        create(&pool, user_id, "ev4@test.com", -1).await.unwrap();
        let removed = super::cleanup_expired(&pool).await.unwrap();
        assert_eq!(removed, 1);
    }
}
