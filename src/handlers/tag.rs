//! 标签相关处理器

use axum::Json;
use axum::extract::{Path, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::handlers::dto::CreateTagRequest;
use crate::middleware::auth::AuthorUser;
use crate::middleware::tenant::ResolvedTenant;
use crate::services::post;

/// 获取所有标签列表
pub async fn list(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
) -> AppResult<ApiResponse<Vec<crate::models::tag::Tag>>> {
    let tags = post::list_tags(state.tag_repo.as_ref(), tenant.as_str()).await?;
    Ok(ApiResponse::success(tags))
}

/// 创建新标签
pub async fn create(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Json(req): Json<CreateTagRequest>,
) -> AppResult<ApiResponse<crate::models::tag::Tag>> {
    validation::validate(&req)?;
    let tag = post::create_tag(state.tag_repo.as_ref(), req, tenant.as_str()).await?;
    Ok(ApiResponse::success(tag))
}

/// 删除标签
pub async fn delete(
    State(state): State<crate::AppState>,
    _author: AuthorUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    post::delete_tag(state.tag_repo.as_ref(), &id, tenant.as_str()).await?;
    Ok(ApiResponse::success(()))
}
