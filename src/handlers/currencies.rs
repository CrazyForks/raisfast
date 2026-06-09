use axum::Json;
use axum::extract::{Path, State};

use crate::dto::currencies::{CreateCurrencyRequest, CurrencyResponse, UpdateCurrencyRequest};
use crate::errors::app_error::AppError;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::currencies as svc;

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
        "/admin/currencies",
        get,
        list_currencies,
        "system admin",
        "admin/currencies"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/currencies",
        create,
        create_currency,
        "system admin",
        "admin/currencies"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/currencies/{code}",
        get,
        get_currency,
        "system admin",
        "admin/currencies"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/currencies/{code}",
        put,
        update_currency,
        "system admin",
        "admin/currencies"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/currencies/{code}",
        delete,
        delete_currency,
        "system admin",
        "admin/currencies"
    )
}

#[utoipa::path(get, path = "/admin/currencies", tag = "currencies",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List currencies"))
)]
pub async fn list_currencies(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> Result<ApiResponse<Vec<CurrencyResponse>>, AppError> {
    auth.ensure_admin()?;
    let rows = svc::list(&state.pool, auth.tenant_id()).await?;
    Ok(ApiResponse::success(
        rows.into_iter().map(CurrencyResponse::from).collect(),
    ))
}

#[utoipa::path(get, path = "/admin/currencies/{code}", tag = "currencies",
    security(("bearer_auth" = [])),
    params(("code" = String, Path, description = "Currency code")),
    responses((status = 200, description = "Currency detail"))
)]
pub async fn get_currency(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(code): Path<String>,
) -> Result<ApiResponse<CurrencyResponse>, AppError> {
    auth.ensure_admin()?;
    let c = svc::get_by_code(&state.pool, &code, auth.tenant_id()).await?;
    Ok(ApiResponse::success(CurrencyResponse::from(c)))
}

#[utoipa::path(post, path = "/admin/currencies", tag = "currencies",
    security(("bearer_auth" = [])),
    request_body = CreateCurrencyRequest,
    responses((status = 200, description = "Currency created"))
)]
pub async fn create_currency(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<CreateCurrencyRequest>,
) -> Result<ApiResponse<CurrencyResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let tenant_id = auth.tenant_id().unwrap_or(crate::constants::DEFAULT_TENANT);
    let decimals = req.decimals.unwrap_or(2);
    let c = svc::create(&state.pool, tenant_id, &req.code, &req.name, decimals).await?;
    Ok(ApiResponse::success(CurrencyResponse::from(c)))
}

#[utoipa::path(put, path = "/admin/currencies/{code}", tag = "currencies",
    security(("bearer_auth" = [])),
    params(("code" = String, Path, description = "Currency code")),
    request_body = UpdateCurrencyRequest,
    responses((status = 200, description = "Currency updated"))
)]
pub async fn update_currency(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(code): Path<String>,
    Json(req): Json<UpdateCurrencyRequest>,
) -> Result<ApiResponse<CurrencyResponse>, AppError> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    svc::update(
        &state.pool,
        &code,
        req.name.as_deref(),
        req.is_active,
        auth.tenant_id(),
    )
    .await?;
    let c = svc::get_by_code(&state.pool, &code, auth.tenant_id()).await?;
    Ok(ApiResponse::success(CurrencyResponse::from(c)))
}

#[utoipa::path(delete, path = "/admin/currencies/{code}", tag = "currencies",
    security(("bearer_auth" = [])),
    params(("code" = String, Path, description = "Currency code")),
    responses((status = 200, description = "Currency deleted"))
)]
pub async fn delete_currency(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(code): Path<String>,
) -> Result<ApiResponse<serde_json::Value>, AppError> {
    auth.ensure_admin()?;
    svc::delete(&state.pool, &code, auth.tenant_id()).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "code": code,
        "deleted": true,
    })))
}
