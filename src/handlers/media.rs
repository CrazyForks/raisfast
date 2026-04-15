//! 媒体文件相关处理器

use axum::extract::{Multipart, Path, Query, State};

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::middleware::auth::AuthUser;
use crate::middleware::tenant::ResolvedTenant;
use crate::services::media as media_service;
use crate::utils::pagination::PaginationParams;

/// 上传媒体文件
pub async fn upload(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<crate::handlers::dto::MediaResponse>> {
    let field = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("multipart_read_failed".into()))?
        .ok_or_else(|| AppError::BadRequest("no_file".into()))?;

    let filename = field.file_name().unwrap_or("unknown").to_string();
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = field
        .bytes()
        .await
        .map_err(|_| AppError::BadRequest("file_data_read_failed".into()))?;

    let media = media_service::save_file(
        state.media_repo.as_ref(),
        &auth_user.user_id,
        &state.config.upload_dir,
        state.config.max_upload_size,
        &filename,
        &content_type,
        &data,
        tenant.as_str(),
    )
    .await?;

    let base_url = &state.config.base_url;
    Ok(ApiResponse::success(
        crate::handlers::dto::media_to_response(&media, base_url),
    ))
}

/// 获取当前用户的媒体文件列表（分页）
pub async fn list(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::handlers::dto::MediaResponse>>> {
    params.sanitize();
    let base_url = &state.config.base_url;
    let (items, total) = media_service::list(
        state.media_repo.as_ref(),
        &auth_user.user_id,
        params.page,
        params.page_size,
        tenant.as_str(),
    )
    .await?;
    let responses = items
        .iter()
        .map(|m| crate::handlers::dto::media_to_response(m, base_url))
        .collect();

    Ok(params.paginate(responses, total))
}

/// 删除媒体文件
pub async fn delete(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    media_service::delete_media(
        state.media_repo.as_ref(),
        &state.config.upload_dir,
        &id,
        &auth_user.user_id,
        &auth_user.role,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}
