//! sqlx-based `RbacRepository` implementation

use crate::commands::CreatePermissionCmd;
use crate::errors::app_error::AppResult;
use crate::models::rbac::{Permission, Role};
use raisfast_derive::repository;

#[repository(model = "rbac", struct_name = SqlxRbacRepository)]
pub trait RbacRepository: Send + Sync {
    async fn list_roles(&self) -> AppResult<Vec<Role>>;

    async fn find_role_by_id(&self, id: &str) -> AppResult<Option<Role>>;

    async fn find_role_id_by_name(&self, name: &str) -> AppResult<Option<i64>>;

    async fn create_role(&self, id: &str, name: &str, description: Option<&str>)
    -> AppResult<Role>;

    async fn update_role(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
    ) -> AppResult<Role>;

    async fn delete_role(&self, id: &str) -> AppResult<()>;

    async fn find_permissions_by_role_id(&self, role_id: i64) -> AppResult<Vec<Permission>>;

    async fn delete_permissions_by_role_id(&self, role_id: i64) -> AppResult<()>;

    async fn insert_permission(&self, cmd: &CreatePermissionCmd) -> AppResult<()>;
}
