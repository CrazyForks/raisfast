//! Email verification token model and database queries

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Email verification token database row model
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct EmailVerificationToken {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub token: String,
    pub email: String,
    pub expires_at: Timestamp,
    pub verified_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

/// Create a new email verification token
pub async fn create(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    email: &str,
    expires_in_secs: i64,
) -> AppResult<EmailVerificationToken> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );

    let mut token_bytes = [0u8; 32];
    getrandom::getrandom(&mut token_bytes).map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "verification token generation failed: {e}"
        ))
    })?;
    let token = hex::encode(token_bytes);

    let expires_at = crate::utils::tz::now_utc() + chrono::Duration::seconds(expires_in_secs);

    raisfast_derive::crud_insert!(pool, "email_verification_tokens", [
        "id" => id,
        "user_id" => user_id,
        "token" => &token,
        "email" => email,
        "expires_at" => expires_at,
        "created_at" => now
    ])?;

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
    Ok(raisfast_derive::crud_find!(
        pool,
        "email_verification_tokens",
        EmailVerificationToken,
        where: AND(("token", token), ("verified_at", IS_NULL))
    )?)
}

/// Mark a token as verified
pub async fn mark_verified(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "email_verification_tokens",
        bind: ["verified_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

/// Delete all unused verification tokens for a user
pub async fn delete_unused_by_user(pool: &crate::db::Pool, user_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "email_verification_tokens",
        where: AND(("user_id", user_id), ("verified_at", IS_NULL))
    )?;
    Ok(())
}

/// Clean up expired verification tokens
pub async fn cleanup_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    raisfast_derive::check_schema!("email_verification_tokens", "expires_at", "verified_at");
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "DELETE FROM email_verification_tokens WHERE expires_at < {} AND verified_at IS NULL",
        Driver::ph(1),
    );
    let result = sqlx::query(&sql).bind(now).execute(pool).await?;
    Ok(result.rows_affected())
}

pub async fn tx_mark_verified(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    raisfast_derive::crud_update!(&mut *tx, "email_verification_tokens",
        bind: ["verified_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        *user.id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let row = create(&pool, SnowflakeId(user_id), "ev1@test.com", 3600)
            .await
            .unwrap();
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
        let row = create(&pool, SnowflakeId(user_id), "ev2@test.com", 3600)
            .await
            .unwrap();
        assert!(row.verified_at.is_none());
        super::mark_verified(&pool, row.id).await.unwrap();
        let found = find_by_token(&pool, &row.token).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_unused_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create(&pool, SnowflakeId(user_id), "ev3a@test.com", 3600)
            .await
            .unwrap();
        create(&pool, SnowflakeId(user_id), "ev3b@test.com", 3600)
            .await
            .unwrap();
        super::delete_unused_by_user(&pool, SnowflakeId(user_id))
            .await
            .unwrap();
        let sql = format!(
            "SELECT COUNT(*) FROM email_verification_tokens WHERE user_id = {}",
            Driver::ph(1),
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
        create(&pool, SnowflakeId(user_id), "ev4@test.com", -1)
            .await
            .unwrap();
        let removed = super::cleanup_expired(&pool).await.unwrap();
        assert_eq!(removed, 1);
    }
}
