//! Site options API handler

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
    let r = reg_route!(
        r,
        registry,
        "/options/public",
        get(get_public_options),
        "system public",
        "options",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/options",
        get(list_options).put(update_options),
        "system admin",
        "admin/options",
        ["GET", "PUT"]
    );
    reg_route!(
        r,
        registry,
        "/admin/options/{key}",
        get(get_option).put(set_option).delete(delete_option),
        "system admin",
        "admin/options",
        ["GET", "PUT", "DELETE"]
    )
}

/// GET /options/public — Public options (values only) + system feature flags
#[utoipa::path(get, path = "/options/public", tag = "options",
    responses((status = 200, description = "Public options"))
)]
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

/// GET /admin/options — All options (grouped, with metadata)
#[utoipa::path(get, path = "/admin/options", tag = "options",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "All options grouped"))
)]
pub async fn list_options(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    let groups = state.options.get_grouped().await?;
    Ok(ApiResponse::success(groups))
}

/// GET /admin/options/:key — Get a single option
#[utoipa::path(get, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option value"))
)]
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

/// Batch update request body
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateOptionsRequest {
    pub options: HashMap<String, Value>,
}

/// PUT /admin/options — Batch update options
#[utoipa::path(put, path = "/admin/options", tag = "options",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Options updated"))
)]
pub async fn update_options(
    State(state): State<AppState>,
    Json(body): Json<UpdateOptionsRequest>,
) -> AppResult<ApiResponse<Vec<crate::services::options::OptionGroup>>> {
    state.options.set_batch(body.options).await?;
    let groups = state.options.get_grouped().await?;
    Ok(ApiResponse::success(groups))
}

/// Update single option request body
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateOptionRequest {
    pub value: Value,
}

/// PUT /admin/options/:key — Set a single option
#[utoipa::path(put, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option set"))
)]
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

/// DELETE /admin/options/:key — Delete an option
#[utoipa::path(delete, path = "/admin/options/{key}", tag = "options",
    security(("bearer_auth" = [])),
    params(("key" = String, Path, description = "Option key")),
    responses((status = 200, description = "Option deleted"))
)]
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
