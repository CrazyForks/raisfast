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

pub use crate::commands::*;
pub use cached_post::CachedPostRepository;
pub use sqlx_category::{CategoryRepository, SqlxCategoryRepository};
pub use sqlx_comment::{CommentRepository, SqlxCommentRepository};
pub use sqlx_media::{MediaRepository, SqlxMediaRepository};
pub use sqlx_options::{OptionsRepository, SqlxOptionsRepository};
pub use sqlx_post::{PostRepository, SqlxPostRepository};
pub use sqlx_rbac::{RbacRepository, SqlxRbacRepository};
pub use sqlx_refresh_token::{RefreshTokenRepository, SqlxRefreshTokenRepository};
pub use sqlx_tag::{SqlxTagRepository, TagRepository};
pub use sqlx_tenant::{SqlxTenantRepository, TenantRepository};
pub use sqlx_user::{SqlxUserRepository, UserRepository};

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
