//! 基于 sqlx 的 UserRepository 实现

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::user::{self, User};

use super::UserRepository;
use crate::commands::{CreateUserCmd, UpdateProfileCmd};

/// 基于 sqlx 的用户 Repository
pub struct SqlxUserRepository {
    pool: Pool,
}

impl SqlxUserRepository {
    /// 创建新的 SqlxUserRepository
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl UserRepository for SqlxUserRepository {
    async fn find_by_email(&self, email: &str) -> AppResult<Option<User>> {
        user::find_by_email(&self.pool, email).await
    }

    async fn find_by_id(&self, id: &str) -> AppResult<Option<User>> {
        user::find_by_id(&self.pool, id).await
    }

    async fn create(&self, cmd: CreateUserCmd) -> AppResult<User> {
        user::create(&self.pool, &cmd).await
    }

    async fn update_profile(&self, cmd: UpdateProfileCmd) -> AppResult<User> {
        user::update_profile(&self.pool, &cmd).await
    }

    async fn update_password(&self, id: &str, new_password_hash: &str) -> AppResult<()> {
        user::update_password(&self.pool, id, new_password_hash).await
    }

    async fn find_all(&self, page: i64, page_size: i64) -> AppResult<(Vec<User>, i64)> {
        user::find_all(&self.pool, page, page_size).await
    }

    async fn update_role(&self, id: &str, role: &str) -> AppResult<User> {
        user::update_role(&self.pool, id, role).await
    }
}
