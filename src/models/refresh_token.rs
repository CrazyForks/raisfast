//! Refresh token model and database queries
//!
//! Defines the data structure for refresh tokens and create, find, delete operations
//! on the `refresh_tokens` table. Refresh tokens are used to obtain new token pairs
//! after access tokens expire, and are stored in the database to support active revocation.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Refresh token full database row model
///
/// Directly maps all fields of the `refresh_tokens` table.
/// `expires_at` is the expiration time in ISO 8601 format.
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub token: String,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
}

/// Create a new refresh token record
///
/// Automatically generates a Snowflake ID as the primary key.
pub async fn create_token(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    token: &str,
    expires_at: &str,
) -> AppResult<()> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(pool, "refresh_tokens", [
        "id" => id,
        "user_id" => user_id,
        "token" => token,
        "expires_at" => expires_at,
        "created_at" => now
    ])?;
    Ok(())
}

/// Find a refresh token by token string
///
/// Returns `Ok(Some(token))` or `Ok(None)` when not found.
pub async fn find_by_token(pool: &crate::db::Pool, token: &str) -> AppResult<Option<RefreshToken>> {
    let result: Option<RefreshToken> =
        raisfast_derive::crud_find!(pool, "refresh_tokens", RefreshToken, where: ("token", token))?;
    Ok(result)
}

/// Delete a refresh token by token string
///
/// Used to revoke a specific refresh token on logout.
pub async fn delete_by_token(pool: &crate::db::Pool, token: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "refresh_tokens", where: ("token", token))?;
    Ok(())
}

/// Delete all refresh tokens for a given user
///
/// Used for logging out all devices or forcing re-login after a password change.
pub async fn delete_by_user(pool: &crate::db::Pool, user_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "refresh_tokens", where: ("user_id", user_id))?;
    Ok(())
}

pub async fn tx_delete_by_token(
    tx: &mut crate::db::pool::DbConnection,
    token: &str,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(&mut *tx, "refresh_tokens", where: ("token", token))?;
    Ok(())
}

pub async fn tx_create_token(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    token: &str,
    expires_at: &str,
) -> AppResult<()> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(&mut *tx, "refresh_tokens", [
        "id" => id,
        "user_id" => user_id,
        "token" => token,
        "expires_at" => expires_at,
        "created_at" => now
    ])?;
    Ok(())
}

pub async fn tx_delete_by_user(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(&mut *tx, "refresh_tokens", where: ("user_id", user_id))?;
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
        let cmd = crate::commands::user::CreateUserCmd {
            username: crate::utils::id::new_id().to_string(),
            registered_via: crate::models::user::RegisteredVia::Email,
            role: None,
        };
        let user = crate::models::user::create(pool, &cmd, None).await.unwrap();
        *user.id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token = crate::utils::id::new_id().to_string();
        create_token(&pool, SnowflakeId(user_id), &token, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        let found = find_by_token(&pool, &token).await.unwrap().unwrap();
        assert_eq!(found.token, token);
        assert_eq!(found.user_id, SnowflakeId(user_id));
        assert_eq!(
            found.expires_at,
            "2099-12-31T00:00:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[tokio::test]
    async fn delete_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token = crate::utils::id::new_id().to_string();
        create_token(&pool, SnowflakeId(user_id), &token, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        super::delete_by_token(&pool, &token).await.unwrap();
        assert!(find_by_token(&pool, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token1 = crate::utils::id::new_id().to_string();
        let token2 = crate::utils::id::new_id().to_string();
        create_token(&pool, SnowflakeId(user_id), &token1, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        create_token(&pool, SnowflakeId(user_id), &token2, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        super::delete_by_user(&pool, SnowflakeId(user_id))
            .await
            .unwrap();
        assert!(find_by_token(&pool, &token1).await.unwrap().is_none());
        assert!(find_by_token(&pool, &token2).await.unwrap().is_none());
    }
}
