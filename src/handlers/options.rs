//! 站点配置 API Handler

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::get;

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/options/public", get(get_public_options), "system", "options", ["GET"]);
    let r = reg_route!(r, registry, "/admin/options", get(list_options).put(update_options), "system", "options", ["GET", "PUT"]);
    reg_route!(r, registry, "/admin/options/{key}", get(get_option).put(set_option).delete(delete_option), "system", "options", ["GET", "PUT", "DELETE"])
}


/// GET /options/public — 公开配置（仅值）+ 系统特性标志
pub async fn get_public_options(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<HashMap<String, Value>>> {
    let mut options = state.options.get_public().await;
    options.insert(
        "builtin_tenantable".into(),
        Value::Bool(state.config.builtin_tenantable),
    );
    Ok(ApiResponse::success(options))
}

/// GET /admin/options — 所有配置（按分组，含元数据）
pub async fn list_options(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    let groups = state.options.get_grouped().await?;
    Ok(ApiResponse::success(groups))
}

/// GET /admin/options/:key — 获取单个配置
pub async fn get_option(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let entry = state
        .options
        .get_entry(&key)
        .await
        .ok_or_else(|| AppError::not_found(&format!("option/{key}")))?;
    Ok(ApiResponse::success(
        serde_json::to_value(entry).map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?,
    ))
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
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    state.options.set_batch(body.options).await?;
    let groups = state.options.get_grouped().await?;
    Ok(ApiResponse::success(groups))
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
        "option_key": key,
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
        "option_key": key,
        "deleted": true,
    })))
}
