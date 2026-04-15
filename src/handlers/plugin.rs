//! 插件管理 API handler
//!
//! 提供运行时插件管理端点：列表、详情、启用、禁用、重载。

use axum::extract::{Path, State};

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AdminUser;
use crate::plugins::PluginInfoResponse;

/// GET /api/v1/admin/plugins — 列出所有插件及状态
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<PluginInfoResponse>>> {
    let plugins = state.plugins.list_plugins_detail().await;
    Ok(ApiResponse::success(plugins))
}

/// GET /api/v1/admin/plugins/:id — 插件详情
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PluginInfoResponse>> {
    let detail = state
        .plugins
        .get_plugin_detail(&id)
        .await
        .ok_or_else(|| AppError::not_found("plugin"))?;
    Ok(ApiResponse::success(detail))
}

/// POST /api/v1/admin/plugins/:id/enable — 启用插件
pub async fn enable(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.plugins.enable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/disable — 禁用插件
pub async fn disable(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.plugins.disable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/reload — 重载插件
pub async fn reload(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let plugin_dir = match &state.config.plugin_dir {
        Some(d) => std::path::PathBuf::from(d).join(&id),
        None => return Err(AppError::not_found("plugin")),
    };
    if !plugin_dir.exists() {
        return Err(AppError::not_found("plugin"));
    }
    state.plugins.reload_plugin(&plugin_dir).await;
    Ok(ApiResponse::success(()))
}

/// DELETE /api/v1/admin/plugins/:id — 卸载插件
pub async fn remove(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.plugins.unload_plugin(&id).await;
    Ok(ApiResponse::success(()))
}
