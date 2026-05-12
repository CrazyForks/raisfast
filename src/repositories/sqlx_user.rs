//! sqlx-based `UserRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::user::{self, User};

use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxUserRepository);

/// User Repository interface
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    /// Find a user by ID
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<User>>;

    /// Create a new user
    async fn create(&self, cmd: CreateUserCmd, tenant_id: Option<&str>) -> AppResult<User>;

    /// Update user profile
    async fn update_profile(
        &self,
        cmd: UpdateProfileCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;

    /// Find all users with pagination
    async fn find_all(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<User>, i64)>;

    /// Admin update user role
    async fn update_role(
        &self,
        id: &str,
        role: crate::models::user::UserRole,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;
}

#[async_trait::async_trait]
impl UserRepository for SqlxUserRepository {
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

    async fn find_all(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<User>, i64)> {
        user::find_all(&self.pool, page, page_size, tenant_id).await
    }

    async fn update_role(
        &self,
        id: &str,
        role: crate::models::user::UserRole,
        tenant_id: Option<&str>,
    ) -> AppResult<User> {
        user::update_role(&self.pool, id, role, tenant_id).await
    }
}
