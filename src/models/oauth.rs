//! OAuth account binding model and database queries
//!
//! Defines data structures and CRUD operations for `oauth_accounts` and `oauth_states` tables.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// OAuth account binding record
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct OAuthAccount {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
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
    pub id: i64,
    pub document_id: String,
    pub provider: String,
    pub code_verifier: String,
    pub user_id: Option<i64>,
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
) -> AppResult<String> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO oauth_states (document_id, provider, code_verifier, user_id, expires_at, created_at) VALUES ({}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6)
    );
    let mut q = sqlx::query(&sql)
        .bind(&document_id)
        .bind(provider)
        .bind(code_verifier);
    q = if let Some(uid) = user_id {
        q.bind(uid)
    } else {
        q.bind(Option::<i64>::None)
    };
    q.bind(expires_at).bind(now).execute(pool).await?;
    Ok(document_id)
}

/// Find and delete a state by document_id (one-time use)
pub async fn consume_state(
    pool: &crate::db::Pool,
    document_id: &str,
) -> AppResult<Option<OAuthState>> {
    let sql = format!(
        "SELECT * FROM oauth_states WHERE document_id = {} AND expires_at > {}",
        ph(1),
        crate::db::dialect::now_fn(),
    );
    let state = sqlx::query_as::<_, OAuthState>(&sql)
        .bind(document_id)
        .fetch_optional(pool)
        .await?;

    if state.is_some() {
        let del_sql = format!("DELETE FROM oauth_states WHERE document_id = {}", ph(1));
        sqlx::query(&del_sql)
            .bind(document_id)
            .execute(pool)
            .await?;
    }

    Ok(state)
}

/// Clean up expired OAuth state records
pub async fn cleanup_expired_states(pool: &crate::db::Pool) -> AppResult<u64> {
    let sql = format!(
        "DELETE FROM oauth_states WHERE expires_at <= {}",
        crate::db::dialect::now_fn(),
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
    let sql = format!(
        "SELECT * FROM oauth_accounts WHERE provider = {} AND provider_user_id = {}",
        ph(1),
        ph(2)
    );
    let account = sqlx::query_as::<_, OAuthAccount>(&sql)
        .bind(provider)
        .bind(provider_user_id)
        .fetch_optional(pool)
        .await?;
    Ok(account)
}

/// Find all OAuth bindings for a user
pub async fn find_by_user_id(pool: &crate::db::Pool, user_id: i64) -> AppResult<Vec<OAuthAccount>> {
    let sql = format!(
        "SELECT * FROM oauth_accounts WHERE user_id = {} ORDER BY created_at",
        ph(1)
    );
    let accounts = sqlx::query_as::<_, OAuthAccount>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(accounts)
}

/// Parameters for creating an OAuth account binding
pub struct CreateOAuthAccountParams<'a> {
    pub user_id: i64,
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
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    let sql = format!(
        "INSERT INTO oauth_accounts (document_id, user_id, provider, provider_user_id, email, display_name, avatar_url, access_token, refresh_token, token_expires_at, profile, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10),
        ph(11),
        ph(12),
        ph(13)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(params.user_id)
        .bind(params.provider)
        .bind(params.provider_user_id)
        .bind(params.email)
        .bind(params.display_name)
        .bind(params.avatar_url)
        .bind(params.access_token)
        .bind(params.refresh_token)
        .bind(params.token_expires_at)
        .bind(params.profile)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

    let sql2 = format!("SELECT * FROM oauth_accounts WHERE document_id = {}", ph(1));
    let account = sqlx::query_as::<_, OAuthAccount>(&sql2)
        .bind(&document_id)
        .fetch_one(pool)
        .await?;

    Ok(account)
}

/// Parameters for updating an OAuth account binding
pub struct UpdateOAuthAccountParams<'a> {
    pub id: i64,
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
    let sql = format!(
        "UPDATE oauth_accounts SET updated_at = {}, email = {}, display_name = {}, avatar_url = {}, access_token = {}, refresh_token = {}, token_expires_at = {}, profile = {} WHERE id = {}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9)
    );
    sqlx::query(&sql)
        .bind(now)
        .bind(params.email)
        .bind(params.display_name)
        .bind(params.avatar_url)
        .bind(params.access_token)
        .bind(params.refresh_token)
        .bind(params.token_expires_at)
        .bind(params.profile)
        .bind(params.id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete an OAuth account binding (unlink)
pub async fn delete_account(
    pool: &crate::db::Pool,
    user_id: i64,
    provider: &str,
) -> AppResult<bool> {
    let sql = format!(
        "DELETE FROM oauth_accounts WHERE user_id = {} AND provider = {}",
        ph(1),
        ph(2)
    );
    let result = sqlx::query(&sql)
        .bind(user_id)
        .bind(provider)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Count the number of OAuth providers bound to a user
pub async fn count_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<i64> {
    let sql = format!(
        "SELECT COUNT(*) FROM oauth_accounts WHERE user_id = {}",
        ph(1)
    );
    let (count,) = sqlx::query_as::<_, (i64,)>(&sql)
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(count)
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
        let cmd = crate::commands::user::CreateUserCmd {
            username: crate::utils::id::new_document_id(),
            registered_via: crate::models::user::RegisteredVia::Email,
        };
        let user = crate::models::user::create(pool, &cmd, None).await.unwrap();
        user.id
    }

    #[tokio::test]
    async fn create_and_consume_state() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let doc_id = create_state(
            &pool,
            "github",
            "verifier123",
            Some(user_id),
            "2099-12-31T00:00:00Z",
        )
        .await
        .unwrap();
        let state = consume_state(&pool, &doc_id).await.unwrap().unwrap();
        assert_eq!(state.document_id, doc_id);
        assert_eq!(state.provider, "github");
        assert_eq!(state.code_verifier, "verifier123");
        assert_eq!(state.user_id, Some(user_id));
    }

    #[tokio::test]
    async fn consume_state_twice_returns_none() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let doc_id = create_state(
            &pool,
            "github",
            "verifier123",
            Some(user_id),
            "2099-12-31T00:00:00Z",
        )
        .await
        .unwrap();
        let first = consume_state(&pool, &doc_id).await.unwrap();
        assert!(first.is_some());
        let second = consume_state(&pool, &doc_id).await.unwrap();
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn create_and_find_account() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let account = create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id,
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
                user_id,
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
                user_id,
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
        let accounts = super::find_by_user_id(&pool, user_id).await.unwrap();
        assert_eq!(accounts.len(), 2);
    }

    #[tokio::test]
    async fn delete_account() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        create_account(
            &pool,
            CreateOAuthAccountParams {
                user_id,
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
        let deleted = super::delete_account(&pool, user_id, "github")
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
                user_id,
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
                user_id,
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
        let count = super::count_by_user(&pool, user_id).await.unwrap();
        assert_eq!(count, 2);
    }
}
