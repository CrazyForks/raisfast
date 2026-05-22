//! Authentication handlers
//!
//! Handles user registration, login, token refresh, and logout requests.
//! All functions are thin layers — only parameter extraction, request validation, and service calls.

use axum::Json;
use axum::extract::State;

use crate::dto::{
    AuthConfigResponse, BindEmailRequest, BindPhoneRequest, CredentialResponse,
    ForgotPasswordRequest, LoginRequest, RefreshRequest, RegisterRequest,
    ResendVerificationRequest, ResetPasswordRequest, SendSmsCodeRequest, SetPasswordRequest,
    VerifyEmailRequest, VerifySmsRequest,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::{auth, email_verification, password_reset, sms};
use crate::types::snowflake_id::SnowflakeId;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    use crate::middleware::rate_limit::{login_rate_limit, register_rate_limit};
    use axum::middleware::from_fn;

    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = {
        let mr = axum::routing::post(register).layer(from_fn(register_rate_limit));
        r.route("/auth/register", mr)
    };
    registry.record("POST", "/api/v1/auth/register", "system public", "auth");
    let r = {
        let mr = axum::routing::post(login).layer(from_fn(login_rate_limit));
        r.route("/auth/login", mr)
    };
    registry.record("POST", "/api/v1/auth/login", "system public", "auth");
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/refresh",
        post,
        refresh,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/logout",
        post,
        logout,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/forgot-password",
        post,
        forgot_password,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/reset-password",
        post,
        reset_password,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/set-password",
        post,
        set_password,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/config",
        get,
        auth_config,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/sms/send",
        post,
        send_sms_code,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/sms/verify",
        post,
        verify_sms,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/phone/bind",
        post,
        bind_phone,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/verify-email",
        post,
        verify_email,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/resend-verification",
        post,
        resend_verification,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/credentials/bind-email",
        post,
        bind_email_credential,
        "system public",
        "auth"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/auth/credentials",
        get,
        list_credentials,
        "system public",
        "auth"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/auth/credentials/{id}",
        delete,
        delete_credential,
        "system public",
        "auth"
    )
}

/// User registration
#[utoipa::path(post, path = "/auth/register", tag = "auth",
    request_body = RegisterRequest,
    responses((status = 200, description = "Registration successful"))
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
        &state.aspect_engine,
        req,
        auth.tenant_id(),
        state.config.require_email_verification,
        &state.pool,
    )
    .await?;
    Ok(ApiResponse::success(user))
}

/// User login
#[utoipa::path(post, path = "/auth/login", tag = "auth",
    request_body = LoginRequest,
    responses((status = 200, description = "Login successful"))
)]
pub async fn login(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<LoginRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = auth::login(
        &state.aspect_engine,
        &state.pool,
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

/// Verify email
pub async fn verify_email(
    State(state): State<crate::AppState>,
    Json(req): Json<VerifyEmailRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    email_verification::verify_email(&state.pool, &req.token).await?;
    Ok(ApiResponse::success(()))
}

/// Resend verification email
pub async fn resend_verification(
    State(state): State<crate::AppState>,
    Json(req): Json<ResendVerificationRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    email_verification::resend_verification(&state.pool, &state.aspect_engine, &req.email).await?;
    Ok(ApiResponse::success(()))
}

/// Refresh access token
#[utoipa::path(post, path = "/auth/refresh", tag = "auth",
    request_body = RefreshRequest,
    responses((status = 200, description = "Token refreshed successfully"))
)]
pub async fn refresh(
    State(state): State<crate::AppState>,
    Json(req): Json<RefreshRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = auth::refresh(
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

/// User logout
#[utoipa::path(post, path = "/auth/logout", tag = "auth",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Logout successful"))
)]
pub async fn logout(
    State(state): State<crate::AppState>,
    auth: AuthUser,
) -> AppResult<ApiResponse<()>> {
    auth::logout(&state.pool, &auth).await?;
    Ok(ApiResponse::success(()))
}

/// Request password reset
#[utoipa::path(post, path = "/auth/forgot-password", tag = "auth",
    request_body = ForgotPasswordRequest,
    responses((status = 200, description = "Reset email sent"))
)]
pub async fn forgot_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<ForgotPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::forgot_password(
        &state.pool,
        &state.aspect_engine,
        &req.email,
        auth.tenant_id(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// Reset password
#[utoipa::path(post, path = "/auth/reset-password", tag = "auth",
    request_body = ResetPasswordRequest,
    responses((status = 200, description = "Password reset"))
)]
pub async fn reset_password(
    State(state): State<crate::AppState>,
    Json(req): Json<ResetPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::reset_password(&state.pool, &req.token, &req.new_password, None).await?;
    Ok(ApiResponse::success(()))
}

/// Set password for OAuth user
#[utoipa::path(post, path = "/auth/set-password", tag = "auth",
    security(("bearer_auth" = [])),
    request_body = SetPasswordRequest,
    responses((status = 200, description = "Password set"))
)]
pub async fn set_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<SetPasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    password_reset::set_password(&state.pool, &auth, &req.email, &req.new_password).await?;
    Ok(ApiResponse::success(()))
}

/// Get authentication config (supported registration methods, etc.)
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

/// Send SMS verification code
pub async fn send_sms_code(
    State(state): State<crate::AppState>,
    Json(req): Json<SendSmsCodeRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    sms::send_sms_code(&state.pool, &state.config, &req.phone, &req.purpose).await?;
    Ok(ApiResponse::success(()))
}

/// Verify SMS code (auto register/login)
pub async fn verify_sms(
    State(state): State<crate::AppState>,
    Json(req): Json<VerifySmsRequest>,
) -> AppResult<ApiResponse<crate::dto::LoginResponse>> {
    validation::validate(&req)?;
    let resp = sms::verify_sms_and_auth(
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

/// Bind phone number
pub async fn bind_phone(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BindPhoneRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_authenticated()?;
    validation::validate(&req)?;
    sms::bind_phone(&state.pool, &auth, &req.phone, &req.code).await?;
    Ok(ApiResponse::success(()))
}

/// Bind email/password credential
pub async fn bind_email_credential(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BindEmailRequest>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_authenticated()?;
    validation::validate(&req)?;
    auth::bind_email_credential(&state.pool, &auth, &req.email, &req.password).await?;
    Ok(ApiResponse::success(()))
}

/// List all credentials for the current user
pub async fn list_credentials(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<CredentialResponse>>> {
    auth.ensure_authenticated()?;
    let creds = auth::list_credentials(&state.pool, &auth).await?;
    let responses: AppResult<Vec<CredentialResponse>> = creds
        .into_iter()
        .map(CredentialResponse::from_credential)
        .collect();
    Ok(ApiResponse::success(responses?))
}

/// Delete a specific credential
pub async fn delete_credential(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_authenticated()?;
    auth::delete_credential(&state.pool, &auth, SnowflakeId(id)).await?;
    Ok(ApiResponse::success(()))
}
