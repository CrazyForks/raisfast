use axum::Json;
use axum::extract::{Path, State};
use axum::routing::{get, post as http_post, put};

use crate::dto::currencies::{CreateCurrencyRequest, CurrencyResponse, UpdateCurrencyRequest};
use crate::errors::app_error::AppError;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::currencies;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    let r = axum::Router::new();
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/currencies",
        get(list_currencies),
        "admin currencies",
        "admin/currencies",
        ["GET"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/currencies",
        http_post(create_currency),
        "admin currencies",
        "admin/currencies",
        ["POST"]
    );
    let r = crate::reg_route!(
        r,
        registry,
        "/admin/currencies/{code}",
        get(get_currency),
        "admin currencies",
        "admin/currencies",
        ["GET"]
    );
    crate::reg_route!(
        r,
        registry,
        "/admin/currencies/{code}",
        put(update_currency),
        "admin currencies",
        "admin/currencies",
        ["PUT"]
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
    let rows = currencies::find_all(&state.pool).await?;
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
    let c = currencies::find_by_code(&state.pool, &code)
        .await?
        .ok_or_else(|| AppError::not_found("currency"))?;
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
    let decimals = req.decimals.unwrap_or(2);
    if !(0..=18).contains(&decimals) {
        return Err(AppError::BadRequest(
            "decimals must be between 0 and 18".into(),
        ));
    }
    let c = currencies::create(&state.pool, &req.code, &req.name, decimals).await?;
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
    let c = currencies::update(&state.pool, &code, req.name.as_deref(), req.is_active)
        .await?
        .ok_or_else(|| AppError::not_found("currency"))?;
    Ok(ApiResponse::success(CurrencyResponse::from(c)))
}
