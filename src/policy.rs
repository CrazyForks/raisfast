//! Resource-level authorization policies
//!
//! Complements the global RBAC [`PermissionGuard`](crate::middleware::permission::PermissionGuard)
//! with per-resource ownership rules. Whereas RBAC checks "can *role* do *action* on *subject*",
//! Policy checks "can *this user* touch *this specific resource*".

use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;

/// Trait for resource-level authorization.
///
/// Implement for each resource type that needs ownership-based access control.
/// Default `can_view` allows all access; override for visibility rules.
pub trait Policy {
    type Resource;

    fn can_view(_user: &AuthUser, _resource: &Self::Resource) -> AppResult<()> {
        Ok(())
    }

    fn can_update(user: &AuthUser, resource: &Self::Resource) -> AppResult<()>;

    fn can_delete(user: &AuthUser, resource: &Self::Resource) -> AppResult<()>;
}

fn owner_or_admin(user: &AuthUser, owner_id: SnowflakeId) -> AppResult<()> {
    let uid = user.user_id().ok_or(AppError::Unauthorized)?;
    if user.is_admin() || SnowflakeId(uid) == owner_id {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn owner_or_admin_opt(
    user: &AuthUser,
    owner_id: Option<crate::types::snowflake_id::SnowflakeId>,
) -> AppResult<()> {
    let uid = user.user_id().ok_or(AppError::Unauthorized)?;
    if user.is_admin() || owner_id == Some(crate::types::snowflake_id::SnowflakeId(uid)) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// Post policy: only the author or an admin may update or delete.
pub struct PostPolicy;

impl Policy for PostPolicy {
    type Resource = crate::models::post::Post;

    fn can_update(user: &AuthUser, post: &Self::Resource) -> AppResult<()> {
        owner_or_admin(user, post.created_by)
    }

    fn can_delete(user: &AuthUser, post: &Self::Resource) -> AppResult<()> {
        owner_or_admin(user, post.created_by)
    }
}

/// Comment policy: only the author or an admin may update or delete.
pub struct CommentPolicy;

impl Policy for CommentPolicy {
    type Resource = crate::models::comment::Comment;

    fn can_update(user: &AuthUser, comment: &Self::Resource) -> AppResult<()> {
        owner_or_admin_opt(user, comment.created_by)
    }

    fn can_delete(user: &AuthUser, comment: &Self::Resource) -> AppResult<()> {
        owner_or_admin_opt(user, comment.created_by)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserRole;

    fn admin() -> AuthUser {
        AuthUser::from_parts(Some(1), UserRole::Admin, Some("t".into()))
    }

    fn author(uid: i64) -> AuthUser {
        AuthUser::from_parts(Some(uid), UserRole::Author, Some("t".into()))
    }

    fn anon() -> AuthUser {
        AuthUser::from_parts(None, UserRole::Reader, Some("t".into()))
    }

    fn mock_post(created_by: i64) -> crate::models::post::Post {
        use crate::types::snowflake_id::SnowflakeId;
        use crate::utils::tz::Timestamp;
        crate::models::post::Post {
            id: SnowflakeId(1),
            tenant_id: None,
            title: String::new(),
            slug: String::new(),
            content: String::new(),
            excerpt: None,
            cover_image: None,
            image_ids: None,
            status: crate::models::post::PostStatus::Draft,
            created_by: SnowflakeId(created_by),
            updated_by: None,
            category_id: None,
            view_count: 0,
            is_pinned: false,
            password: None,
            comment_status: crate::models::post::CommentOpenStatus::Open,
            format: String::new(),
            template: String::new(),
            meta_title: None,
            meta_description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            reading_time: 0,
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
            published_at: None,
        }
    }

    fn mock_comment(created_by: Option<i64>) -> crate::models::comment::Comment {
        use crate::types::snowflake_id::SnowflakeId;
        use crate::utils::tz::Timestamp;
        crate::models::comment::Comment {
            id: SnowflakeId(1),
            tenant_id: None,
            post_id: SnowflakeId(1),
            created_by: created_by.map(SnowflakeId),
            updated_by: None,
            nickname: None,
            email: None,
            content: "hi".into(),
            parent_id: None,
            author_ip: None,
            author_url: None,
            status: crate::models::comment::CommentStatus::Approved,
            created_at: Timestamp::default(),
            updated_at: Timestamp::default(),
        }
    }

    #[test]
    fn post_policy_allows_owner() {
        assert!(PostPolicy::can_update(&author(10), &mock_post(10)).is_ok());
        assert!(PostPolicy::can_delete(&author(10), &mock_post(10)).is_ok());
    }

    #[test]
    fn post_policy_allows_admin() {
        assert!(PostPolicy::can_update(&admin(), &mock_post(99)).is_ok());
        assert!(PostPolicy::can_delete(&admin(), &mock_post(99)).is_ok());
    }

    #[test]
    fn post_policy_rejects_other_author() {
        assert!(PostPolicy::can_update(&author(1), &mock_post(2)).is_err());
        assert!(PostPolicy::can_delete(&author(1), &mock_post(2)).is_err());
    }

    #[test]
    fn post_policy_rejects_anon() {
        assert!(PostPolicy::can_update(&anon(), &mock_post(1)).is_err());
    }

    #[test]
    fn comment_policy_allows_owner() {
        assert!(CommentPolicy::can_delete(&author(5), &mock_comment(Some(5))).is_ok());
    }

    #[test]
    fn comment_policy_allows_admin_with_none_owner() {
        assert!(CommentPolicy::can_delete(&admin(), &mock_comment(None)).is_ok());
    }

    #[test]
    fn comment_policy_rejects_other() {
        assert!(CommentPolicy::can_delete(&author(1), &mock_comment(Some(2))).is_err());
    }

    #[test]
    fn default_view_allows_all() {
        assert!(PostPolicy::can_view(&anon(), &mock_post(99)).is_ok());
    }
}
