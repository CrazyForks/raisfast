//! API Token management endpoints
//!
//! Provides HTTP handlers for creating, listing, and deleting API tokens.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;

use crate::AppState;
use crate::dto::CreateTokenRequest;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::services::api_token;

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
        "/tokens",
        get,
        self::list,
        "system authed",
        "tokens"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/tokens",
        create,
        self::create,
        "system authed",
        "tokens"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/tokens/{id}",
        delete,
        self::delete,
        "system authed",
        "tokens"
    )
}

/// Create API Token
///
/// `POST /api/v1/tokens`
#[utoipa::path(post, path = "/tokens", tag = "tokens",
    security(("bearer_auth" = [])),
    request_body = CreateTokenRequest,
    responses((status = 201, description = "Token created successfully"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTokenRequest>,
) -> AppResult<impl IntoResponse> {
    auth.ensure_authenticated()?;
    crate::errors::validation::validate(&body)?;
    let result = api_token::create_token(
        &state.pool,
        &auth,
        &body.name,
        body.scopes,
        body.expires_at.as_deref(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(ApiResponse::success(result))))
}

/// List current user's API Tokens
///
/// `GET /api/v1/tokens`
#[utoipa::path(get, path = "/tokens", tag = "tokens",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token list"))
)]
pub async fn list(auth: AuthUser, State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    auth.ensure_authenticated()?;
    let tokens = api_token::list_tokens(&state.pool, &auth).await?;
    Ok(Json(ApiResponse::success(tokens)))
}

/// Delete API Token
///
/// `DELETE /api/v1/tokens/:id`
#[utoipa::path(delete, path = "/tokens/{id}", tag = "tokens",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Token ID")),
    responses((status = 200, description = "Token deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    auth.ensure_authenticated()?;
    api_token::delete_token(&state.pool, &*state.cache, &id, &auth).await?;
    Ok(Json(ApiResponse::success(
        serde_json::json!({"deleted": true}),
    )))
}
