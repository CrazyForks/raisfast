//! sqlx-based `RefreshTokenRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::refresh_token::RefreshToken;
use raisfast_derive::repository;

/// Refresh token Repository interface
#[repository(model = "refresh_token", struct_name = SqlxRefreshTokenRepository)]
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
