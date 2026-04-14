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

pub mod cached_post;
pub mod sqlx_category;
pub mod sqlx_comment;
pub mod sqlx_media;
pub mod sqlx_post;
pub mod sqlx_refresh_token;
pub mod sqlx_tag;
pub mod sqlx_user;

use std::collections::HashMap;

use crate::errors::app_error::AppResult;
use crate::models::category::Category;
use crate::models::comment::{AdminCommentRow, Comment};
use crate::models::media::Media;
use crate::models::post::{Post, PostJoinedRow, TagBrief};
use crate::models::refresh_token::RefreshToken;
use crate::models::tag::Tag;
use crate::models::user::User;

pub use crate::commands::*;
pub use cached_post::CachedPostRepository;
pub use sqlx_category::SqlxCategoryRepository;
pub use sqlx_comment::SqlxCommentRepository;
pub use sqlx_media::SqlxMediaRepository;
pub use sqlx_post::SqlxPostRepository;
pub use sqlx_refresh_token::SqlxRefreshTokenRepository;
pub use sqlx_tag::SqlxTagRepository;
pub use sqlx_user::SqlxUserRepository;

/// 文章 Repository 接口
#[async_trait::async_trait]
pub trait PostRepository: Send + Sync {
    /// 根据 slug 查找文章
    async fn find_by_slug(&self, slug: &str) -> AppResult<Option<Post>>;

    /// 根据 ID 查找文章
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Post>>;

    /// 根据 ID 查找文章（JOIN 作者名和分类名）
    async fn find_joined_by_id(&self, id: &str) -> AppResult<PostJoinedRow>;

    /// 分页查询已发布文章（JOIN 作者名和分类名）
    async fn find_published_joined(
        &self,
        query: FindPublishedQuery,
    ) -> AppResult<(Vec<PostJoinedRow>, i64)>;

    /// 原子性增加浏览量并返回 JOIN 查询结果
    async fn increment_view_count_joined(&self, slug: &str) -> AppResult<PostJoinedRow>;

    /// 获取单篇文章的标签
    async fn get_post_tags(&self, post_id: &str) -> AppResult<Vec<TagBrief>>;

    /// 批量获取多篇文章的标签
    async fn get_tags_for_posts(
        &self,
        post_ids: &[String],
    ) -> AppResult<HashMap<String, Vec<TagBrief>>>;

    /// 根据 ID 列表批量查询已发布文章（JOIN 作者名和分类名）
    async fn find_joined_by_ids(&self, ids: &[String]) -> AppResult<Vec<PostJoinedRow>>;

    /// 创建文章，根据 tag_ids 是否为 Some 决定是否同步标签
    async fn create(&self, cmd: CreatePostCmd) -> AppResult<Post>;

    /// 更新文章，根据 tag_ids 是否为 Some 决定是否同步标签
    async fn update(&self, cmd: UpdatePostCmd) -> AppResult<Post>;

    /// 删除文章
    async fn delete(&self, id: &str) -> AppResult<()>;
}

/// 用户 Repository 接口
#[async_trait::async_trait]
pub trait UserRepository: Send + Sync {
    /// 根据邮箱查找用户
    async fn find_by_email(&self, email: &str) -> AppResult<Option<User>>;

    /// 根据 ID 查找用户
    async fn find_by_id(&self, id: &str) -> AppResult<Option<User>>;

    /// 创建新用户
    async fn create(&self, cmd: CreateUserCmd) -> AppResult<User>;

    /// 更新用户资料
    async fn update_profile(&self, cmd: UpdateProfileCmd) -> AppResult<User>;

    /// 更新用户密码
    async fn update_password(&self, id: &str, new_password_hash: &str) -> AppResult<()>;

    /// 分页查询所有用户
    async fn find_all(&self, page: i64, page_size: i64) -> AppResult<(Vec<User>, i64)>;

    /// 管理员更新用户角色
    async fn update_role(&self, id: &str, role: &str) -> AppResult<User>;
}

/// 分类 Repository 接口
#[async_trait::async_trait]
pub trait CategoryRepository: Send + Sync {
    /// 查询所有分类
    async fn find_all(&self) -> AppResult<Vec<Category>>;

    /// 根据 ID 查找分类
    async fn find_by_id(&self, id: &str) -> AppResult<Category>;

    /// 创建新分类
    async fn create(&self, cmd: CreateCategoryCmd) -> AppResult<Category>;

    /// 更新分类
    async fn update(&self, cmd: UpdateCategoryCmd) -> AppResult<Category>;

    /// 删除分类
    async fn delete(&self, id: &str) -> AppResult<()>;
}

/// 标签 Repository 接口
#[async_trait::async_trait]
pub trait TagRepository: Send + Sync {
    /// 查询所有标签
    async fn find_all(&self) -> AppResult<Vec<Tag>>;

    /// 创建新标签
    async fn create(&self, name: &str, slug: &str) -> AppResult<Tag>;

    /// 删除标签
    async fn delete(&self, id: &str) -> AppResult<()>;
}

/// 评论 Repository 接口
#[async_trait::async_trait]
pub trait CommentRepository: Send + Sync {
    /// 根据评论 ID 查找评论
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Comment>>;

    /// 创建新评论
    async fn create(&self, cmd: CreateCommentCmd) -> AppResult<Comment>;

    /// 查询指定文章下已审核通过的评论
    async fn find_approved_by_post(&self, post_id: &str) -> AppResult<Vec<Comment>>;

    /// 分页查询指定文章下已审核通过的评论
    async fn find_approved_by_post_paginated(
        &self,
        post_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Comment>, i64)>;

    /// 分页查询全局所有评论（管理员）
    async fn find_all_paginated(
        &self,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)>;

    /// 更新评论审核状态
    async fn update_status(&self, id: &str, status: &str) -> AppResult<()>;

    /// 删除评论
    async fn delete(&self, id: &str) -> AppResult<()>;
}

/// 媒体文件 Repository 接口
#[async_trait::async_trait]
pub trait MediaRepository: Send + Sync {
    /// 创建媒体文件记录
    async fn create(&self, cmd: CreateMediaCmd) -> AppResult<Media>;

    /// 分页查询指定用户的媒体文件
    async fn find_all(
        &self,
        user_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Media>, i64)>;

    /// 根据媒体文件 ID 查找
    async fn find_by_id(&self, id: &str) -> AppResult<Option<Media>>;

    /// 删除媒体文件记录
    async fn delete(&self, id: &str) -> AppResult<()>;
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
