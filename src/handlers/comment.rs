//! 评论相关处理器
//!
//! 处理评论的创建（登录用户和访客）、列表、审核状态更新和删除请求。
//! 评论支持多级嵌套回复，树形结构在 service 层构建。

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::models::comment::{CreateCommentRequest, UpdateCommentStatusRequest};
use crate::services::comment as comment_service;
use crate::utils::pagination::PaginationParams;

/// 获取指定文章的评论列表（树形结构）
///
/// - **方法/路径：** `GET /api/posts/:slug/comments`
/// - **认证：** 无需认证
/// - **说明：** 返回指定文章下已审核通过的评论，以嵌套树形结构返回。
/// - **返回：** `ApiResponse<Vec<CommentResponse>>`
pub async fn list(
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
) -> AppResult<ApiResponse<Vec<crate::models::comment::CommentResponse>>> {
    let comments = comment_service::list_comments(&state.pool, &slug).await?;
    Ok(ApiResponse::success(comments))
}

/// 管理员获取全局评论列表（分页）
///
/// - **方法/路径：** `GET /api/v1/comments`
/// - **认证：** 需要管理员权限
/// - **查询参数：** `page`, `page_size`
pub async fn list_all(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    let mut p = params;
    p.sanitize();
    let (comments, total) =
        crate::models::comment::find_all_paginated(&state.pool, p.page, p.page_size).await?;
    Ok(ApiResponse::success(PaginatedData {
        items: comments,
        total,
        page: p.page,
        page_size: p.page_size,
    }))
}

/// 创建评论（已登录用户）
///
/// - **方法/路径：** `POST /api/posts/:slug/comments`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 已登录用户创建评论，自动关联 `author_id`，支持回复（`parent_id`）。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<CommentResponse>`
pub async fn create(
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
    auth_user: AuthUser,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    validation::validate(&req)?;

    let comment = comment_service::create_comment(
        &state.pool,
        &slug,
        Some(&auth_user.user_id),
        &req.content,
        req.parent_id.as_deref(),
        None,
        None,
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// 创建评论（访客）
///
/// - **方法/路径：** `POST /api/posts/:slug/comments/guest`
/// - **认证：** 无需认证
/// - **说明：** 访客创建评论，`nickname` 为必填字段，`email` 可选。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<CommentResponse>`
pub async fn create_guest(
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    validation::validate(&req)?;

    let nickname = req
        .nickname
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("nickname_required".into()))?;

    let comment = comment_service::create_comment(
        &state.pool,
        &slug,
        None,
        &req.content,
        req.parent_id.as_deref(),
        Some(nickname),
        req.email.as_deref(),
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// 删除评论
///
/// - **方法/路径：** `DELETE /api/comments/:id`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 评论作者或管理员可删除评论。
/// - **返回：** `ApiResponse<()>`
pub async fn delete(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    auth_user: AuthUser,
) -> AppResult<ApiResponse<()>> {
    comment_service::delete_comment(&state.pool, &id, &auth_user.user_id, &auth_user.role).await?;
    Ok(ApiResponse::success(()))
}

/// 更新评论审核状态（管理员）
///
/// - **方法/路径：** `PUT /api/comments/:id/status`
/// - **认证：** 需要管理员权限（`AdminUser`）
/// - **说明：** 更新评论的审核状态（`pending`/`approved`/`rejected`）。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<()>`
pub async fn update_status(
    State(state): State<crate::AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    comment_service::update_comment_status(&state.pool, &id, &req.status).await?;
    Ok(ApiResponse::success(()))
}
