//! 评论服务。
//!
//! 处理评论相关的业务逻辑，包括评论创建（含嵌套深度校验）、
//! 评论列表获取（树形结构）、评论删除和状态管理。

use crate::aspects::engine::AspectEngine;
use crate::commands::CreateCommentCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::models::comment::{self, CommentResponse};
use crate::plugins::{HookPoint, PluginManager};
use crate::repositories::{CommentRepository, PostRepository};
use crate::services::aspect_dispatch::{AspectDispatch, id_record};

/// 评论输入数据（用于 Hook 传递）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommentInput {
    pub content: String,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub parent_id: Option<String>,
}

/// 创建评论。
///
/// 执行以下校验：
/// 1. 目标文章必须存在。
/// 2. 若指定父评论，父评论必须属于同一篇文章。
/// 3. 嵌套深度不得超过 3 层。
/// 4. 通过插件 `on_comment_creating` Hook 过滤。
///
/// 校验通过后以 `"pending"` 状态创建评论。
#[allow(clippy::too_many_arguments)]
pub async fn create_comment(
    post_repo: &dyn PostRepository,
    comment_repo: &dyn CommentRepository,
    plugins: &PluginManager,
    eventbus: &EventBus,
    post_slug: &str,
    author_id: Option<&str>,
    content: &str,
    parent_id: Option<&str>,
    nickname: Option<&str>,
    email: Option<&str>,
    tenant_id: Option<&str>,
    aspect_engine: &AspectEngine,
    pool: &crate::db::pool::Pool,
) -> AppResult<CommentResponse> {
    let p = post_repo
        .find_by_slug(post_slug, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    if let Some(pid) = parent_id {
        let all_comments = comment_repo.find_approved_by_post(&p.id, tenant_id).await?;
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

    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "comments",
        user_id: author_id,
        tenant_id,
    };
    dsp.before_create(id_record("")).await?;
    let c = comment_repo
        .create(
            CreateCommentCmd {
                post_id: p.id,
                author_id: author_id.map(std::string::ToString::to_string),
                nickname: filtered.nickname,
                email: filtered.email,
                content: filtered.content,
                parent_id: filtered.parent_id,
            },
            tenant_id,
        )
        .await?;
    dsp.after_create(id_record(&c.id)).await;

    eventbus.emit(Event::CommentCreated {
        id: c.id.clone(),
        post_slug: post_slug.to_string(),
        author_name: c.nickname.clone().unwrap_or_default(),
    });

    Ok(CommentResponse {
        id: c.id,
        post_id: c.post_id,
        author_id: c.author_id,
        nickname: c.nickname,
        content: c.content,
        parent_id: c.parent_id,
        depth: 0,
        replies: vec![],
        created_at: c.created_at,
    })
}

/// 分页获取指定文章的评论列表。
///
/// 仅返回状态为 `"approved"` 的评论，并组织为树形结构。
pub async fn list_comments_paginated(
    post_repo: &dyn PostRepository,
    comment_repo: &dyn CommentRepository,
    post_slug: &str,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<CommentResponse>, i64)> {
    let p = post_repo
        .find_by_slug(post_slug, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("post"))?;

    let (comments, total) = comment_repo
        .find_approved_by_post_paginated(&p.id, page, page_size, tenant_id)
        .await?;
    Ok((comment::build_tree(&comments), total))
}

/// 删除评论。
///
/// 仅评论作者或管理员有权限执行此操作。
pub async fn delete_comment(
    comment_repo: &dyn CommentRepository,
    comment_id: &str,
    user_id: &str,
    role: &str,
    tenant_id: Option<&str>,
    aspect_engine: &AspectEngine,
    pool: &crate::db::pool::Pool,
) -> AppResult<()> {
    let c = comment_repo
        .find_by_id(comment_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("comment"))?;

    crate::utils::auth::require_owner_or_admin_opt(role, user_id, c.author_id.as_deref())?;

    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "comments",
        user_id: Some(user_id),
        tenant_id,
    };
    dsp.before_delete(id_record(comment_id)).await?;
    comment_repo.delete(comment_id, tenant_id).await?;
    dsp.after_delete().await;
    Ok(())
}

/// 更新评论状态。
///
/// 仅接受 `"approved"`、`"spam"`、`"pending"` 三种状态值。
pub async fn update_comment_status(
    comment_repo: &dyn CommentRepository,
    comment_id: &str,
    status: &str,
    tenant_id: Option<&str>,
    aspect_engine: &AspectEngine,
    pool: &crate::db::pool::Pool,
) -> AppResult<()> {
    if status != "approved" && status != "spam" && status != "pending" {
        return Err(AppError::BadRequest("invalid_comment_status".into()));
    }
    let dsp = AspectDispatch {
        engine: aspect_engine,
        pool,
        table: "comments",
        user_id: None,
        tenant_id,
    };
    dsp.before_update(id_record(comment_id), id_record(comment_id))
        .await?;
    comment_repo
        .update_status(comment_id, status, tenant_id)
        .await?;
    dsp.after_update(id_record(comment_id)).await;
    Ok(())
}
