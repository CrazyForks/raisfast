//! Tag handlers

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{BatchRequest, BatchResponse, CreateTagRequest, TagResponse, UpdateTagRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::utils::pagination::PaginationParams;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tags",
        get,
        self::list,
        "system public",
        "tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tags",
        create,
        self::create,
        "system public",
        "tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tags/{id}",
        get,
        self::get,
        "system public",
        "tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tags/{id}",
        put,
        update,
        "system public",
        "tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tags/{id}",
        delete,
        self::delete,
        "system public",
        "tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tags",
        get,
        admin_list,
        "system admin",
        "admin/tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tags",
        create,
        admin_create,
        "system admin",
        "admin/tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tags/{id}",
        put,
        admin_update,
        "system admin",
        "admin/tags"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tags/{id}",
        delete,
        admin_delete,
        "system admin",
        "admin/tags"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/tags/batch",
        post,
        admin_batch,
        "system admin",
        "admin/tags"
    )
}

/// Get tag list (paginated)
#[utoipa::path(get, path = "/tags", tag = "tags",
    responses((status = 200, description = "Tag list"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<TagResponse>>> {
    params.sanitize();
    let (items, total) = state
        .tag_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<TagResponse> = items.into_iter().map(TagResponse::from_tag).collect();
    Ok(params.paginate(items, total))
}

/// Get a single tag
#[utoipa::path(get, path = "/tags/{id}", tag = "tags",
    params(("id" = String, Path, description = "Tag ID")),
    responses((status = 200, description = "Tag details"))
)]
pub async fn get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<TagResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let t = state.tag_service.get(id, &auth).await?;
    Ok(ApiResponse::success(TagResponse::from_tag(t)))
}

/// Create a new tag
#[utoipa::path(post, path = "/tags", tag = "tags",
    security(("bearer_auth" = [])),
    request_body = CreateTagRequest,
    responses((status = 200, description = "Tag created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<TagResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let t = state.tag_service.create(&auth, req).await?;
    Ok(ApiResponse::success(TagResponse::from_tag(t)))
}

/// Delete a tag
#[utoipa::path(delete, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Tag ID")),
    responses((status = 200, description = "Tag deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.tag_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// Update a tag
#[utoipa::path(put, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Tag ID")),
    request_body = UpdateTagRequest,
    responses((status = 200, description = "Tag updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<ApiResponse<TagResponse>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let slug = crate::services::tag::generate_slug(&req.name);
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let t = state
        .tag_service
        .update(&auth, id, req.name.clone(), slug)
        .await?;
    Ok(ApiResponse::success(TagResponse::from_tag(t)))
}

// ── Admin handlers ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<TagResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = state
        .tag_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<TagResponse> = items.into_iter().map(TagResponse::from_tag).collect();
    Ok(params.paginate(items, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<TagResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let t = state.tag_service.create(&auth, req).await?;
    Ok(ApiResponse::success(TagResponse::from_tag(t)))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<ApiResponse<TagResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let slug = crate::services::tag::generate_slug(&req.name);
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let t = state
        .tag_service
        .update(&auth, id, req.name.clone(), slug)
        .await?;
    Ok(ApiResponse::success(TagResponse::from_tag(t)))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.tag_service.delete(id, &auth).await?;
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
    if req.action == "delete" {
        for raw_id in &req.ids {
            if let Ok(id) = crate::types::snowflake_id::parse_id(raw_id)
                && state.tag_service.delete(id, &auth).await.is_ok()
            {
                affected += 1;
            }
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
