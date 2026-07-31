//! Category handlers

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    BatchRequest, BatchResponse, CategoryResponse, CreateCategoryRequest, UpdateCategoryRequest,
};
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
        "/categories",
        get,
        self::list,
        "blog",
        "categories",
        "public"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/categories",
        create,
        self::create,
        "blog",
        "categories",
        "categories:create"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/categories/{id}",
        get,
        self::get,
        "blog",
        "categories",
        "public"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/categories/{id}",
        put,
        update,
        "blog",
        "categories",
        "categories:update"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/categories/{id}",
        delete,
        self::delete,
        "blog",
        "categories",
        "categories:delete"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/categories",
        get,
        admin_list,
        "blog",
        "admin/categories",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/categories",
        create,
        admin_create,
        "blog",
        "admin/categories",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/categories/{id}",
        put,
        admin_update,
        "blog",
        "admin/categories",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/categories/{id}",
        delete,
        admin_delete,
        "blog",
        "admin/categories",
        "admin"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/categories/batch",
        post,
        admin_batch,
        "blog",
        "admin/categories",
        "admin"
    )
}

/// Get category list (paginated)
#[utoipa::path(get, path = "/categories", tag = "categories",
    responses((status = 200, description = "Category list"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<CategoryResponse>>> {
    params.sanitize();
    let (items, total) = state
        .category_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<CategoryResponse> = items
        .into_iter()
        .map(CategoryResponse::from_category)
        .collect();
    Ok(params.paginate(items, total))
}

/// Get a single category
#[utoipa::path(get, path = "/categories/{id}", tag = "categories",
    params(("id" = String, Path, description = "Category ID")),
    responses((status = 200, description = "Category details"))
)]
pub async fn get(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<CategoryResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state.category_service.get(id, &auth).await?;
    Ok(ApiResponse::success(CategoryResponse::from_category(cat)))
}

/// Create a new category
#[utoipa::path(post, path = "/categories", tag = "categories",
    security(("bearer_auth" = [])),
    request_body = CreateCategoryRequest,
    responses((status = 200, description = "Category created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<CategoryResponse>> {
    validation::validate(&req)?;
    let cat = state.category_service.create(&auth, req).await?;
    Ok(ApiResponse::success(CategoryResponse::from_category(cat)))
}

/// Update a category
#[utoipa::path(put, path = "/categories/{id}", tag = "categories",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Category ID")),
    request_body = UpdateCategoryRequest,
    responses((status = 200, description = "Category updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<CategoryResponse>> {
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state.category_service.update(&auth, id, req).await?;
    Ok(ApiResponse::success(CategoryResponse::from_category(cat)))
}

/// Delete a category
#[utoipa::path(delete, path = "/categories/{id}", tag = "categories",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Category ID")),
    responses((status = 200, description = "Category deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.category_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

// ── Admin handlers ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<CategoryResponse>>> {
    params.sanitize();
    let (items, total) = state
        .category_service
        .list_paginated(&auth, params.page, params.page_size)
        .await?;
    let items: Vec<CategoryResponse> = items
        .into_iter()
        .map(CategoryResponse::from_category)
        .collect();
    Ok(params.paginate(items, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<CategoryResponse>> {
    validation::validate(&req)?;
    let cat = state.category_service.create(&auth, req).await?;
    Ok(ApiResponse::success(CategoryResponse::from_category(cat)))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<CategoryResponse>> {
    validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let cat = state.category_service.update(&auth, id, req).await?;
    Ok(ApiResponse::success(CategoryResponse::from_category(cat)))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.category_service.delete(id, &auth).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    validation::validate(&req)?;
    let mut affected = 0usize;
    if req.action == "delete" {
        for raw_id in &req.ids {
            if let Ok(id) = crate::types::snowflake_id::parse_id(raw_id)
                && state.category_service.delete(id, &auth).await.is_ok()
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
