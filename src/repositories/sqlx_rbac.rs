//! 基于 sqlx 的 `RbacRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::rbac;

use super::RbacRepository;
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxRbacRepository);

#[async_trait::async_trait]
impl RbacRepository for SqlxRbacRepository {
    async fn list_roles(&self) -> AppResult<Vec<rbac::Role>> {
        rbac::list_roles(&self.pool).await
    }

    async fn find_role_by_id(&self, id: &str) -> AppResult<Option<rbac::Role>> {
        rbac::find_role_by_id(&self.pool, id).await
    }

    async fn find_role_id_by_name(&self, name: &str) -> AppResult<Option<String>> {
        rbac::find_role_id_by_name(&self.pool, name).await
    }

    async fn create_role(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        created_at: &str,
    ) -> AppResult<rbac::Role> {
        rbac::create_role(&self.pool, id, name, description, created_at).await
    }

    async fn update_role(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        updated_at: &str,
    ) -> AppResult<rbac::Role> {
        rbac::update_role(&self.pool, id, name, description, updated_at).await
    }

    async fn delete_role(&self, id: &str) -> AppResult<()> {
        rbac::delete_role(&self.pool, id).await
    }

    async fn find_permissions_by_role_id(&self, role_id: &str) -> AppResult<Vec<rbac::Permission>> {
        rbac::find_permissions_by_role_id(&self.pool, role_id).await
    }

    async fn delete_permissions_by_role_id(&self, role_id: &str) -> AppResult<()> {
        rbac::delete_permissions_by_role_id(&self.pool, role_id).await
    }

    async fn insert_permission(
        &self,
        id: &str,
        role_id: &str,
        action: &str,
        subject: &str,
        fields: Option<&str>,
        conditions: Option<&str>,
        created_at: &str,
    ) -> AppResult<()> {
        rbac::insert_permission(
            &self.pool, id, role_id, action, subject, fields, conditions, created_at,
        )
        .await
    }
}
