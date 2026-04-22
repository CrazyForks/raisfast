//! 基于 sqlx 的 `UserRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::user::{self, User};

use super::UserRepository;
use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxUserRepository);

#[async_trait::async_trait]
impl UserRepository for SqlxUserRepository {
    async fn find_by_email(&self, email: &str, tenant_id: Option<&str>) -> AppResult<Option<User>> {
        user::find_by_email(&self.pool, email, tenant_id).await
    }

    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<User>> {
        user::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn create(&self, cmd: CreateUserCmd, tenant_id: Option<&str>) -> AppResult<User> {
        user::create(&self.pool, &cmd, tenant_id).await
    }

    async fn update_profile(
        &self,
        cmd: UpdateProfileCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<User> {
        user::update_profile(&self.pool, &cmd, tenant_id).await
    }

    async fn update_password(
        &self,
        id: &str,
        new_password_hash: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<()> {
        user::update_password(&self.pool, id, new_password_hash, tenant_id).await
    }

    async fn find_all(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<User>, i64)> {
        user::find_all(&self.pool, page, page_size, tenant_id).await
    }

    async fn update_role(&self, id: &str, role: &str, tenant_id: Option<&str>) -> AppResult<User> {
        user::update_role(&self.pool, id, role, tenant_id).await
    }

    async fn find_by_phone(&self, phone: &str) -> AppResult<Option<User>> {
        user::find_by_phone(&self.pool, phone).await
    }

    async fn update_phone(&self, id: &str, phone: &str, tenant_id: Option<&str>) -> AppResult<()> {
        user::update_phone(&self.pool, id, phone, tenant_id).await
    }
}
