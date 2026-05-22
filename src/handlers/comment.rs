//! Comment handlers
//!
//! Handles comment creation (logged-in users and guests), listing, review status updates, and deletion.
//! Comments support multi-level nested replies; tree structure is built in the service layer.

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    AdminCommentListQuery, BatchRequest, BatchResponse, CreateCommentRequest,
    UpdateCommentStatusRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::comment::CommentStatus;
use crate::utils::pagination::PaginationParams;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    use crate::middleware::rate_limit::comment_rate_limit;
    use axum::middleware::from_fn;

    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts/{slug}/comments",
        get,
        self::list,
        "system public",
        "comments"
    );
    let r = {
        let mr = axum::routing::post(create_guest).layer(from_fn(comment_rate_limit));
        r.route("/posts/{slug}/comments", mr)
    };
    registry.record(
        "POST",
        "/api/v1/posts/{slug}/comments",
        "system public",
        "comments",
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/posts/{slug}/comments/authed",
        post,
        create,
        "system authed",
        "comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/comments/{id}",
        delete,
        self::delete,
        "system authed",
        "comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/comments/{id}/status",
        put,
        update_status,
        "system authed",
        "comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/comments",
        get,
        list_all,
        "system admin",
        "comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/comments",
        get,
        admin_list,
        "system admin",
        "admin/comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/comments/{id}/status",
        put,
        admin_update_status,
        "system admin",
        "admin/comments"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/comments/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/comments"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/comments/batch",
        post,
        admin_batch,
        "system admin",
        "admin/comments"
    )
}

/// Get the comment list for a specific post (tree structure, paginated)
#[utoipa::path(get, path = "/posts/{slug}/comments", tag = "comments",
    params(("slug" = String, Path, description = "Post slug")),
    responses((status = 200, description = "Comment list for a post"))
)]
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
    let (comments, total) = state
        .comment_service
        .list_paginated(&slug, p.page, p.page_size, &auth)
        .await?;
    Ok(p.paginate(comments, total))
}

/// Admin gets global comment list (paginated)
#[utoipa::path(get, path = "/comments", tag = "comments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Global comment list"))
)]
pub async fn list_all(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Query(params): axum::extract::Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    auth.ensure_admin()?;
    let mut p = params;
    p.sanitize();
    let (comments, total) = state
        .comment_service
        .list_all_paginated(p.page, p.page_size, auth.tenant_id())
        .await?;
    Ok(p.paginate(comments, total))
}

/// Create a comment (logged-in user)
#[utoipa::path(post, path = "/posts/{slug}/comments/authed", tag = "comments",
    security(("bearer_auth" = [])),
    params(("slug" = String, Path, description = "Post slug")),
    request_body = CreateCommentRequest,
    responses((status = 200, description = "Comment created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(slug): Path<String>,
    Json(req): Json<CreateCommentRequest>,
) -> AppResult<ApiResponse<crate::models::comment::CommentResponse>> {
    auth.ensure_authenticated()?;
    validation::validate(&req)?;

    let comment = state
        .comment_service
        .create(
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
#[utoipa::path(post, path = "/posts/{slug}/comments", tag = "comments",
    params(("slug" = String, Path, description = "Post slug")),
    request_body = CreateCommentRequest,
    responses((status = 200, description = "Guest comment created"))
)]
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

    let comment = state
        .comment_service
        .create(
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
#[utoipa::path(delete, path = "/comments/{id}", tag = "comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    responses((status = 200, description = "Comment deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_authenticated()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.comment_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// Update comment review status (admin)
#[utoipa::path(put, path = "/comments/{id}/status", tag = "comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    request_body = UpdateCommentStatusRequest,
    responses((status = 200, description = "Comment status updated"))
)]
pub async fn update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .comment_service
        .update_status(id, req.status, &auth)
        .await?;
    Ok(ApiResponse::success(()))
}

// ── Admin handlers ──

#[utoipa::path(get, path = "/admin/comments", tag = "comments",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Admin comment list"))
)]
pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(query): Query<AdminCommentListQuery>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::comment::AdminCommentRow>>> {
    auth.ensure_admin()?;
    let pagination = PaginationParams::from_options(query.page, query.page_size);
    let (comments, total) = state
        .comment_service
        .list_all_paginated(pagination.page, pagination.page_size, auth.tenant_id())
        .await?;
    Ok(pagination.paginate(comments, total))
}

#[utoipa::path(put, path = "/admin/comments/{id}/status", tag = "comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    request_body = UpdateCommentStatusRequest,
    responses((status = 200, description = "Comment status updated"))
)]
pub async fn admin_update_status(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCommentStatusRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state
        .comment_service
        .update_status(id, req.status, &auth)
        .await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(delete, path = "/admin/comments/{id}", tag = "comments",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Comment ID")),
    responses((status = 200, description = "Comment deleted"))
)]
pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.comment_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/comments/batch", tag = "comments",
    security(("bearer_auth" = [])),
    request_body = BatchRequest,
    responses((status = 200, description = "Batch operation completed"))
)]
pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    for raw_id in &req.ids {
        let Ok(id) = crate::types::snowflake_id::parse_id(raw_id) else {
            continue;
        };
        match req.action.as_str() {
            "delete" if state.comment_service.delete(id, &auth).await.is_ok() => {
                affected += 1;
            }
            "approve" | "reject" | "spam" => {
                let status = match req.action.as_str() {
                    "approve" => CommentStatus::Approved,
                    "reject" => CommentStatus::Pending,
                    "spam" => CommentStatus::Spam,
                    _ => unreachable!(),
                };
                if state
                    .comment_service
                    .update_status(id, status, &auth)
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
