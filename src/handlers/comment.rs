//! Comment handlers
//!
//! Handles comment creation (logged-in users and guests), listing, review status updates, and deletion.
//! Comments support multi-level nested replies; tree structure is built in the service layer.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::dto::{BatchRequest, BatchResponse, CreateCommentRequest, UpdateCommentStatusRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::comment::CommentStatus;
use crate::services::comment as comment_service;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use crate::middleware::rate_limit::comment_rate_limit;
    use axum::middleware::from_fn;
    use axum::routing::{delete, get, post as http_post, put};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/posts/{slug}/comments",
        get(self::list),
        "system public",
        "comments",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/posts/{slug}/comments",
        http_post(create_guest).layer(from_fn(comment_rate_limit)),
        "system public",
        "comments",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/posts/{slug}/comments/authed",
        http_post(create),
        "system public",
        "comments",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/comments/{id}",
        delete(self::delete),
        "system public",
        "comments",
        ["DELETE"]
    );
    let r = reg_route!(
        r,
        registry,
        "/comments/{id}/status",
        put(update_status),
        "system public",
        "comments",
        ["PUT"]
    );
    let r = reg_route!(
        r,
        registry,
        "/comments",
        get(list_all),
        "system public",
        "comments",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/comments",
        get(admin_list),
        "system admin",
        "admin/comments",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/comments/{id}/status",
        put(admin_update_status),
        "system admin",
        "admin/comments",
        ["PUT"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/comments/{id}",
        delete(admin_delete),
        "system admin",
        "admin/comments",
        ["DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/comments/batch",
        http_post(admin_batch),
        "system admin",
        "admin/comments",
        ["POST"]
    )
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct AdminCommentListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<CommentStatus>,
}

/// Get the comment list for a specific post (tree structure, paginated)
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
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
        &auth,
    )
    .await?;
    Ok(p.paginate(comments, total))
}

/// Admin gets global comment list (paginated)
pub async fn list_all(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    auth.ensure_admin()?;
    let mut p = params;
    p.sanitize();
    let (comments, total) = state
        .comment_repo
        .find_all_paginated(p.page, p.page_size, auth.tenant_id())
        .await?;
    Ok(p.paginate(comments, total))
}

/// Create a comment (logged-in user)
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    validation::validate(&req)?;

    let comment = comment_service::create_comment(
        state.post_repo.as_ref(),
        state.comment_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth,
        &req.content,
        req.parent_id.as_deref(),
        None,
        None,
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// Create a comment (guest)
pub async fn create_guest(
    auth: AuthUser,
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
        state.post_repo.as_ref(),
        state.comment_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &slug,
        &auth,
        &req.content,
        req.parent_id.as_deref(),
        Some(nickname),
        req.email.as_deref(),
    )
    .await?;

    Ok(ApiResponse::success(comment))
}

/// Delete a comment
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    comment_service::delete_comment(state.comment_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// Update comment review status (admin)
pub async fn update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    comment_service::update_comment_status(state.comment_repo.as_ref(), &id, req.status, &auth)
        .await?;
    Ok(ApiResponse::success(()))
}

// ── Admin handlers ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminCommentListQuery>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    auth.ensure_admin()?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);
    let (comments, total) = state
        .comment_repo
        .find_all_paginated(pagination.page, pagination.page_size, auth.tenant_id())
        .await?;
    Ok(pagination.paginate(comments, total))
}

pub async fn admin_update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    comment_service::update_comment_status(state.comment_repo.as_ref(), &id, req.status, &auth)
        .await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    comment_service::delete_comment(state.comment_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if comment_service::delete_comment(state.comment_repo.as_ref(), id, &auth)
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            "approve" | "reject" | "spam" => {
                let status = match req.action.as_str() {
                    "approve" => CommentStatus::Approved,
                    "reject" => CommentStatus::Pending,
                    "spam" => CommentStatus::Spam,
                    _ => unreachable!(),
                };
                if comment_service::update_comment_status(
                    state.comment_repo.as_ref(),
                    id,
                    status,
                    &auth,
                )
                .await
                .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
