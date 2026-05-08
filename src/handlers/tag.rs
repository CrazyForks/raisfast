//! 标签相关处理器

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::dto::{CreateTagRequest, UpdateTagRequest};
use crate::middleware::auth::AuthUser;
use crate::services::tag;
use crate::utils::pagination::PaginationParams;

/// 获取标签列表（分页）
#[utoipa::path(get, path = "/tags", tag = "tags",
    responses((status = 200, description = "标签列表"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::models::tag::Tag>>> {
    params.sanitize();
    let (items, total) = tag::list_tags_paginated(
        state.tag_repo.as_ref(),
        &auth,
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
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let tag = tag::create_tag(state.tag_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(tag))
}

/// 删除标签
#[utoipa::path(delete, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "标签 ID")),
    responses((status = 200, description = "标签已删除"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_author()?;
    tag::delete_tag(state.tag_repo.as_ref(), &id, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// 更新标签
#[utoipa::path(put, path = "/tags/{id}", tag = "tags",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "标签 ID")),
    request_body = UpdateTagRequest,
    responses((status = 200, description = "标签已更新"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    auth.ensure_author()?;
    validation::validate(&req)?;
    let tag = tag::update_tag(state.tag_repo.as_ref(), &id, &auth, req.name).await?;
    Ok(ApiResponse::success(tag))
}
