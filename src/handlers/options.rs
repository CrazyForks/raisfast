use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use serde_json::Value;

use crate::AppState;
use crate::dto::{UpdateOptionRequest, UpdateOptionsRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;

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
        "/options/public",
        get,
        get_public_options,
        "system public",
        "options"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/options",
        get,
        list_options,
        "system admin",
        "admin/options"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/options",
        put,
        update_options,
        "system admin",
        "admin/options"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/options/{key}",
        get,
        get_option,
        "system admin",
        "admin/options"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/options/{key}",
        put,
        set_option,
        "system admin",
        "admin/options"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/options/{key}",
        delete,
        delete_option,
        "system admin",
        "admin/options"
    )
}

#[utoipa::path(get, path = "/options/public", tag = "options",
    responses((status = 200, description = "Public options"))
)]
pub async fn get_public_options(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<HashMap<String, Value>>> {
    let mut options = state.options.get_public(auth.tenant_id()).await;
    options.insert(
        "builtin_tenantable".into(),
        Value::Bool(state.config.builtin_tenantable),
    );
    Ok(ApiResponse::success(options))
}

#[utoipa::path(get, path = "/admin/options", tag = "options",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "All options grouped"))
)]
pub async fn list_options(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    let groups = state.options.get_grouped(auth.tenant_id()).await?;
    Ok(ApiResponse::success(groups))
}

#[utoipa::path(get, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option value"))
)]
pub async fn get_option(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let entry = state
        .options
        .get_entry(auth.tenant_id(), &key)
        .await
        .ok_or_else(|| AppError::not_found(&format!("option/{key}")))?;
    Ok(ApiResponse::success(
        serde_json::to_value(entry).map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?,
    ))
}

#[utoipa::path(put, path = "/admin/options", tag = "options",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Options updated"))
)]
pub async fn update_options(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateOptionsRequest>,
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    state
        .options
        .set_batch(auth.tenant_id(), body.options)
        .await?;
    let groups = state.options.get_grouped(auth.tenant_id()).await?;
    Ok(ApiResponse::success(groups))
}

#[utoipa::path(put, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option set"))
)]
pub async fn set_option(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<UpdateOptionRequest>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state
        .options
        .set(auth.tenant_id(), &key, body.value)
        .await?;
    Ok(ApiResponse::success(serde_json::json!({
        "option_key": key,
        "updated": true,
    })))
}

#[utoipa::path(delete, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option deleted"))
)]
pub async fn delete_option(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.options.delete(auth.tenant_id(), &key).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "option_key": key,
        "deleted": true,
    })))
}
