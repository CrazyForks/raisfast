//! OAuth2 社交登录业务逻辑
//!
//! 处理 OAuth 授权流程的核心业务：
//! - 发起授权（生成 state + PKCE verifier）
//! - 处理回调（code exchange → 用户信息 → 查找/创建/绑定用户 → 签发 JWT）
//! - 绑定/解绑 OAuth 账号

use chrono::Utc;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::CreateUserCmd;
use crate::errors::app_error::{AppError, AppResult};
use crate::handlers::dto::LoginResponse;
use crate::middleware::auth::AuthUser;
use crate::models::oauth;
use crate::oauth::{OAuthProviderRegistry, OAuthUserInfo};
use crate::repositories::{RefreshTokenRepository, UserRepository};

/// 发起 OAuth 授权
///
/// 生成 state 和 PKCE code_verifier，存入数据库，返回 Provider 授权 URL。
pub async fn initiate_oauth(
    pool: &crate::db::Pool,
    registry: &OAuthProviderRegistry,
    provider_name: &str,
    auth: &AuthUser,
) -> AppResult<String> {
    let current_user_id = auth.user_id();
    let provider = registry.get(provider_name).ok_or_else(|| {
        AppError::BadRequest(format!("unsupported OAuth provider: {provider_name}"))
    })?;

    let state = crate::oauth::generate_state();
    let code_verifier = crate::oauth::generate_code_verifier();
    let code_challenge = crate::oauth::generate_code_challenge(&code_verifier);

    let expires_at = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    oauth::create_state(
        pool,
        &state,
        provider_name,
        &code_verifier,
        current_user_id,
        &expires_at,
    )
    .await?;

    Ok(provider.authorize_url(&state, &code_challenge))
}

/// OAuth 回调处理结果
pub enum OAuthCallbackResult {
    /// 登录成功（自动重定向到前端）
    LoginSuccess(LoginResponse),
    /// 需要绑定到已有账号（返回待绑定信息）
    BindingRequired {
        state: String,
        provider: String,
        user_info: OAuthUserInfo,
    },
}

/// 处理 OAuth 回调
///
/// 1. 校验 state，获取 code_verifier
/// 2. 用 code 换 access_token
/// 3. 获取 Provider 用户信息
/// 4. 查找已有绑定 → 直接签发 JWT
/// 5. 若无绑定且有绑定用户 ID → 执行绑定
/// 6. 若无绑定 → 自动注册新用户
#[allow(clippy::too_many_arguments)]
pub async fn handle_callback(
    pool: &crate::db::Pool,
    registry: &OAuthProviderRegistry,
    user_repo: &dyn UserRepository,
    refresh_token_repo: &dyn RefreshTokenRepository,
    provider_name: &str,
    code: &str,
    state: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    eventbus: &crate::eventbus::EventBus,
) -> AppResult<OAuthCallbackResult> {
    let provider = registry.get(provider_name).ok_or_else(|| {
        AppError::BadRequest(format!("unsupported OAuth provider: {provider_name}"))
    })?;

    let oauth_state = oauth::consume_state(pool, state)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired OAuth state".into()))?;

    if oauth_state.provider != provider_name {
        return Err(AppError::BadRequest(
            "provider mismatch in OAuth state".into(),
        ));
    }

    let token_resp = provider
        .exchange_code(code, &oauth_state.code_verifier)
        .await?;
    let user_info = provider.fetch_user_info(&token_resp.access_token).await?;

    let existing =
        oauth::find_by_provider_user(pool, provider_name, &user_info.provider_user_id).await?;

    if let Some(account) = existing {
        let user = user_repo
            .find_by_id(&account.user_id, None)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("OAuth bound user not found")))?;

        let login_resp = create_login_response_for_user(
            &user,
            refresh_token_repo,
            jwt_secret,
            jwt_access_expires,
            jwt_refresh_expires,
        )
        .await?;

        update_oauth_account(pool, &account.id, &token_resp, &user_info).await?;

        return Ok(OAuthCallbackResult::LoginSuccess(login_resp));
    }

    if let Some(bind_user_id) = oauth_state.user_id {
        do_bind_oauth(pool, &bind_user_id, provider_name, &token_resp, &user_info).await?;

        let user = user_repo
            .find_by_id(&bind_user_id, None)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;

        let login_resp = create_login_response_for_user(
            &user,
            refresh_token_repo,
            jwt_secret,
            jwt_access_expires,
            jwt_refresh_expires,
        )
        .await?;

        return Ok(OAuthCallbackResult::LoginSuccess(login_resp));
    }

    if let Some(email) = &user_info.email
        && let Some(user) = user_repo.find_by_email(email, None).await?
    {
        do_bind_oauth(pool, &user.id, provider_name, &token_resp, &user_info).await?;

        let login_resp = create_login_response_for_user(
            &user,
            refresh_token_repo,
            jwt_secret,
            jwt_access_expires,
            jwt_refresh_expires,
        )
        .await?;

        eventbus.emit(crate::eventbus::Event::UserLoggedIn {
            id: user.id.clone(),
            success: true,
        });

        return Ok(OAuthCallbackResult::LoginSuccess(login_resp));
    }

    let user = auto_register_user(pool, user_repo, provider_name, &user_info, eventbus).await?;

    do_bind_oauth(pool, &user.id, provider_name, &token_resp, &user_info).await?;

    let login_resp = create_login_response_for_user(
        &user,
        refresh_token_repo,
        jwt_secret,
        jwt_access_expires,
        jwt_refresh_expires,
    )
    .await?;

    Ok(OAuthCallbackResult::LoginSuccess(login_resp))
}

/// 解绑 OAuth 账号
pub async fn unbind_oauth(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    provider_name: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_authenticated()?;
    let user = crate::models::user::find_by_id(pool, user_id, None)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    if user.password_hash.is_empty() {
        let count = oauth::count_by_user(pool, user_id).await?;
        if count <= 1 {
            return Err(AppError::BadRequest(
                "cannot unbind: user has no password and this is the only login method".into(),
            ));
        }
    }

    let deleted = oauth::delete_account(pool, user_id, provider_name).await?;
    if !deleted {
        return Err(AppError::not_found("oauth binding"));
    }

    Ok(())
}

/// 获取用户已绑定的 Provider 列表
pub async fn list_bindings(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<OAuthBindingInfo>> {
    let user_id = auth.ensure_authenticated()?;
    let accounts = oauth::find_by_user_id(pool, user_id).await?;
    Ok(accounts
        .into_iter()
        .map(|a| OAuthBindingInfo {
            provider: a.provider,
            display_name: a.display_name,
            avatar_url: a.avatar_url,
            email: a.email,
            created_at: a.created_at,
        })
        .collect())
}

/// 绑定信息
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, serde::Serialize)]
pub struct OAuthBindingInfo {
    pub provider: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub created_at: String,
}

/// 用完整用户数据生成 JWT + refresh token
async fn create_login_response_for_user(
    user: &crate::models::user::User,
    refresh_token_repo: &dyn RefreshTokenRepository,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let access_token = crate::services::auth::generate_access_token_internal(
        &user.id,
        &user.role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;

    let refresh_token_str = crate::services::auth::generate_refresh_token_string_internal()?;
    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);

    refresh_token_repo
        .create_token(&user.id, &refresh_token_str, &expires_at.to_rfc3339())
        .await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: user.clone().into(),
    })
}

/// 自动注册新用户
async fn auto_register_user(
    pool: &crate::db::Pool,
    user_repo: &dyn UserRepository,
    provider_name: &str,
    user_info: &OAuthUserInfo,
    eventbus: &crate::eventbus::EventBus,
) -> AppResult<crate::models::user::User> {
    let base_username = user_info.display_name.clone().unwrap_or_else(|| {
        format!(
            "{provider_name}_{}",
            &user_info.provider_user_id[..8.min(user_info.provider_user_id.len())]
        )
    });

    let username = ensure_unique_username(pool, &base_username).await?;
    let email = user_info.email.clone().unwrap_or_default();
    let password_hash = format!("!oauth:{provider_name}:{}", user_info.provider_user_id);

    let user = user_repo
        .create(
            CreateUserCmd {
                email,
                username,
                password_hash,
            },
            None,
        )
        .await?;

    if let Some(avatar) = &user_info.avatar_url {
        let now = crate::utils::tz::now_str();
        let sql = crate::db::dialect::translate(
            "UPDATE users SET avatar = ?, email_verified = 1, updated_at = ? WHERE id = ?",
        );
        sqlx::query(&sql)
            .bind(avatar)
            .bind(now)
            .bind(&user.id)
            .execute(pool)
            .await?;
    } else {
        let now = crate::utils::tz::now_str();
        let sql = crate::db::dialect::translate(
            "UPDATE users SET email_verified = 1, updated_at = ? WHERE id = ?",
        );
        sqlx::query(&sql)
            .bind(now)
            .bind(&user.id)
            .execute(pool)
            .await?;
    }

    let user = crate::models::user::find_by_id(pool, &user.id, None)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch created user")))?;

    eventbus.emit(crate::eventbus::Event::UserRegistered {
        id: user.id.clone(),
        username: user.username.clone(),
        email: user.email.clone(),
    });

    Ok(user)
}

/// 确保用户名唯一，冲突时追加后缀
async fn ensure_unique_username(pool: &crate::db::Pool, base: &str) -> AppResult<String> {
    let username = sanitize_username(base);

    if crate::models::user::find_by_username(pool, &username)
        .await?
        .is_none()
    {
        return Ok(username);
    }

    let prefixed = format!("github_{username}");
    if crate::models::user::find_by_username(pool, &prefixed)
        .await?
        .is_none()
    {
        return Ok(prefixed);
    }

    let suffix = crate::utils::id::random_hex(2);
    let final_name = format!("{prefixed}_{suffix}");
    Ok(final_name)
}

/// 清理用户名（只保留字母数字和下划线）
fn sanitize_username(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "user".to_string()
    } else {
        cleaned.to_string()
    }
}

/// 执行 OAuth 绑定（upsert）
async fn do_bind_oauth(
    pool: &crate::db::Pool,
    user_id: &str,
    provider_name: &str,
    token_resp: &crate::oauth::OAuthTokenResponse,
    user_info: &OAuthUserInfo,
) -> AppResult<()> {
    let existing =
        oauth::find_by_provider_user(pool, provider_name, &user_info.provider_user_id).await?;

    let profile_str = serde_json::to_string(&user_info.raw_profile).unwrap_or_default();
    let token_expires = token_resp
        .expires_in
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());

    if let Some(account) = existing {
        oauth::update_account(
            pool,
            oauth::UpdateOAuthAccountParams {
                id: &account.id,
                email: user_info.email.as_deref(),
                display_name: user_info.display_name.as_deref(),
                avatar_url: user_info.avatar_url.as_deref(),
                access_token: Some(&token_resp.access_token),
                refresh_token: token_resp.refresh_token.as_deref(),
                token_expires_at: token_expires.as_deref(),
                profile: Some(&profile_str),
            },
        )
        .await?;
    } else {
        oauth::create_account(
            pool,
            oauth::CreateOAuthAccountParams {
                user_id,
                provider: provider_name,
                provider_user_id: &user_info.provider_user_id,
                email: user_info.email.as_deref(),
                display_name: user_info.display_name.as_deref(),
                avatar_url: user_info.avatar_url.as_deref(),
                access_token: Some(&token_resp.access_token),
                refresh_token: token_resp.refresh_token.as_deref(),
                token_expires_at: token_expires.as_deref(),
                profile: Some(&profile_str),
            },
        )
        .await?;
    }

    Ok(())
}

/// 更新已有的 OAuth 绑定信息
async fn update_oauth_account(
    pool: &crate::db::Pool,
    account_id: &str,
    token_resp: &crate::oauth::OAuthTokenResponse,
    user_info: &OAuthUserInfo,
) -> AppResult<()> {
    let profile_str = serde_json::to_string(&user_info.raw_profile).unwrap_or_default();
    let token_expires = token_resp
        .expires_in
        .map(|secs| (Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());

    oauth::update_account(
        pool,
        oauth::UpdateOAuthAccountParams {
            id: account_id,
            email: user_info.email.as_deref(),
            display_name: user_info.display_name.as_deref(),
            avatar_url: user_info.avatar_url.as_deref(),
            access_token: Some(&token_resp.access_token),
            refresh_token: token_resp.refresh_token.as_deref(),
            token_expires_at: token_expires.as_deref(),
            profile: Some(&profile_str),
        },
    )
    .await
}
