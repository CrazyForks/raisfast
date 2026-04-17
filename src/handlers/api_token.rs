//! API Token 管理端点
//!
//! 提供创建、列表、删除 API Token 的 HTTP handler。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use validator::Validate;

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::middleware::auth::AuthUser;
use crate::services::api_token;

/// 创建 API Token 请求体
#[derive(Debug, Deserialize, Validate, utoipa::ToSchema)]
pub struct CreateTokenRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    #[validate(length(min = 1))]
    pub scopes: Vec<String>,
    pub expires_at: Option<String>,
}

/// 创建 API Token
///
/// `POST /api/v1/tokens`
#[utoipa::path(post, path = "/tokens", tag = "tokens",
    security(("bearer_auth" = [])),
    request_body = CreateTokenRequest,
    responses((status = 201, description = "Token 创建成功"))
)]
pub async fn create(
    user: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTokenRequest>,
) -> AppResult<impl IntoResponse> {
    crate::errors::validation::validate(&body)?;
    let result = api_token::create_token(
        &state.pool,
        &user.user_id,
        &body.name,
        body.scopes,
        body.expires_at.as_deref(),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "code": 20100,
            "message": "created",
            "data": result
        })),
    ))
}

/// 列出当前用户的 API Token
///
/// `GET /api/v1/tokens`
#[utoipa::path(get, path = "/tokens", tag = "tokens",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token 列表"))
)]
pub async fn list(user: AuthUser, State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    let tokens = api_token::list_tokens(&state.pool, &user.user_id).await?;
    Ok(Json(serde_json::json!({
        "code": 20000,
        "message": "ok",
        "data": tokens
    })))
}

/// 删除 API Token
///
/// `DELETE /api/v1/tokens/:id`
#[utoipa::path(delete, path = "/tokens/{id}", tag = "tokens",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Token ID")),
    responses((status = 200, description = "Token 已删除"))
)]
pub async fn delete(
    user: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let is_admin = user.role == "admin";
    api_token::delete_token(&state.pool, &id, &user.user_id, is_admin).await?;
    Ok(Json(serde_json::json!({
        "code": 20000,
        "message": "deleted"
    })))
}
