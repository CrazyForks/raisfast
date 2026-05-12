//! Category handlers

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{BatchRequest, BatchResponse, CreateCategoryRequest, UpdateCategoryRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::category;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post, put};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/categories",
        get(self::list).post(create),
        "system public",
        "categories",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/categories/{id}",
        get(self::get).put(update).delete(self::delete),
        "system public",
        "categories",
        ["GET", "PUT", "DELETE"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/categories",
        get(admin_list).post(admin_create),
        "system admin",
        "admin/categories",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/categories/{id}",
        put(admin_update).delete(admin_delete),
        "system admin",
        "admin/categories",
        ["PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/categories/batch",
        http_post(admin_batch),
        "system admin",
        "admin/categories",
        ["POST"]
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
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::models::category::Category>>>
{
    params.sanitize();
    let (items, total) = category::list_categories_paginated(
        state.category_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
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
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    let cat = category::get_category(state.category_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(cat))
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
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let cat = category::create_category(state.category_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(cat))
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
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let cat = category::update_category(state.category_repo.as_ref(), &auth, &id, req).await?;
    Ok(ApiResponse::success(cat))
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
    auth.ensure_author()?;
    category::delete_category(state.category_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

// ── Admin handlers ──

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::category::Category>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = category::list_categories_paginated(
        state.category_repo.as_ref(),
        &auth,
        params.page,
        params.page_size,
    )
    .await?;
    Ok(params.paginate(items, total))
}

pub async fn admin_create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let cat = category::create_category(state.category_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(cat))
}

pub async fn admin_update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let cat = category::update_category(state.category_repo.as_ref(), &auth, &id, req).await?;
    Ok(ApiResponse::success(cat))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    category::delete_category(state.category_repo.as_ref(), &id, &auth).await?;
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
        for id in &req.ids {
            if category::delete_category(state.category_repo.as_ref(), id, &auth)
                .await
                .is_ok()
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
