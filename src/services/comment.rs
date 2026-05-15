//! Comment service.
//!
//! Handles comment-related business logic, including comment creation (with nesting depth validation),
//! comment listing (tree structure), comment deletion, and status management.

use crate::commands::CreateCommentCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::eventbus::EventBus;
use crate::middleware::auth::AuthUser;
use crate::models::comment::{self, CommentResponse, CommentStatus};
use crate::plugins::{HookPoint, PluginManager};
use crate::policy::Policy;
use crate::repositories::{CommentRepository, PostRepository};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommentInput {
    pub content: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub parent_id: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_comment(
    post_repo: &dyn PostRepository,
    comment_repo: &dyn CommentRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    post_slug: &str,
    auth: &AuthUser,
    content: &str,
    parent_id: Option<&str>,
    nickname: Option<&str>,
    email: Option<&str>,
) -> AppResult<CommentResponse> {
    let p = post_repo
        .find_by_slug(post_slug, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    if let Some(pid_str) = parent_id {
        let pid: i64 = pid_str
            .parse()
            .map_err(|_| AppError::BadRequest("invalid parent_id".into()))?;
        let all_comments = comment_repo
            .find_approved_by_post(p.id, auth.tenant_id())
            .await?;
        let parent = all_comments
            .iter()
            .find(|c| c.id == pid)
            .ok_or_else(|| AppError::not_found("parent_comment"))?;

        if parent.post_id != p.id {
            return Err(AppError::BadRequest("parent_comment_mismatch".into()));
        }

        comment::validate_depth(&all_comments, pid)?;
    }

    let comment_input = CommentInput {
        content: content.to_string(),
        nickname: nickname.map(std::string::ToString::to_string),
        email: email.map(std::string::ToString::to_string),
        parent_id: parent_id.map(std::string::ToString::to_string),
    };

    let filtered = plugins
        .dispatch_filter(HookPoint::CommentCreating, comment_input)
        .await?;

    let parent_id = if let Some(ref doc_id) = filtered.parent_id {
        if doc_id.is_empty() {
            None
        } else if let Ok(int_id) = doc_id.parse::<i64>() {
            Some(int_id)
        } else {
            comment_repo
                .find_by_document_id(doc_id, auth.tenant_id())
                .await?
                .map(|c| c.id)
        }
    } else {
        None
    };

    let c = comment_repo
        .create(
            CreateCommentCmd {
                post_id: p.id,
                created_by: auth.user_int_id(),
                nickname: filtered.nickname,
                email: filtered.email,
                content: filtered.content,
                parent_id,
            },
            auth.tenant_id(),
        )
        .await?;

    eventbus.emit(Event::CommentCreated(c.clone()));

    Ok(CommentResponse {
        id: c.id,
        document_id: c.document_id.clone(),
        post_id: c.post_id,
        created_by: c.created_by,
        nickname: c.nickname,
        content: c.content,
        parent_id: c.parent_id,
        depth: 0,
        replies: vec![],
        created_at: c.created_at,
    })
}

pub async fn list_comments_paginated(
    post_repo: &dyn PostRepository,
    comment_repo: &dyn CommentRepository,
    post_slug: &str,
    page: i64,
    page_size: i64,
    auth: &AuthUser,
) -> AppResult<(Vec<CommentResponse>, i64)> {
    let p = post_repo
        .find_by_slug(post_slug, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    let (comments, total) = comment_repo
        .find_approved_by_post_paginated(p.id, page, page_size, auth.tenant_id())
        .await?;
    Ok((comment::build_tree(&comments), total))
}

pub async fn delete_comment(
    comment_repo: &dyn CommentRepository,
    comment_id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let c = comment_repo
        .find_by_document_id(comment_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("comment"))?;

    crate::policy::CommentPolicy::can_delete(auth, &c)?;

    comment_repo.delete(c.id, auth.tenant_id()).await?;
    Ok(())
}

pub async fn update_comment_status(
    comment_repo: &dyn CommentRepository,
    comment_id: &str,
    status: CommentStatus,
    auth: &AuthUser,
) -> AppResult<()> {
    let c = comment_repo
        .find_by_document_id(comment_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("comment"))?;
    comment_repo
        .update_status(c.id, status, auth.tenant_id())
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreateCommentCmd;
    use crate::repositories::sqlx_comment::SqlxCommentRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn auth(user: &crate::models::user::User) -> AuthUser {
        AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            crate::models::user::UserRole::Admin,
            None,
        )
    }

    async fn insert_user(pool: &crate::db::Pool) -> crate::models::user::User {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_document_id(),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            None,
        )
        .await
        .unwrap();
        crate::models::user::update_role(
            pool,
            &user.document_id,
            crate::models::user::UserRole::Admin,
            None,
        )
        .await
        .unwrap()
    }

    async fn insert_post(pool: &crate::db::Pool, user_id: i64) -> i64 {
        crate::models::post::create(
            pool,
            &crate::commands::CreatePostCmd {
                title: "Test".into(),
                slug: "test".into(),
                content: "body".into(),
                excerpt: None,
                cover_image: None,
                status: crate::models::post::PostStatus::Published,
                created_by: user_id,
                updated_by: None,
                category_id: None,
                tag_ids: None,
            },
            None,
        )
        .await
        .unwrap()
        .id
    }

    async fn insert_comment(
        pool: &crate::db::Pool,
        post_id: i64,
        user_id: i64,
    ) -> crate::models::comment::Comment {
        let repo = SqlxCommentRepository::new(pool.clone());
        repo.create(
            CreateCommentCmd {
                post_id,
                created_by: Some(user_id),
                nickname: None,
                email: None,
                content: "nice post".into(),
                parent_id: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn update_comment_status_valid() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let post_id = insert_post(&pool, user.id).await;
        let c = insert_comment(&pool, post_id, user.id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        super::update_comment_status(&repo, &c.document_id, CommentStatus::Approved, &auth(&user))
            .await
            .unwrap();
        let updated = repo.find_by_id(c.id, None).await.unwrap().unwrap();
        assert_eq!(
            updated.status,
            crate::models::comment::CommentStatus::Approved
        );
    }

    #[tokio::test]
    async fn update_comment_status_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxCommentRepository::new(pool.clone());
        let a = AuthUser::new_test("any", crate::models::user::UserRole::Admin, "");
        assert!(
            super::update_comment_status(&repo, "missing", CommentStatus::Approved, &a)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_comment_by_owner() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let post_id = insert_post(&pool, user.id).await;
        let c = insert_comment(&pool, post_id, user.id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        let a = AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            crate::models::user::UserRole::Admin,
            None,
        );
        super::delete_comment(&repo, &c.document_id, &a)
            .await
            .unwrap();
        assert!(repo.find_by_id(c.id, None).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_comment_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxCommentRepository::new(pool.clone());
        let a = AuthUser::new_test("any", crate::models::user::UserRole::Admin, "");
        assert!(super::delete_comment(&repo, "missing", &a).await.is_err());
    }

    #[tokio::test]
    async fn update_comment_status_spam() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let post_id = insert_post(&pool, user.id).await;
        let c = insert_comment(&pool, post_id, user.id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        super::update_comment_status(&repo, &c.document_id, CommentStatus::Spam, &auth(&user))
            .await
            .unwrap();
        let updated = repo.find_by_id(c.id, None).await.unwrap().unwrap();
        assert_eq!(updated.status, crate::models::comment::CommentStatus::Spam);
    }
}
