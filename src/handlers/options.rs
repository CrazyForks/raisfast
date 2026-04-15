//! 站点配置 API Handler

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;

/// GET /options/public — 公开配置
pub async fn get_public_options(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<HashMap<String, Value>>> {
    let options = state.options.get_public().await;
    Ok(ApiResponse::success(options))
}

/// GET /admin/options — 所有配置
pub async fn list_options(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<HashMap<String, Value>>> {
    let options = state.options.get_all().await?;
    Ok(ApiResponse::success(options))
}

/// GET /admin/options/:key — 获取单个配置
pub async fn get_option(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    match state.options.get(&key).await {
        Some(value) => Ok(ApiResponse::success(serde_json::json!({
            "key": key,
            "value": value,
        }))),
        None => Err(AppError::not_found(&format!("option/{key}"))),
    }
}

/// 批量更新请求体
#[derive(Debug, Deserialize)]
pub struct UpdateOptionsRequest {
    pub options: HashMap<String, Value>,
}

/// PUT /admin/options — 批量更新配置
pub async fn update_options(
    State(state): State<AppState>,
    Json(body): Json<UpdateOptionsRequest>,
) -> AppResult<ApiResponse<HashMap<String, Value>>> {
    state.options.set_batch(body.options).await?;
    let options = state.options.get_all().await?;
    Ok(ApiResponse::success(options))
}

/// 更新单个配置请求体
#[derive(Debug, Deserialize)]
pub struct UpdateOptionRequest {
    pub value: Value,
}

/// PUT /admin/options/:key — 设置单个配置
pub async fn set_option(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<UpdateOptionRequest>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.options.set(&key, body.value).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "key": key,
        "updated": true,
    })))
}

/// DELETE /admin/options/:key — 删除配置
pub async fn delete_option(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.options.delete(&key).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "key": key,
        "deleted": true,
    })))
}
