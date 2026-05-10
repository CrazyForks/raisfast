//! 媒体文件相关处理器

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};

use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::middleware::auth::AuthUser;
use crate::services::media as media_service;
use crate::utils::pagination::PaginationParams;

pub fn routes(max_upload: usize, registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get, post as http_post};
    use tower_http::limit::RequestBodyLimitLayer;

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/media/upload", http_post(upload).layer(RequestBodyLimitLayer::new(max_upload)), "system public", "media", ["POST"]);
    let r = reg_route!(r, registry, "/media", get(self::list), "system public", "media", ["GET"]);
    let r = reg_route!(r, registry, "/media/stats", get(stats), "system public", "media", ["GET"]);
    let r = reg_route!(r, registry, "/media/{id}", delete(self::delete), "system public", "media", ["DELETE"]);
    let r = reg_route!(r, registry, "/admin/media/upload", http_post(admin_upload).layer(RequestBodyLimitLayer::new(max_upload)), "system admin", "admin/media", ["POST"]);
    let r = reg_route!(r, registry, "/admin/media", get(admin_list), "system admin", "admin/media", ["GET"]);
    let r = reg_route!(r, registry, "/admin/media/{id}", delete(admin_delete), "system admin", "admin/media", ["DELETE"]);
    reg_route!(r, registry, "/admin/media/batch", http_post(admin_batch), "system admin", "admin/media", ["POST"])
}


/// 上传媒体文件
pub async fn upload(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<crate::dto::MediaResponse>> {
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
        &state.pool,
        &auth,
        state.config.max_upload_size,
        bucket,
        &filename,
        &content_type,
        &data,
    )
    .await?;

    let url = state.storage.url(&media.filepath).await?;
    Ok(ApiResponse::success(
        crate::dto::media_to_response_with_url(&media, &url),
    ))
}

/// 获取当前用户的媒体文件列表（分页）
pub async fn list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::dto::MediaResponse>>> {
    params.sanitize();
    let (items, total) = media_service::list(
        state.media_repo.as_ref(),
        &state.pool,
        &auth,
        params.page,
        params.page_size,
    )
    .await?;

    let storage = state.storage.as_ref();
    let responses = futures::future::join_all(items.iter().map(|m| async {
        let url = storage.url(&m.filepath).await.unwrap_or_default();
        crate::dto::media_to_response_with_url(m, &url)
    }))
    .await;

    Ok(params.paginate(responses, total))
}

/// 删除媒体文件
pub async fn delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    media_service::delete_media(
        state.storage.as_ref(),
        state.media_repo.as_ref(),
        &state.pool,
        &id,
        &auth,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 获取存储统计
pub async fn stats(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<crate::dto::MediaStatsResponse>> {
    let s = media_service::stats(state.media_repo.as_ref(), &state.pool, &auth).await?;
    Ok(ApiResponse::success(crate::dto::stats_to_response(&s)))
}

// ── Admin handlers ──

pub async fn admin_upload(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<crate::dto::MediaResponse>> {
    auth.ensure_admin()?;
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
        state.storage.as_ref(),
        state.media_repo.as_ref(),
        &state.pool,
        &auth,
        state.config.max_upload_size,
        "blog",
        &filename,
        &content_type,
        &data,
    )
    .await?;

    let url = state.storage.url(&media.filepath).await?;
    Ok(ApiResponse::success(
        crate::dto::media_to_response_with_url(&media, &url),
    ))
}

pub async fn admin_list(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<crate::dto::MediaResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = media_service::admin_list(
        state.media_repo.as_ref(),
        params.page,
        params.page_size,
        &auth,
    )
    .await?;

    let storage = state.storage.as_ref();
    let responses = futures::future::join_all(items.iter().map(|m| async {
        let url = storage.url(&m.filepath).await.unwrap_or_default();
        crate::dto::media_to_response_with_url(m, &url)
    }))
    .await;

    Ok(params.paginate(responses, total))
}

pub async fn admin_delete(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    media_service::admin_delete_media(
        state.storage.as_ref(),
        state.media_repo.as_ref(),
        &state.pool,
        &id,
        &auth,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    if req.action == "delete" {
        for id in &req.ids {
            if media_service::admin_delete_media(
                state.storage.as_ref(),
                state.media_repo.as_ref(),
                &state.pool,
                id,
                &auth,
            )
            .await
            .is_ok()
            {
                affected += 1;
            }
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(&req.action, affected)))
}
