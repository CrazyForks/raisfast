//! 分类相关处理器

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::category;
use crate::utils::pagination::PaginationParams;

/// 获取分类列表（分页）
#[utoipa::path(get, path = "/categories", tag = "categories",
    responses((status = 200, description = "分类列表"))
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

/// 创建新分类
#[utoipa::path(post, path = "/categories", tag = "categories",
    security(("bearer_auth" = [])),
    request_body = CreateCategoryRequest,
    responses((status = 200, description = "分类已创建"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let category = category::create_category(state.category_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(category))
}

/// 更新分类
#[utoipa::path(put, path = "/categories/{id}", tag = "categories",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "分类 ID")),
    request_body = UpdateCategoryRequest,
    responses((status = 200, description = "分类已更新"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let category = category::update_category(state.category_repo.as_ref(), &auth, &id, req).await?;
    Ok(ApiResponse::success(category))
}

/// 删除分类
#[utoipa::path(delete, path = "/categories/{id}", tag = "categories",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "分类 ID")),
    responses((status = 200, description = "分类已删除"))
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
