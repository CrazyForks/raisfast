//! sqlx-based `UserRepository` implementation

use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::errors::app_error::AppResult;
use crate::models::user::{User, UserRole};
use raisfast_derive::repository;

#[repository(model = "user", struct_name = SqlxUserRepository)]
pub trait UserRepository: Send + Sync {
    /// Find a user by ID
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<User>>;

    /// Create a new user
    async fn create(&self, cmd: &CreateUserCmd, tenant_id: Option<&str>) -> AppResult<User>;

    /// Update user profile
    async fn update_profile(
        &self,
        cmd: &UpdateProfileCmd,
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
        role: UserRole,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;
}
