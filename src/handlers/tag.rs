//! 标签相关处理器

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::handlers::dto::CreateTagRequest;
use crate::middleware::auth::AuthorUser;
use crate::middleware::tenant::ResolvedTenant;
use crate::services::post;
use crate::utils::pagination::PaginationParams;

/// 获取标签列表（分页）
#[utoipa::path(get, path = "/tags", tag = "tags",
    responses((status = 200, description = "标签列表"))
)]
pub async fn list(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::models::tag::Tag>>> {
    params.sanitize();
    let (items, total) = post::list_tags_paginated(
        state.tag_repo.as_ref(),
        tenant.as_str(),
        params.page,
        params.page_size,
    )
    .await?;
    Ok(params.paginate(items, total))
}

/// 创建新标签
#[utoipa::path(post, path = "/tags", tag = "tags",
    security(("bearer_auth" = [])),
    request_body = CreateTagRequest,
    responses((status = 200, description = "标签已创建"))
)]
pub async fn create(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    validation::validate(&req)?;
    let tag = post::create_tag(
        state.tag_repo.as_ref(),
        &state.aspect_engine,
        &state.pool,
        Some(&_author.user_id),
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(tag))
}

/// 删除标签
#[utoipa::path(delete, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "标签 ID")),
    responses((status = 200, description = "标签已删除"))
)]
pub async fn delete(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post::delete_tag(
        state.tag_repo.as_ref(),
        &state.aspect_engine,
        &state.pool,
        Some(&_author.user_id),
        &id,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}
