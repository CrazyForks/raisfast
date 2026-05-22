//! Plugin management API handler
//!
//! Provides runtime plugin management endpoints: list, details, enable, disable, reload.

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::plugins::PluginInfoResponse;
use crate::utils::pagination::PaginationParams;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins",
        get,
        self::list,
        "system admin",
        "admin/plugins"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/{id}",
        get,
        self::get,
        "system admin",
        "admin/plugins"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/{id}",
        delete,
        remove,
        "system admin",
        "admin/plugins"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/{id}/enable",
        post,
        enable,
        "system admin",
        "admin/plugins"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/{id}/disable",
        post,
        disable,
        "system admin",
        "admin/plugins"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/{id}/reload",
        post,
        reload,
        "system admin",
        "admin/plugins"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/plugins/batch",
        post,
        admin_batch,
        "system admin",
        "admin/plugins"
    )
}

/// GET /api/v1/admin/plugins — List all plugins and their status (paginated)
#[utoipa::path(get, path = "/admin/plugins", tag = "plugins",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Plugin list"))
)]
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

/// GET /api/v1/admin/plugins/:id — Plugin details
#[utoipa::path(get, path = "/admin/plugins/{id}", tag = "plugins",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Plugin ID")),
    responses((status = 200, description = "Plugin detail"))
)]
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

/// POST /api/v1/admin/plugins/:id/enable — Enable plugin
#[utoipa::path(post, path = "/admin/plugins/{id}/enable", tag = "plugins",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Plugin ID")),
    responses((status = 200, description = "Plugin enabled"))
)]
pub async fn enable(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.enable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/disable — Disable plugin
#[utoipa::path(post, path = "/admin/plugins/{id}/disable", tag = "plugins",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Plugin ID")),
    responses((status = 200, description = "Plugin disabled"))
)]
pub async fn disable(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.disable_plugin(&id).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/plugins/:id/reload — Reload plugin
#[utoipa::path(post, path = "/admin/plugins/{id}/reload", tag = "plugins",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Plugin ID")),
    responses((status = 200, description = "Plugin reloaded"))
)]
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

/// DELETE /api/v1/admin/plugins/:id — Uninstall plugin
#[utoipa::path(delete, path = "/admin/plugins/{id}", tag = "plugins",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Plugin ID")),
    responses((status = 200, description = "Plugin removed"))
)]
pub async fn remove(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state.plugins.unload_plugin(&id).await;
    Ok(ApiResponse::success(()))
}

#[utoipa::path(post, path = "/admin/plugins/batch", tag = "plugins",
    security(("bearer_auth" = [])),
    request_body = BatchRequest,
    responses((status = 200, description = "Batch operation completed"))
)]
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
            "enable" if state.plugins.enable_plugin(id).await.is_ok() => {
                affected += 1;
            }
            "disable" if state.plugins.disable_plugin(id).await.is_ok() => {
                affected += 1;
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
