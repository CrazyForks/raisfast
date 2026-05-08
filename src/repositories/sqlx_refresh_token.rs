//! 基于 sqlx 的 `RefreshTokenRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::refresh_token::{self, RefreshToken};

use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxRefreshTokenRepository);

/// 刷新令牌 Repository 接口
#[async_trait::async_trait]
pub trait RefreshTokenRepository: Send + Sync {
    /// 创建新的刷新令牌记录
    async fn create_token(&self, user_id: i64, token: &str, expires_at: &str) -> AppResult<()>;

    /// 根据令牌字符串查找刷新令牌
    async fn find_by_token(&self, token: &str) -> AppResult<Option<RefreshToken>>;

    /// 根据令牌字符串删除刷新令牌
    async fn delete_by_token(&self, token: &str) -> AppResult<()>;

    /// 删除指定用户的所有刷新令牌
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
