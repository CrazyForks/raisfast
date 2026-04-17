//! 分类相关处理器

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::handlers::dto::{CreateCategoryRequest, UpdateCategoryRequest};
use crate::middleware::auth::AuthorUser;
use crate::middleware::tenant::ResolvedTenant;
use crate::services::post;
use crate::utils::pagination::PaginationParams;

/// 获取分类列表（分页）
#[utoipa::path(get, path = "/categories", tag = "categories",
    responses((status = 200, description = "分类列表"))
)]
pub async fn list(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::models::category::Category>>>
{
    params.sanitize();
    let (items, total) = post::list_categories_paginated(
        state.category_repo.as_ref(),
        tenant.as_str(),
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
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Json(req): Json<CreateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    validation::validate(&req)?;
    let category =
        post::create_category(state.category_repo.as_ref(), req, tenant.as_str()).await?;
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
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
    Json(req): Json<UpdateCategoryRequest>,
) -> AppResult<ApiResponse<crate::models::category::Category>> {
    validation::validate(&req)?;
    let category =
        post::update_category(state.category_repo.as_ref(), &id, req, tenant.as_str()).await?;
    Ok(ApiResponse::success(category))
}

/// 删除分类
#[utoipa::path(delete, path = "/categories/{id}", tag = "categories",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "分类 ID")),
    responses((status = 200, description = "分类已删除"))
)]
pub async fn delete(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post::delete_category(state.category_repo.as_ref(), &id, tenant.as_str()).await?;
    Ok(ApiResponse::success(()))
}
