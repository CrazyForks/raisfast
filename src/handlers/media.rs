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
    tracing::info!(filename = %filename, content_type = %content_type, "uploading media file");
    let data = field
        .bytes()
        .await
        .map_err(|_| AppError::BadRequest("file_data_read_failed".into()))?;

    let bucket = "blog";

    let media = media_service::save_file(
        state.storage.as_ref(),
        state.media_repo.as_ref(),
        &auth_user.user_id,
        state.config.max_upload_size,
        bucket,
        &filename,
        &content_type,
        &data,
        tenant.as_str(),
    )
    .await?;

    let url = state.storage.url(&media.filepath).await?;
    Ok(ApiResponse::success(
        crate::handlers::dto::media_to_response_with_url(&media, &url),
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
    let (items, total) = media_service::list(
        state.media_repo.as_ref(),
        &auth_user.user_id,
        params.page,
        params.page_size,
        tenant.as_str(),
    )
    .await?;

    let storage = state.storage.as_ref();
    let responses = futures::future::join_all(items.iter().map(|m| async {
        let url = storage.url(&m.filepath).await.unwrap_or_default();
        crate::handlers::dto::media_to_response_with_url(m, &url)
    }))
    .await;

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
        state.storage.as_ref(),
        state.media_repo.as_ref(),
        &id,
        &auth_user.user_id,
        &auth_user.role,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 获取存储统计
pub async fn stats(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
) -> AppResult<ApiResponse<crate::handlers::dto::MediaStatsResponse>> {
    let s = media_service::stats(
        state.media_repo.as_ref(),
        &auth_user.user_id,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(
        crate::handlers::dto::stats_to_response(&s),
    ))
}
