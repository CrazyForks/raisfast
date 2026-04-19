//! Repository 抽象层。
//!
//! 将数据访问从 Model 层解耦。每个领域实体对应一个 Repository trait：
//!
//! - `PostRepository` / `SqlxPostRepository` / `CachedPostRepository`
//! - `UserRepository` / `SqlxUserRepository`
//! - `CategoryRepository` / `SqlxCategoryRepository`
//! - `TagRepository` / `SqlxTagRepository`
//! - `CommentRepository` / `SqlxCommentRepository`
//! - `MediaRepository` / `SqlxMediaRepository`
//! - `RefreshTokenRepository` / `SqlxRefreshTokenRepository`
//! - `OptionsRepository` / `SqlxOptionsRepository`
//! - `RbacRepository` / `SqlxRbacRepository`

pub mod cached_post;
pub mod sqlx_category;
pub mod sqlx_comment;
pub mod sqlx_media;
pub mod sqlx_options;
pub mod sqlx_post;
pub mod sqlx_rbac;
pub mod sqlx_refresh_token;
pub mod sqlx_tag;
pub mod sqlx_tenant;
pub mod sqlx_user;

use std::collections::HashMap;

use crate::errors::app_error::AppResult;
use crate::models::category::Category;
use crate::models::comment::{AdminCommentRow, Comment};
use crate::models::media::{Media, MediaStats};
use crate::models::post::{Post, PostJoinedRow, TagBrief};
use crate::models::rbac::{Permission, Role};
use crate::models::refresh_token::RefreshToken;
use crate::models::tag::Tag;
use crate::models::tenant::Tenant;
use crate::models::user::User;

pub use crate::commands::*;
pub use cached_post::CachedPostRepository;
pub use sqlx_category::SqlxCategoryRepository;
pub use sqlx_comment::SqlxCommentRepository;
pub use sqlx_media::SqlxMediaRepository;
pub use sqlx_options::SqlxOptionsRepository;
pub use sqlx_post::SqlxPostRepository;
pub use sqlx_rbac::SqlxRbacRepository;
pub use sqlx_refresh_token::SqlxRefreshTokenRepository;
pub use sqlx_tag::SqlxTagRepository;
pub use sqlx_tenant::SqlxTenantRepository;
pub use sqlx_user::SqlxUserRepository;

/// 定义 sqlx Repository 的 struct 和 `new()` 构造函数。
///
/// 每个 `Sqlx*Repository` 都有相同的 `pool: Pool` 字段和 `new(pool) -> Self` 构造函数，
/// 用此宏消除样板代码。
macro_rules! define_sqlx_repo {
    ($name:ident) => {
        pub struct $name {
            pool: $crate::db::Pool,
        }

        impl $name {
            #[must_use]
            pub fn new(pool: $crate::db::Pool) -> Self {
                Self { pool }
            }
        }
    };
}

pub(crate) use define_sqlx_repo;

/// 文章 Repository 接口
#[async_trait::async_trait]
pub trait PostRepository: Send + Sync {
    /// 根据 slug 查找文章
    async fn find_by_slug(&self, slug: &str, tenant_id: Option<&str>) -> AppResult<Option<Post>>;

    /// 根据 ID 查找文章
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<Post>>;

    /// 根据 ID 查找文章（JOIN 作者名和分类名）
    async fn find_joined_by_id(
        &self,
        id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow>;

    /// 分页查询已发布文章（JOIN 作者名和分类名）
    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)>;

    /// 查询全部文章（含所有状态），用于后台管理
    async fn find_all_joined(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)>;

    /// 原子性增加浏览量并返回 JOIN 查询结果
    async fn increment_view_count_joined(
        &self,
        slug: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<PostJoinedRow>;

    /// 获取单篇文章的标签
    async fn get_post_tags(
        &self,
        post_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<TagBrief>>;

    /// 批量获取多篇文章的标签
    async fn get_tags_for_posts(
        &self,
        post_ids: &[String],
        tenant_id: Option<&str>,
    ) -> AppResult<HashMap<String, Vec<TagBrief>>>;

    /// 根据 ID 列表批量查询已发布文章（JOIN 作者名和分类名）
    async fn find_joined_by_ids(
        &self,
        ids: &[String],
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<PostJoinedRow>>;

    /// 创建文章，根据 `tag_ids` 是否为 Some 决定是否同步标签
    async fn create(&self, cmd: CreatePostCmd, tenant_id: Option<&str>) -> AppResult<Post>;

    /// 更新文章，根据 `tag_ids` 是否为 Some 决定是否同步标签
    async fn update(&self, cmd: UpdatePostCmd, tenant_id: Option<&str>) -> AppResult<Post>;

    /// 删除文章
    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()>;
}

/// 用户 Repository 接口
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    /// 根据邮箱查找用户
    async fn find_by_email(&self, email: &str, tenant_id: Option<&str>) -> AppResult<Option<User>>;

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

    /// 更新用户密码
    async fn update_password(
        &self,
        id: &str,
        new_password_hash: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<()>;

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

/// 分类 Repository 接口
#[async_trait::async_trait]
pub trait CategoryRepository: Send + Sync {
    /// 查询所有分类
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Category>>;

    /// 分页查询分类
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Category>, i64)>;

    /// 根据 ID 查找分类
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Category>;

    /// 创建新分类
    async fn create(&self, cmd: CreateCategoryCmd, tenant_id: Option<&str>) -> AppResult<Category>;

    /// 更新分类
    async fn update(&self, cmd: UpdateCategoryCmd, tenant_id: Option<&str>) -> AppResult<Category>;

    /// 删除分类
    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()>;
}

/// 标签 Repository 接口
#[async_trait::async_trait]
pub trait TagRepository: Send + Sync {
    /// 查询所有标签
    async fn find_all(&self, tenant_id: Option<&str>) -> AppResult<Vec<Tag>>;

    /// 分页查询标签
    async fn find_paginated(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Tag>, i64)>;

    /// 创建新标签
    async fn create(&self, name: &str, slug: &str, tenant_id: Option<&str>) -> AppResult<Tag>;

    /// 删除标签
    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()>;
}

/// 评论 Repository 接口
#[async_trait::async_trait]
pub trait CommentRepository: Send + Sync {
    /// 根据评论 ID 查找评论
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<Comment>>;

    /// 创建新评论
    async fn create(&self, cmd: CreateCommentCmd, tenant_id: Option<&str>) -> AppResult<Comment>;

    /// 查询指定文章下已审核通过的评论
    async fn find_approved_by_post(
        &self,
        post_id: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<Comment>>;

    /// 分页查询指定文章下已审核通过的评论
    async fn find_approved_by_post_paginated(
        &self,
        post_id: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Comment>, i64)>;

    /// 分页查询全局所有评论（管理员）
    async fn find_all_paginated(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)>;

    /// 更新评论审核状态
    async fn update_status(&self, id: &str, status: &str, tenant_id: Option<&str>)
    -> AppResult<()>;

    /// 删除评论
    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()>;
}

/// 媒体文件 Repository 接口
#[async_trait::async_trait]
pub trait MediaRepository: Send + Sync {
    /// 创建媒体文件记录
    async fn create(&self, cmd: CreateMediaCmd, tenant_id: Option<&str>) -> AppResult<Media>;

    /// 分页查询指定用户的媒体文件
    async fn find_all(
        &self,
        user_id: &str,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)>;

    /// 根据媒体文件 ID 查找
    async fn find_by_id(&self, id: &str, tenant_id: Option<&str>) -> AppResult<Option<Media>>;

    /// 删除媒体文件记录
    async fn delete(&self, id: &str, tenant_id: Option<&str>) -> AppResult<()>;

    /// 获取存储统计
    async fn stats(&self, user_id: &str, tenant_id: Option<&str>) -> AppResult<MediaStats>;
}

/// 刷新令牌 Repository 接口
#[async_trait::async_trait]
pub trait RefreshTokenRepository: Send + Sync {
    /// 创建新的刷新令牌记录
    async fn create_token(&self, user_id: &str, token: &str, expires_at: &str) -> AppResult<()>;

    /// 根据令牌字符串查找刷新令牌
    async fn find_by_token(&self, token: &str) -> AppResult<Option<RefreshToken>>;

    /// 根据令牌字符串删除刷新令牌
    async fn delete_by_token(&self, token: &str) -> AppResult<()>;

    /// 删除指定用户的所有刷新令牌
    async fn delete_by_user(&self, user_id: &str) -> AppResult<()>;
}

/// 站点配置 Repository 接口
#[async_trait::async_trait]
pub trait OptionsRepository: Send + Sync {
    /// 查询所有 autoload 配置（含元数据）
    async fn find_autoload(&self) -> AppResult<Vec<crate::models::options::OptionRow>>;

    /// 根据 key 查询单条配置（含元数据）
    async fn find_by_key(
        &self,
        key: &str,
        tenant_id: &str,
    ) -> AppResult<Option<crate::models::options::OptionRow>>;

    /// 查询所有配置（含元数据）
    async fn find_all(&self, tenant_id: &str) -> AppResult<Vec<crate::models::options::OptionRow>>;

    /// 更新配置值
    async fn upsert_value(
        &self,
        key: &str,
        value: &str,
        tenant_id: &str,
        updated_at: &str,
    ) -> AppResult<()>;

    /// 根据 key 删除配置
    async fn delete_by_key(&self, key: &str, tenant_id: &str) -> AppResult<()>;
}

/// RBAC Repository 接口
#[async_trait::async_trait]
pub trait RbacRepository: Send + Sync {
    /// 查询所有角色
    async fn list_roles(&self) -> AppResult<Vec<Role>>;

    /// 根据 ID 查找角色
    async fn find_role_by_id(&self, id: &str) -> AppResult<Option<Role>>;

    /// 根据角色名查找角色 ID
    async fn find_role_id_by_name(&self, name: &str) -> AppResult<Option<String>>;

    /// 创建角色
    async fn create_role(
        &self,
        id: &str,
        name: &str,
        description: Option<&str>,
        created_at: &str,
    ) -> AppResult<Role>;

    /// 更新角色
    async fn update_role(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        updated_at: &str,
    ) -> AppResult<Role>;

    /// 删除角色
    async fn delete_role(&self, id: &str) -> AppResult<()>;

    /// 查询角色的所有权限
    async fn find_permissions_by_role_id(&self, role_id: &str) -> AppResult<Vec<Permission>>;

    /// 删除角色的所有权限
    async fn delete_permissions_by_role_id(&self, role_id: &str) -> AppResult<()>;

    /// 插入单条权限
    #[allow(clippy::too_many_arguments)]
    async fn insert_permission(
        &self,
        id: &str,
        role_id: &str,
        action: &str,
        subject: &str,
        fields: Option<&str>,
        conditions: Option<&str>,
        created_at: &str,
    ) -> AppResult<()>;
}

/// 租户 Repository 接口
#[async_trait::async_trait]
pub trait TenantRepository: Send + Sync {
    /// 查询所有租户
    async fn find_all(&self) -> AppResult<Vec<Tenant>>;

    /// 根据 ID 查找租户
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Tenant>>;

    /// 根据域名查找租户
    async fn find_by_domain(&self, domain: &str) -> AppResult<Option<Tenant>>;

    /// 创建租户
    async fn create(
        &self,
        id: &str,
        name: &str,
        domain: Option<&str>,
        config: &str,
        created_at: &str,
    ) -> AppResult<Tenant>;

    /// 更新租户
    #[allow(clippy::too_many_arguments)]
    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<&str>,
        updated_at: &str,
    ) -> AppResult<Tenant>;

    /// 删除租户
    async fn delete(&self, id: &str) -> AppResult<()>;
}
