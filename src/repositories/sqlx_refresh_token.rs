//! sqlx-based `RefreshTokenRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::refresh_token::{self, RefreshToken};

use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxRefreshTokenRepository);

/// Refresh token Repository interface
#[async_trait::async_trait]
pub trait RefreshTokenRepository: Send + Sync {
    /// Create a new refresh token record
    async fn create_token(&self, user_id: i64, token: &str, expires_at: &str) -> AppResult<()>;

    /// Find a refresh token by token string
    async fn find_by_token(&self, token: &str) -> AppResult<Option<RefreshToken>>;

    /// Delete a refresh token by token string
    async fn delete_by_token(&self, token: &str) -> AppResult<()>;

    /// Delete all refresh tokens for a given user
    async fn delete_by_user(&self, user_id: i64) -> AppResult<()>;
}

#[async_trait::async_trait]
impl RefreshTokenRepository for SqlxRefreshTokenRepository {
    async fn create_token(&self, user_id: i64, token: &str, expires_at: &str) -> AppResult<()> {
        refresh_token::create_token(&self.pool, user_id, token, expires_at).await
    }

    async fn find_by_token(&self, token: &str) -> AppResult<Option<RefreshToken>> {
        refresh_token::find_by_token(&self.pool, token).await
    }

    async fn delete_by_token(&self, token: &str) -> AppResult<()> {
        refresh_token::delete_by_token(&self.pool, token).await
    }

    async fn delete_by_user(&self, user_id: i64) -> AppResult<()> {
        refresh_token::delete_by_user(&self.pool, user_id).await
    }
}
