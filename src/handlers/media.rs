//! 媒体文件相关处理器
//!
//! 处理媒体文件的上传、列表和删除请求。
//! 上传使用 `multipart/form-data` 格式。

use axum::extract::{Multipart, Path, Query, State};

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::middleware::auth::AuthUser;
use crate::services::media as media_service;
use crate::utils::pagination::PaginationParams;

/// 上传媒体文件
///
/// - **方法/路径：** `POST /api/media`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 接收 `multipart/form-data` 上传的文件，保存到磁盘并创建数据库记录。
///   受 `max_upload_size` 配置限制。
/// - **返回：** `ApiResponse<MediaResponse>`（包含完整访问 URL）
pub async fn upload(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<crate::models::media::MediaResponse>> {
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
        &state.pool,
        &auth_user.user_id,
        &state.config.upload_dir,
        state.config.max_upload_size,
        &filename,
        &content_type,
        &data,
    )
    .await?;

    let base_url = &state.config.base_url;
    Ok(ApiResponse::success(media.to_response(base_url)))
}

/// 获取当前用户的媒体文件列表（分页）
///
/// - **方法/路径：** `GET /api/media`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 分页查询当前用户上传的媒体文件，返回包含完整 URL 的响应。
/// - **返回：** `ApiResponse<PaginatedData<MediaResponse>>`
pub async fn list(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::models::media::MediaResponse>>> {
    params.sanitize();
    let base_url = &state.config.base_url;
    let (items, total) = media_service::list(
        &state.pool,
        &auth_user.user_id,
        params.page,
        params.page_size,
    )
    .await?;
    let responses = items.iter().map(|m| m.to_response(base_url)).collect();

    Ok(ApiResponse::success(PaginatedData {
        items: responses,
        total,
        page: params.page,
        page_size: params.page_size,
    }))
}

/// 删除媒体文件
///
/// - **方法/路径：** `DELETE /api/media/:id`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 删除指定的媒体文件（数据库记录和磁盘文件）。仅文件所有者或管理员可删除。
/// - **返回：** `ApiResponse<()>`
pub async fn delete(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    media_service::delete_media(
        &state.pool,
        &state.config.upload_dir,
        &id,
        &auth_user.user_id,
        &auth_user.role,
    )
    .await?;
    Ok(ApiResponse::success(()))
}
