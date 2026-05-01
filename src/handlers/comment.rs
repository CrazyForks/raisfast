//! 评论相关处理器
//!
//! 处理评论的创建（登录用户和访客）、列表、审核状态更新和删除请求。
//! 评论支持多级嵌套回复，树形结构在 service 层构建。

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::handlers::dto::{CreateCommentRequest, UpdateCommentStatusRequest};
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::middleware::tenant::ResolvedTenant;
use crate::services::comment as comment_service;
use crate::utils::pagination::PaginationParams;

/// 获取指定文章的评论列表（树形结构，分页）
pub async fn list(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
    axum::extract::Query(params): axum::extract::Query<crate::utils::pagination::PaginationParams>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::models::comment::CommentResponse>>,
> {
    let mut p = params;
    p.sanitize();
    let (comments, total) = comment_service::list_comments_paginated(
        state.post_repo.as_ref(),
        state.comment_repo.as_ref(),
        &slug,
        p.page,
        p.page_size,
        tenant.as_str(),
    )
    .await?;
    Ok(p.paginate(comments, total))
}

/// 管理员获取全局评论列表（分页）
pub async fn list_all(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    let mut p = params;
    p.sanitize();
    let (comments, total) = state
        .comment_repo
        .find_all_paginated(p.page, p.page_size, tenant.as_str())
        .await?;
    Ok(p.paginate(comments, total))
}

/// 创建评论（已登录用户）
pub async fn create(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
    auth_user: AuthUser,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    validation::validate(&req)?;

    let comment = comment_service::create_comment(
        state.post_repo.as_ref(),
        state.comment_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        Some(&auth_user.user_id),
        &req.content,
        req.parent_id.as_deref(),
        None,
        None,
        tenant.as_str(),
        &state.aspect_engine,
        &state.pool,
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// 创建评论（访客）
pub async fn create_guest(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(slug): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    validation::validate(&req)?;

    let nickname = req
        .nickname
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("nickname_required".into()))?;

    let comment = comment_service::create_comment(
        state.post_repo.as_ref(),
        state.comment_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        None,
        &req.content,
        req.parent_id.as_deref(),
        Some(nickname),
        req.email.as_deref(),
        tenant.as_str(),
        &state.aspect_engine,
        &state.pool,
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// 删除评论
pub async fn delete(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
) -> AppResult<ApiResponse<()>> {
    comment_service::delete_comment(
        state.comment_repo.as_ref(),
        &id,
        &auth_user.user_id,
        &auth_user.role,
        tenant.as_str(),
        &state.aspect_engine,
        &state.pool,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 更新评论审核状态（管理员）
pub async fn update_status(
    State(state): State<crate::AppState>,
    _admin: AdminUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    comment_service::update_comment_status(
        state.comment_repo.as_ref(),
        &id,
        &req.status,
        tenant.as_str(),
        &state.aspect_engine,
        &state.pool,
    )
    .await?;
    Ok(ApiResponse::success(()))
}
