//! 认证相关处理器
//!
//! 处理用户注册、登录、令牌刷新和登出请求。
//! 所有函数均为薄层，仅做参数提取、请求验证和 service 调用。

use axum::Json;
use axum::extract::State;

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::handlers::dto::{LoginRequest, RefreshRequest, RegisterRequest};
use crate::middleware::tenant::ResolvedTenant;
use crate::services::auth;

/// 用户注册
pub async fn register(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Json(req): Json<RegisterRequest>,
) -> AppResult<ApiResponse<crate::handlers::dto::UserResponse>> {
    validation::validate(&req)?;
    let user = auth::register(
        state.user_repo.as_ref(),
        &state.eventbus,
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(user))
}

/// 用户登录
pub async fn login(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Json(req): Json<LoginRequest>,
) -> AppResult<ApiResponse<crate::handlers::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = auth::login(
        state.user_repo.as_ref(),
        state.refresh_token_repo.as_ref(),
        &state.plugins,
        &state.eventbus,
        &req,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 刷新访问令牌
pub async fn refresh(
    State(state): State<crate::AppState>,
    Json(req): Json<RefreshRequest>,
) -> AppResult<ApiResponse<crate::handlers::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = auth::refresh(
        state.user_repo.as_ref(),
        state.refresh_token_repo.as_ref(),
        &state.pool,
        &req.refresh_token,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
        None,
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 用户登出
pub async fn logout(
    State(state): State<crate::AppState>,
    auth_user: crate::middleware::auth::AuthUser,
) -> AppResult<ApiResponse<()>> {
    auth::logout(state.refresh_token_repo.as_ref(), &auth_user.user_id).await?;
    Ok(ApiResponse::success(()))
}
