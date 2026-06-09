//! Comment service.
//!
//! Handles comment-related business logic, including comment creation (with nesting depth validation),
//! comment listing (tree structure), comment deletion, and status management.

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::commands::CreateCommentCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::comment::{self, AdminCommentRow, CommentResponse, CommentStatus};
use crate::policy::Policy;
use crate::types::snowflake_id::SnowflakeId;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommentInput {
    pub content: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub parent_id: Option<String>,
}

/// Comment business logic trait.
#[async_trait]
pub trait CommentService: Send + Sync {
    async fn create(
        &self,
        post_slug: &str,
        auth: &AuthUser,
        content: &str,
        parent_id: Option<&str>,
        nickname: Option<&str>,
        email: Option<&str>,
    ) -> AppResult<CommentResponse>;
    async fn list_paginated(
        &self,
        post_slug: &str,
        page: i64,
        page_size: i64,
        auth: &AuthUser,
    ) -> AppResult<(Vec<CommentResponse>, i64)>;
    async fn delete(&self, comment_id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn update_status(
        &self,
        comment_id: SnowflakeId,
        status: CommentStatus,
        auth: &AuthUser,
    ) -> AppResult<()>;
    async fn list_all_paginated(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)>;
}

pub struct CommentServiceImpl {
    pool: Arc<crate::db::Pool>,
    aspect_engine: Arc<AspectEngine>,
}

impl CommentServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>, aspect_engine: Arc<AspectEngine>) -> Self {
        Self {
            pool,
            aspect_engine,
        }
    }

    async fn before_delete(
        &self,
        auth: &AuthUser,
        existing: &crate::models::comment::Comment,
    ) -> AppResult<crate::aspects::Dispatched> {
        crate::policy::CommentPolicy::can_delete(auth, existing)?;
        self.aspect_engine
            .before_delete("comments", auth, existing)
            .await
    }

    fn after_updated(&self, c: &crate::models::comment::Comment) {
        self.aspect_engine.emit(Event::CommentUpdated(c.clone()));
    }

    fn after_deleted(&self, c: &crate::models::comment::Comment) {
        self.aspect_engine.emit(Event::CommentDeleted(c.clone()));
    }
}

#[async_trait]
impl CommentService for CommentServiceImpl {
    async fn create(
        &self,
        post_slug: &str,
        auth: &AuthUser,
        content: &str,
        parent_id: Option<&str>,
        nickname: Option<&str>,
        email: Option<&str>,
    ) -> AppResult<CommentResponse> {
        let p = crate::models::post::find_by_slug(&self.pool, post_slug, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        if let Some(pid_str) = parent_id {
            let pid = crate::types::snowflake_id::parse_id(pid_str)?;
            let all_comments =
                comment::find_approved_by_post(&self.pool, p.id, auth.tenant_id()).await?;
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

        let (filtered, _d) = self
            .aspect_engine
            .before_create("comments", auth, comment_input)
            .await?;

        let parent_id = if let Some(ref raw_id) = filtered.parent_id {
            if raw_id.is_empty() {
                None
            } else if let Ok(int_id) = raw_id.parse::<i64>() {
                Some(int_id)
            } else {
                let cid = crate::types::snowflake_id::parse_id(raw_id)?;
                comment::find_by_id(&self.pool, cid, auth.tenant_id())
                    .await?
                    .map(|c| *c.id)
            }
        } else {
            None
        };

        let c = comment::create(
            &self.pool,
            &CreateCommentCmd {
                post_id: p.id,
                created_by: auth.user_id(),
                nickname: filtered.nickname,
                email: filtered.email,
                content: filtered.content,
                parent_id,
            },
            auth.tenant_id(),
        )
        .await?;

        self.aspect_engine.emit(Event::CommentCreated(c.clone()));

        Ok(CommentResponse {
            id: c.id.to_string(),
            nickname: c.nickname,
            content: c.content,
            depth: 0,
            replies: vec![],
            created_at: c.created_at,
            post_id: None,
            created_by: None,
            parent_id: None,
        })
    }

    async fn list_paginated(
        &self,
        post_slug: &str,
        page: i64,
        page_size: i64,
        auth: &AuthUser,
    ) -> AppResult<(Vec<CommentResponse>, i64)> {
        let p = crate::models::post::find_by_slug(&self.pool, post_slug, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("post"))?;

        let (comments, total) = comment::find_approved_by_post_paginated(
            &self.pool,
            p.id,
            page,
            page_size,
            auth.tenant_id(),
        )
        .await?;
        Ok((comment::build_tree(&comments), total))
    }

    async fn delete(&self, comment_id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let c = comment::find_by_id(&self.pool, comment_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("comment"))?;

        self.before_delete(auth, &c).await?;
        comment::delete(&self.pool, c.id, auth.tenant_id()).await?;
        self.after_deleted(&c);
        Ok(())
    }

    async fn update_status(
        &self,
        comment_id: SnowflakeId,
        status: CommentStatus,
        auth: &AuthUser,
    ) -> AppResult<()> {
        let c = comment::find_by_id(&self.pool, comment_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("comment"))?;
        self.aspect_engine
            .before_update("comments", auth, &c, status)
            .await?;
        comment::update_status(&self.pool, c.id, status, auth.tenant_id()).await?;
        self.after_updated(&c);
        Ok(())
    }

    async fn list_all_paginated(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<AdminCommentRow>, i64)> {
        comment::find_all_paginated(&self.pool, page, page_size, tenant_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreateCommentCmd;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn auth(user: &crate::models::user::User) -> AuthUser {
        AuthUser::from_parts(Some(*user.id), crate::models::user::UserRole::Admin, None)
    }

    async fn insert_user(pool: &crate::db::Pool) -> crate::models::user::User {
        let user = crate::models::user::create(
            pool,
            &crate::commands::user::CreateUserCmd {
                username: crate::utils::id::new_id().to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
                role: None,
            },
            None,
        )
        .await
        .unwrap();
        crate::models::user::update_role(pool, user.id, crate::models::user::UserRole::Admin, None)
            .await
            .unwrap()
    }

    async fn insert_post(pool: &crate::db::Pool, user_id: i64) -> i64 {
        *crate::models::post::create(
            pool,
            &crate::commands::CreatePostCmd {
                title: "Test".into(),
                slug: "test".into(),
                content: "body".into(),
                excerpt: None,
                cover_image: None,
                image_ids: None,
                status: crate::models::post::PostStatus::Published,
                created_by: user_id,
                updated_by: None,
                category_id: None,
                tag_ids: None,
                meta_title: None,
                meta_description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
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
        crate::models::comment::create(
            pool,
            &CreateCommentCmd {
                post_id: SnowflakeId(post_id),
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
        let post_id = insert_post(&pool, *user.id).await;
        let c = insert_comment(&pool, post_id, *user.id).await;
        let svc = CommentServiceImpl::new(Arc::new(pool.clone()), Arc::new(AspectEngine::new()));
        svc.update_status(c.id, CommentStatus::Approved, &auth(&user))
            .await
            .unwrap();
        let updated = crate::models::comment::find_by_id(&pool, c.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            updated.status,
            crate::models::comment::CommentStatus::Approved
        );
    }

    #[tokio::test]
    async fn update_comment_status_not_found() {
        let pool = setup_pool().await;
        let svc = CommentServiceImpl::new(Arc::new(pool.clone()), Arc::new(AspectEngine::new()));
        let a = AuthUser::new_test(0, crate::models::user::UserRole::Admin, "");
        assert!(
            svc.update_status(SnowflakeId(999999), CommentStatus::Approved, &a)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_comment_by_owner() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let post_id = insert_post(&pool, *user.id).await;
        let c = insert_comment(&pool, post_id, *user.id).await;
        let svc = CommentServiceImpl::new(Arc::new(pool.clone()), Arc::new(AspectEngine::new()));
        let a = AuthUser::from_parts(Some(*user.id), crate::models::user::UserRole::Admin, None);
        svc.delete(c.id, &a).await.unwrap();
        assert!(
            crate::models::comment::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_comment_not_found() {
        let pool = setup_pool().await;
        let svc = CommentServiceImpl::new(Arc::new(pool.clone()), Arc::new(AspectEngine::new()));
        let a = AuthUser::new_test(0, crate::models::user::UserRole::Admin, "");
        assert!(svc.delete(SnowflakeId(999999), &a).await.is_err());
    }

    #[tokio::test]
    async fn update_comment_status_spam() {
        let pool = setup_pool().await;
        let user = insert_user(&pool).await;
        let post_id = insert_post(&pool, *user.id).await;
        let c = insert_comment(&pool, post_id, *user.id).await;
        let svc = CommentServiceImpl::new(Arc::new(pool.clone()), Arc::new(AspectEngine::new()));
        svc.update_status(c.id, CommentStatus::Spam, &auth(&user))
            .await
            .unwrap();
        let updated = crate::models::comment::find_by_id(&pool, c.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.status, crate::models::comment::CommentStatus::Spam);
    }
}
