//! 插件管理 API handler
//!
//! 提供运行时插件管理端点：列表、详情、启用、禁用、重载。

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::plugins::PluginInfoResponse;
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/admin/plugins", get(self::list), "system admin", "admin/plugins", ["GET"]);
    let r = reg_route!(r, registry, "/admin/plugins/{id}", get(self::get).delete(remove), "system admin", "admin/plugins", ["GET", "DELETE"]);
    let r = reg_route!(r, registry, "/admin/plugins/{id}/enable", http_post(enable), "system admin", "admin/plugins", ["POST"]);
    let r = reg_route!(r, registry, "/admin/plugins/{id}/disable", http_post(disable), "system admin", "admin/plugins", ["POST"]);
    let r = reg_route!(r, registry, "/admin/plugins/{id}/reload", http_post(reload), "system admin", "admin/plugins", ["POST"]);
    reg_route!(r, registry, "/admin/plugins/batch", http_post(admin_batch), "system admin", "admin/plugins", ["POST"])
}


/// GET /api/v1/admin/plugins — 列出所有插件及状态（分页）
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<PluginInfoResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let all = state.plugins.list_plugins_detail().await;
    Ok(params.paginate_in_memory(all))
}

/// GET /api/v1/admin/plugins/:id — 插件详情
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<PluginInfoResponse>> {
    auth.ensure_admin()?;
    let detail = state
        .plugins
        .get_plugin_detail(&id)
        .await
        .ok_or_else(|| AppError::not_found("plugin"))?;
    Ok(ApiResponse::success(detail))
}

/// POST /api/v1/admin/plugins/:id/enable — 启用插件
pub async fn enable(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.enable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/disable — 禁用插件
pub async fn disable(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.disable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/reload — 重载插件
pub async fn reload(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
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
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.unload_plugin(&id).await;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    axum::Json(req): axum::Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "enable" => {
                if state.plugins.enable_plugin(id).await.is_ok() {
                    affected += 1;
                }
            }
            "disable" => {
                if state.plugins.disable_plugin(id).await.is_ok() {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(&req.action, affected)))
}
