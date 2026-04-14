//! 评论服务。
//!
//! 处理评论相关的业务逻辑，包括评论创建（含嵌套深度校验）、
//! 评论列表获取（树形结构）、评论删除和状态管理。

use crate::errors::app_error::{AppError, AppResult};
use crate::eventbus::{Event, EventBus};
use crate::models::comment::{self, CommentResponse};
use crate::models::post;
use crate::plugins::{HookPoint, PluginManager};

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
    pool: &crate::db::Pool,
    plugins: &PluginManager,
    eventbus: &EventBus,
    post_slug: &str,
    author_id: Option<&str>,
    content: &str,
    parent_id: Option<&str>,
    nickname: Option<&str>,
    email: Option<&str>,
) -> AppResult<CommentResponse> {
    let p = post::find_by_slug(pool, post_slug)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    if let Some(pid) = parent_id {
        let all_comments = comment::find_approved_by_post(pool, &p.id).await?;
        let parent = all_comments
            .iter()
            .find(|c| c.id == pid)
            .ok_or_else(|| AppError::NotFound("parent_comment".into()))?;

        if parent.post_id != p.id {
            return Err(AppError::BadRequest("parent_comment_mismatch".into()));
        }

        comment::validate_depth(&all_comments, pid)?;
    }

    let comment_input = CommentInput {
        content: content.to_string(),
        nickname: nickname.map(|s| s.to_string()),
        email: email.map(|s| s.to_string()),
        parent_id: parent_id.map(|s| s.to_string()),
    };

    let filtered = plugins
        .dispatch_filter(HookPoint::CommentCreating, comment_input)
        .await?;

    let c = comment::create(
        pool,
        &p.id,
        author_id,
        filtered.nickname.as_deref(),
        filtered.email.as_deref(),
        &filtered.content,
        filtered.parent_id.as_deref(),
    )
    .await?;

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
    pool: &crate::db::Pool,
    post_slug: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<CommentResponse>, i64)> {
    let p = post::find_by_slug(pool, post_slug)
        .await?
        .ok_or_else(|| AppError::NotFound("post".into()))?;

    let (comments, total) =
        comment::find_approved_by_post_paginated(pool, &p.id, page, page_size).await?;
    Ok((comment::build_tree(&comments), total))
}

/// 删除评论。
///
/// 仅评论作者或管理员有权限执行此操作。
pub async fn delete_comment(
    pool: &crate::db::Pool,
    comment_id: &str,
    user_id: &str,
    role: &str,
) -> AppResult<()> {
    let c = comment::find_by_id(pool, comment_id)
        .await?
        .ok_or_else(|| AppError::NotFound("comment".into()))?;

    if role != "admin" && c.author_id.as_deref() != Some(user_id) {
        return Err(AppError::Forbidden);
    }

    comment::delete(pool, comment_id).await
}

/// 更新评论状态。
///
/// 仅接受 `"approved"`、`"spam"`、`"pending"` 三种状态值。
pub async fn update_comment_status(
    pool: &crate::db::Pool,
    comment_id: &str,
    status: &str,
) -> AppResult<()> {
    if status != "approved" && status != "spam" && status != "pending" {
        return Err(AppError::BadRequest("invalid_comment_status".into()));
    }
    comment::update_status(pool, comment_id, status).await
}
