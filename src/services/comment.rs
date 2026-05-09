//! 评论服务。
//!
//! 处理评论相关的业务逻辑，包括评论创建（含嵌套深度校验）、
//! 评论列表获取（树形结构）、评论删除和状态管理。

use crate::commands::CreateCommentCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::middleware::auth::AuthUser;
use crate::models::comment::{self, CommentResponse};
use crate::plugins::{HookPoint, PluginManager};
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

    eventbus.emit(Event::CommentCreated {
        id: c.document_id.clone(),
        post_slug: post_slug.to_string(),
        author_name: c.nickname.clone().unwrap_or_default(),
    });

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

    crate::utils::auth::require_owner_or_admin_opt(
        auth.role(),
        auth.user_int_id().ok_or(AppError::Unauthorized)?,
        c.created_by,
    )?;

    comment_repo.delete(c.id, auth.tenant_id()).await?;
    Ok(())
}

pub async fn update_comment_status(
    comment_repo: &dyn CommentRepository,
    comment_id: &str,
    status: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    if status != "approved" && status != "spam" && status != "pending" {
        return Err(AppError::BadRequest("invalid_comment_status".into()));
    }
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

    fn auth() -> AuthUser {
        AuthUser::from_parts(Some("u1".to_string()), Some(1), "admin".to_string(), None)
    }

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let sql = format!(
            "INSERT INTO users (document_id, email, username, password_hash, role) VALUES ({}, {}, {}, {}, {}) RETURNING id",
            crate::db::dialect::ph(1),
            crate::db::dialect::ph(2),
            crate::db::dialect::ph(3),
            crate::db::dialect::ph(4),
            crate::db::dialect::ph(5)
        );
        let (id,): (i64,) = sqlx::query_as(&sql)
            .bind("u1")
            .bind("u1@test.com")
            .bind("u1user")
            .bind("$argon2id$v=19$m=19456,t=2,p=1$test$test")
            .bind("admin")
            .fetch_one(pool)
            .await
            .unwrap();
        id
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
                status: "published".into(),
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
        let user_id = insert_user(&pool).await;
        let post_id = insert_post(&pool, user_id).await;
        let c = insert_comment(&pool, post_id, user_id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        super::update_comment_status(&repo, &c.document_id, "approved", &auth())
            .await
            .unwrap();
        let updated = repo.find_by_id(c.id, None).await.unwrap().unwrap();
        assert_eq!(updated.status, "approved");
    }

    #[tokio::test]
    async fn update_comment_status_invalid() {
        let pool = setup_pool().await;
        let repo = SqlxCommentRepository::new(pool.clone());
        let err = super::update_comment_status(&repo, "x", "invalid", &auth())
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("invalid_comment_status"), "got: {msg}");
    }

    #[tokio::test]
    async fn update_comment_status_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxCommentRepository::new(pool.clone());
        assert!(
            super::update_comment_status(&repo, "missing", "approved", &auth())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn delete_comment_by_owner() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let post_id = insert_post(&pool, user_id).await;
        let c = insert_comment(&pool, post_id, user_id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        let a = AuthUser::from_parts(
            Some("u1".to_string()),
            Some(user_id),
            "admin".to_string(),
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
        assert!(
            super::delete_comment(&repo, "missing", &auth())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn update_comment_status_spam() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let post_id = insert_post(&pool, user_id).await;
        let c = insert_comment(&pool, post_id, user_id).await;
        let repo = SqlxCommentRepository::new(pool.clone());
        super::update_comment_status(&repo, &c.document_id, "spam", &auth())
            .await
            .unwrap();
        let updated = repo.find_by_id(c.id, None).await.unwrap().unwrap();
        assert_eq!(updated.status, "spam");
    }
}
