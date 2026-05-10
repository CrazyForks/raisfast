//! OAuth2 社交登录处理器
//!
//! 提供 OAuth 授权发起、回调处理、绑定/解绑的 HTTP 端点。

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect};
use serde::Deserialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::services::oauth;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{delete, get};

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/auth/oauth/{provider}", get(redirect_to_provider), "system public", "oauth", ["GET"]);
    let r = reg_route!(r, registry, "/auth/oauth/{provider}/callback", get(callback), "system public", "oauth", ["GET"]);
    let r = reg_route!(r, registry, "/auth/oauth/providers", get(list_providers), "system public", "oauth", ["GET"]);
    let r = reg_route!(r, registry, "/auth/oauth/bindings", get(list_bindings), "system public", "oauth", ["GET"]);
    reg_route!(r, registry, "/auth/oauth/{provider}/unbind", delete(unbind), "system public", "oauth", ["DELETE"])
}


/// 发起 OAuth 登录 — 302 重定向到 Provider 授权页
///
/// `GET /api/v1/auth/oauth/{provider}`
pub async fn redirect_to_provider(
    State(state): State<crate::AppState>,
    Path(provider): Path<String>,
    auth: AuthUser,
) -> AppResult<impl IntoResponse> {
    if !state.config.oauth.enabled {
        tracing::warn!(
            "OAuth not enabled, oauth.enabled={}",
            state.config.oauth.enabled
        );
        return Err(AppError::BadRequest("OAuth is not enabled".into()));
    }

    if !state.config.oauth.is_provider_configured(&provider) {
        tracing::warn!(
            "OAuth provider '{}' not configured, github={}, google={}, wechat={}",
            provider,
            state.config.oauth.github.is_some(),
            state.config.oauth.google.is_some(),
            state.config.oauth.wechat.is_some(),
        );
        return Err(AppError::BadRequest(format!(
            "OAuth provider '{provider}' is not configured"
        )));
    }

    let url =
        oauth::initiate_oauth(&state.pool, state.oauth_registry.as_ref(), &provider, &auth).await?;

    Ok(Redirect::temporary(&url))
}

/// OAuth 回调查询参数
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: String,
    pub state: String,
}

/// OAuth 回调处理
///
/// `GET /api/v1/auth/oauth/{provider}/callback?code=xxx&state=xxx`
pub async fn callback(
    State(state): State<crate::AppState>,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> AppResult<impl IntoResponse> {
    if !state.config.oauth.enabled {
        return Err(AppError::BadRequest("OAuth is not enabled".into()));
    }

    let result = oauth::handle_callback(
        &state.pool,
        state.oauth_registry.as_ref(),
        state.user_repo.as_ref(),
        state.refresh_token_repo.as_ref(),
        &provider,
        &query.code,
        &query.state,
        &state.config.jwt_secret,
        state.config.jwt_access_expires,
        state.config.jwt_refresh_expires,
        &state.eventbus,
    )
    .await?;

    match result {
        oauth::OAuthCallbackResult::LoginSuccess(resp) => {
            let redirect_url = format!(
                "{}?access_token={}&refresh_token={}&expires_in={}",
                state.config.oauth.redirect_url,
                resp.access_token,
                resp.refresh_token,
                resp.expires_in,
            );
            Ok(Redirect::temporary(&redirect_url).into_response())
        }
        oauth::OAuthCallbackResult::BindingRequired { .. } => Err(AppError::BadRequest(
            "binding required but not yet implemented".into(),
        )),
    }
}

/// 获取已配置的 OAuth Provider 列表
///
/// `GET /api/v1/auth/oauth/providers`
pub async fn list_providers(
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<Vec<ProviderInfo>>> {
    let providers = state
        .config
        .oauth
        .configured_providers()
        .into_iter()
        .map(|name| ProviderInfo {
            name: name.to_string(),
            configured: true,
        })
        .collect();
    Ok(ApiResponse::success(providers))
}

/// 获取当前用户的 OAuth 绑定列表
///
/// `GET /api/v1/auth/oauth/bindings`
pub async fn list_bindings(
    State(state): State<crate::AppState>,
    auth: AuthUser,
) -> AppResult<ApiResponse<Vec<oauth::OAuthBindingInfo>>> {
    let bindings = oauth::list_bindings(&state.pool, &auth).await?;
    Ok(ApiResponse::success(bindings))
}

/// 解绑 OAuth 账号
///
/// `DELETE /api/v1/auth/oauth/{provider}/unbind`
pub async fn unbind(
    State(state): State<crate::AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
) -> AppResult<ApiResponse<()>> {
    oauth::unbind_oauth(&state.pool, &auth, &provider).await?;
    Ok(ApiResponse::success(()))
}

/// Provider 信息
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, serde::Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub configured: bool,
}
