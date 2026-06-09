//! OAuth account binding model and database queries
//!
//! Defines data structures and CRUD operations for `oauth_accounts` and `oauth_states` tables.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// OAuth account binding record
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct OAuthAccount {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub provider: String,
    pub provider_user_id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<Timestamp>,
    pub profile: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// OAuth short-lived state record
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct OAuthState {
    pub id: SnowflakeId,
    pub provider: String,
    pub code_verifier: String,
    pub user_id: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// Create an OAuth state record
pub async fn create_state(
    pool: &crate::db::Pool,
    provider: &str,
    code_verifier: &str,
    user_id: Option<i64>,
    expires_at: &str,
) -> AppResult<i64> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(pool, "oauth_states", [
        "id" => id,
        "provider" => provider,
        "code_verifier" => code_verifier,
        "user_id" => user_id,
        "expires_at" => expires_at,
        "created_at" => now,
    ])?;
    Ok(*id)
}

/// Find and delete a state by id (one-time use)
pub async fn consume_state(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<OAuthState>> {
    raisfast_derive::check_schema!("oauth_states", "id", "expires_at");
    let sql = format!(
        "SELECT * FROM oauth_states WHERE id = {} AND expires_at > {}",
        Driver::ph(1),
        crate::db::Driver::now_fn(),
    );
    let state = sqlx::query_as::<_, OAuthState>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;

    if state.is_some() {
        raisfast_derive::crud_delete!(pool, "oauth_states", where: ("id", id))?;
    }

    Ok(state)
}

/// Clean up expired OAuth state records
pub async fn cleanup_expired_states(pool: &crate::db::Pool) -> AppResult<u64> {
    let sql = format!(
        "DELETE FROM oauth_states WHERE expires_at <= {}",
        crate::db::Driver::now_fn(),
    );
    let result = sqlx::query(&sql).execute(pool).await?;
    Ok(result.rows_affected())
}

/// Find a binding by Provider + Provider user ID
pub async fn find_by_provider_user(
    pool: &crate::db::Pool,
    provider: &str,
    provider_user_id: &str,
) -> AppResult<Option<OAuthAccount>> {
    raisfast_derive::crud_find!(pool, "oauth_accounts", OAuthAccount, where: AND(("provider", provider), ("provider_user_id", provider_user_id)))
        .map_err(Into::into)
}

/// Find all OAuth bindings for a user
pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
) -> AppResult<Vec<OAuthAccount>> {
    raisfast_derive::check_schema!("oauth_accounts", "user_id", "created_at");
    let accounts = raisfast_derive::crud_find_all!(pool, "oauth_accounts", OAuthAccount, where: ("user_id", user_id), order_by: "created_at")?;
    Ok(accounts)
}

/// Parameters for creating an OAuth account binding
pub struct CreateOAuthAccountParams<'a> {
    pub user_id: SnowflakeId,
    pub provider: &'a str,
    pub provider_user_id: &'a str,
    pub email: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub access_token: Option<&'a str>,
    pub refresh_token: Option<&'a str>,
    pub token_expires_at: Option<&'a str>,
    pub profile: Option<&'a str>,
}

/// Create an OAuth account binding
pub async fn create_account(
    pool: &crate::db::Pool,
    params: CreateOAuthAccountParams<'_>,
) -> AppResult<OAuthAccount> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );

    raisfast_derive::crud_insert!(pool, "oauth_accounts", [
        "id" => id,
        "user_id" => params.user_id,
        "provider" => params.provider,
        "provider_user_id" => params.provider_user_id,
        "email" => params.email,
        "display_name" => params.display_name,
        "avatar_url" => params.avatar_url,
        "access_token" => params.access_token,
        "refresh_token" => params.refresh_token,
        "token_expires_at" => params.token_expires_at,
        "profile" => params.profile,
        "created_at" => now,
        "updated_at" => now,
    ])?;

    Ok(raisfast_derive::crud_find_one!(pool, "oauth_accounts", OAuthAccount, where: ("id", id))?)
}

/// Parameters for updating an OAuth account binding
pub struct UpdateOAuthAccountParams<'a> {
    pub id: SnowflakeId,
    pub email: Option<&'a str>,
    pub display_name: Option<&'a str>,
    pub avatar_url: Option<&'a str>,
    pub access_token: Option<&'a str>,
    pub refresh_token: Option<&'a str>,
    pub token_expires_at: Option<&'a str>,
    pub profile: Option<&'a str>,
}

/// Update OAuth account binding information
pub async fn update_account(
    pool: &crate::db::Pool,
    params: UpdateOAuthAccountParams<'_>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "oauth_accounts",
        bind: [
            "updated_at" => now,
            "email" => params.email,
            "display_name" => params.display_name,
            "avatar_url" => params.avatar_url,
            "access_token" => params.access_token,
            "refresh_token" => params.refresh_token,
            "token_expires_at" => params.token_expires_at,
            "profile" => params.profile,
        ],
        where: ("id", params.id)
    )?;
    Ok(())
}

/// Delete an OAuth account binding (unlink)
pub async fn delete_account(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    provider: &str,
) -> AppResult<bool> {
    let result = raisfast_derive::crud_delete!(pool, "oauth_accounts", where: AND(("user_id", user_id), ("provider", provider)))?;
    Ok(result.rows_affected() > 0)
}

/// Count the number of OAuth providers bound to a user
pub async fn count_by_user(pool: &crate::db::Pool, user_id: SnowflakeId) -> AppResult<i64> {
    Ok(raisfast_derive::crud_count!(pool, "oauth_accounts", where: ("user_id", user_id))?)
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
    async fn create_and_consume_state() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let state_id = create_state(
            &pool,
            "github",
            "verifier123",
            Some(user_id),
            "2099-12-31T00:00:00Z",
        )
        .await
        .unwrap();
        let state = consume_state(&pool, SnowflakeId(state_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.provider, "github");
        assert_eq!(state.code_verifier, "verifier123");
        assert_eq!(state.user_id, Some(SnowflakeId(user_id)));
    }

    #[tokio::test]
    async fn consume_state_twice_returns_none() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let state_id = create_state(
            &pool,
            "github",
            "verifier123",
            Some(user_id),
            "2099-12-31T00:00:00Z",
        )
        .await
        .unwrap();
        let first = consume_state(&pool, SnowflakeId(state_id)).await.unwrap();
        assert!(first.is_some());
        let second = consume_state(&pool, SnowflakeId(state_id)).await.unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn create_and_find_account() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let account = create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "github",
                provider_user_id: "github-123",
                email: Some("user@example.com"),
                display_name: Some("Test User"),
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        let found = find_by_provider_user(&pool, "github", "github-123")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, account.id);
        assert_eq!(found.provider, "github");
        assert_eq!(found.provider_user_id, "github-123");
    }

    #[tokio::test]
    async fn find_by_user_id() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "github",
                provider_user_id: "github-123",
                email: None,
                display_name: None,
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "google",
                provider_user_id: "google-456",
                email: None,
                display_name: None,
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        let accounts = super::find_by_user_id(&pool, SnowflakeId(user_id))
            .await
            .unwrap();
        assert_eq!(accounts.len(), 2);
    }

    #[tokio::test]
    async fn delete_account() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "github",
                provider_user_id: "github-123",
                email: None,
                display_name: None,
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        let deleted = super::delete_account(&pool, SnowflakeId(user_id), "github")
            .await
            .unwrap();
        assert!(deleted);
        assert!(
            find_by_provider_user(&pool, "github", "github-123")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn count_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "github",
                provider_user_id: "github-123",
                email: None,
                display_name: None,
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id: SnowflakeId(user_id),
                provider: "google",
                provider_user_id: "google-456",
                email: None,
                display_name: None,
                avatar_url: None,
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                profile: None,
            },
        )
        .await
        .unwrap();
        let count = super::count_by_user(&pool, SnowflakeId(user_id))
            .await
            .unwrap();
        assert_eq!(count, 2);
    }
}
