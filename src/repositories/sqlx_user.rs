//! 基于 sqlx 的 `UserRepository` 实现

use crate::errors::app_error::AppResult;
use crate::models::user::{self, User};

use crate::commands::{CreateUserCmd, UpdateProfileCmd};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxUserRepository);

/// 用户 Repository 接口
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    /// 根据 ID 查找用户
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<User>>;

    /// 创建新用户
    async fn create(&self, cmd: CreateUserCmd, tenant_id: Option<&str>) -> AppResult<User>;

    /// 更新用户资料
    async fn update_profile(
        &self,
        cmd: UpdateProfileCmd,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;

    /// 分页查询所有用户
    async fn find_all(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<User>, i64)>;

    /// 管理员更新用户角色
    async fn update_role(&self, id: &str, role: &str, tenant_id: Option<&str>) -> AppResult<User>;
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

    async fn update_role(&self, id: &str, role: &str, tenant_id: Option<&str>) -> AppResult<User> {
        user::update_role(&self.pool, id, role, tenant_id).await
    }
}
