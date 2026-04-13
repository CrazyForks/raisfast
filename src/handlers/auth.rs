//! 认证相关处理器
//!
//! 处理用户注册、登录、令牌刷新和登出请求。
//! 所有函数均为薄层，仅做参数提取、请求验证和 service 调用。

use axum::Json;
use axum::extract::State;

use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::models::user::{LoginRequest, RefreshRequest, RegisterRequest};
use crate::services::auth;

/// 用户注册
///
/// - **方法/路径：** `POST /api/auth/register`
/// - **认证：** 无需认证
/// - **说明：** 验证请求参数后调用 `auth::register` 创建新用户，返回用户公开信息。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<UserResponse>`
pub async fn register(
    State(state): State<crate::AppState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<ApiResponse<crate::models::user::UserResponse>> {
    validation::validate(&req)?;
    let user = auth::register(&state.pool, req).await?;
    Ok(ApiResponse::success(user))
}

/// 用户登录
///
/// - **方法/路径：** `POST /api/auth/login`
/// - **认证：** 无需认证
/// - **说明：** 验证邮箱和密码，签发 JWT 访问令牌和刷新令牌。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<LoginResponse>`
pub async fn login(
    State(state): State<crate::AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<ApiResponse<crate::models::user::LoginResponse>> {
    validation::validate(&req)?;
    let resp = auth::login(
        &state.pool,
        &state.plugins,
        &req,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 刷新访问令牌
///
/// - **方法/路径：** `POST /api/auth/refresh`
/// - **认证：** 无需认证（使用刷新令牌）
/// - **说明：** 验证刷新令牌有效性，签发新的访问令牌和刷新令牌。
/// - **返回：** `ApiResponse<LoginResponse>`
pub async fn refresh(
    State(state): State<crate::AppState>,
    Json(req): Json<RefreshRequest>,
) -> AppResult<ApiResponse<crate::models::user::LoginResponse>> {
    let resp = auth::refresh(
        &state.pool,
        &req.refresh_token,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 用户登出
///
/// - **方法/路径：** `POST /api/auth/logout`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 删除该用户的所有刷新令牌，使已签发的刷新令牌失效。
/// - **返回：** `ApiResponse<()>`
pub async fn logout(
    State(state): State<crate::AppState>,
    auth_user: crate::middleware::auth::AuthUser,
) -> AppResult<ApiResponse<()>> {
    auth::logout(&state.pool, &auth_user.user_id).await?;
    Ok(ApiResponse::success(()))
}
