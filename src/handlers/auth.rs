//! 认证相关处理器
//!
//! 处理用户注册、登录、令牌刷新和登出请求。
//! 所有函数均为薄层，仅做参数提取、请求验证和 service 调用。

use axum::Json;
use axum::extract::State;

use crate::dto::{
    AuthConfigResponse, BindPhoneRequest, ForgotPasswordRequest, LoginRequest, RefreshRequest,
    RegisterRequest, ResendVerificationRequest, ResetPasswordRequest, SendSmsCodeRequest,
    SetPasswordRequest, VerifyEmailRequest, VerifySmsRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::{auth, email_verification, password_reset, sms};

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use crate::middleware::rate_limit::{login_rate_limit, register_rate_limit};
    use axum::middleware::from_fn;
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/auth/register",
        http_post(register).layer(from_fn(register_rate_limit)),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/login",
        http_post(login).layer(from_fn(login_rate_limit)),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/refresh",
        http_post(refresh),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/logout",
        http_post(logout),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/forgot-password",
        http_post(forgot_password),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/reset-password",
        http_post(reset_password),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/set-password",
        http_post(set_password),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/config",
        get(auth_config),
        "system public",
        "auth",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/sms/send",
        http_post(send_sms_code),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/sms/verify",
        http_post(verify_sms),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/phone/bind",
        http_post(bind_phone),
        "system public",
        "auth",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/auth/verify-email",
        http_post(verify_email),
        "system public",
        "auth",
        ["POST"]
    );
    reg_route!(
        r,
        registry,
        "/auth/resend-verification",
        http_post(resend_verification),
        "system public",
        "auth",
        ["POST"]
    )
}

/// 用户注册
#[utoipa::path(post, path = "/auth/register", tag = "auth",
    request_body = RegisterRequest,
    responses((status = 200, description = "注册成功"))
)]
pub async fn register(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<RegisterRequest>,
) -> AppResult<ApiResponse<crate::dto::UserResponse>> {
    if !state.config.registration_email_enabled {
        return Err(crate::errors::app_error::AppError::BadRequest(
            "email_registration_disabled".into(),
        ));
    }
    validation::validate(&req)?;
    let user = auth::register(
        state.user_repo.as_ref(),
        &state.eventbus,
        req,
        auth.tenant_id(),
        state.config.require_email_verification,
        &state.pool,
    )
    .await?;
    Ok(ApiResponse::success(user))
}

/// 用户登录
#[utoipa::path(post, path = "/auth/login", tag = "auth",
    request_body = LoginRequest,
    responses((status = 200, description = "登录成功"))
)]
pub async fn login(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
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
        auth.tenant_id(),
        state.config.require_email_verification,
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 验证邮箱
pub async fn verify_email(
    State(state): State<crate::AppState>,
    Json(req): Json<VerifyEmailRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    email_verification::verify_email(&state.pool, &req.token).await?;
    Ok(ApiResponse::success(()))
}

/// 重新发送验证邮件
pub async fn resend_verification(
    State(state): State<crate::AppState>,
    Json(req): Json<ResendVerificationRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    email_verification::resend_verification(
        &state.pool,
        state.user_repo.as_ref(),
        &state.eventbus,
        &req.email,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 刷新访问令牌
#[utoipa::path(post, path = "/auth/refresh", tag = "auth",
    request_body = RefreshRequest,
    responses((status = 200, description = "令牌刷新成功"))
)]
pub async fn refresh(
    State(state): State<crate::AppState>,
    Json(req): Json<RefreshRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
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
#[utoipa::path(post, path = "/auth/logout", tag = "auth",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "登出成功"))
)]
pub async fn logout(
    State(state): State<crate::AppState>,
    auth: AuthUser,
) -> AppResult<ApiResponse<()>> {
    auth::logout(&state.pool, state.refresh_token_repo.as_ref(), &auth).await?;
    Ok(ApiResponse::success(()))
}

/// 请求密码重置
#[utoipa::path(post, path = "/auth/forgot-password", tag = "auth",
    request_body = ForgotPasswordRequest,
    responses((status = 200, description = "重置邮件已发送"))
)]
pub async fn forgot_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::forgot_password(
        &state.pool,
        state.user_repo.as_ref(),
        &state.eventbus,
        &req.email,
        auth.tenant_id(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 重置密码
#[utoipa::path(post, path = "/auth/reset-password", tag = "auth",
    request_body = ResetPasswordRequest,
    responses((status = 200, description = "密码已重置"))
)]
pub async fn reset_password(
    State(state): State<crate::AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::reset_password(
        state.user_repo.as_ref(),
        &state.pool,
        &req.token,
        &req.new_password,
        None,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// OAuth 用户设置密码
#[utoipa::path(post, path = "/auth/set-password", tag = "auth",
    security(("bearer_auth" = [])),
    request_body = SetPasswordRequest,
    responses((status = 200, description = "密码已设置"))
)]
pub async fn set_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::set_password(
        state.user_repo.as_ref(),
        &state.pool,
        &auth,
        &req.new_password,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 获取认证配置（支持的注册方式等）
pub async fn auth_config(
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<AuthConfigResponse>> {
    let oauth_providers = if state.config.oauth.enabled {
        state
            .oauth_registry
            .provider_names()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![]
    };
    Ok(ApiResponse::success(AuthConfigResponse {
        registration_email_enabled: state.config.registration_email_enabled,
        registration_sms_enabled: state.config.registration_sms_enabled,
        oauth_providers,
        require_email_verification: state.config.require_email_verification,
    }))
}

/// 发送短信验证码
pub async fn send_sms_code(
    State(state): State<crate::AppState>,
    Json(req): Json<SendSmsCodeRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    sms::send_sms_code(&state.pool, &state.config, &req.phone, &req.purpose).await?;
    Ok(ApiResponse::success(()))
}

/// 验证短信验证码（自动注册/登录）
pub async fn verify_sms(
    State(state): State<crate::AppState>,
    Json(req): Json<VerifySmsRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = sms::verify_sms_and_auth(
        state.user_repo.as_ref(),
        state.refresh_token_repo.as_ref(),
        &state.pool,
        &req.phone,
        &req.code,
        &req.purpose,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
    )
    .await?;
    Ok(ApiResponse::success(resp))
}

/// 绑定手机号
pub async fn bind_phone(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BindPhoneRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    sms::bind_phone(
        state.user_repo.as_ref(),
        &state.pool,
        &auth,
        &req.phone,
        &req.code,
    )
    .await?;
    Ok(ApiResponse::success(()))
}
