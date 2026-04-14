//! 基于 sqlx 的 RefreshTokenRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::refresh_token::{self, RefreshToken};

use super::RefreshTokenRepository;

/// 基于 sqlx 的刷新令牌 Repository
pub struct SqlxRefreshTokenRepository {
    pool: Pool,
}

impl SqlxRefreshTokenRepository {
    /// 创建新的 SqlxRefreshTokenRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl RefreshTokenRepository for SqlxRefreshTokenRepository {
    async fn create_token(&self, user_id: &str, token: &str, expires_at: &str) -> AppResult<()> {
        refresh_token::create_token(&self.pool, user_id, token, expires_at).await
    }

    async fn find_by_token(&self, token: &str) -> AppResult<Option<RefreshToken>> {
        refresh_token::find_by_token(&self.pool, token).await
    }

    async fn delete_by_token(&self, token: &str) -> AppResult<()> {
        refresh_token::delete_by_token(&self.pool, token).await
    }

    async fn delete_by_user(&self, user_id: &str) -> AppResult<()> {
        refresh_token::delete_by_user(&self.pool, user_id).await
    }
}
