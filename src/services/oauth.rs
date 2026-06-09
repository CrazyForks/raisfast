//! OAuth2 social login business logic
//!
//! Handles the core OAuth authorization flow:
//! - Initiate authorization (generate state + PKCE verifier)
//! - Handle callback (code exchange → user info → find/create/bind user → issue JWT)
//! - Bind/unbind OAuth accounts

use crate::types::snowflake_id::SnowflakeId;
use chrono::Utc;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::aspects::engine::AspectEngine;
use crate::commands::CreateUserCmd;
use crate::dto::LoginResponse;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::oauth;
use crate::oauth::{OAuthProviderRegistry, OAuthUserInfo};
use crate::utils::tz::Timestamp;

/// Initiate OAuth authorization
///
/// Generates state and PKCE code_verifier, stores them in the database, and returns the Provider authorization URL.
pub async fn initiate_oauth(
    pool: &crate::db::Pool,
    registry: &OAuthProviderRegistry,
    provider_name: &str,
    auth: &AuthUser,
) -> AppResult<String> {
    let provider = registry.get(provider_name).ok_or_else(|| {
        AppError::BadRequest(format!("unsupported OAuth provider: {provider_name}"))
    })?;

    let _state = crate::oauth::generate_state();
    let code_verifier = crate::oauth::generate_code_verifier();
    let code_challenge = crate::oauth::generate_code_challenge(&code_verifier);

    let expires_at = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();
    let resolved_user_id = auth.user_id();
    let state_id = oauth::create_state(
        pool,
        provider_name,
        &code_verifier,
        resolved_user_id,
        &expires_at,
    )
    .await?;
    let state_str = state_id.to_string();
    Ok(provider.authorize_url(&state_str, &code_challenge))
}

/// OAuth callback result
pub enum OAuthCallbackResult {
    /// Login successful (auto-redirect to frontend)
    LoginSuccess(Box<LoginResponse>),
    /// Needs binding to an existing account (returns pending binding info)
    BindingRequired {
        state: String,
        provider: String,
        user_info: OAuthUserInfo,
    },
}

/// Handle OAuth callback
///
/// 1. Validate state, retrieve code_verifier
/// 2. Exchange code for access_token
/// 3. Fetch Provider user info
/// 4. Find existing binding → directly issue JWT
/// 5. If no binding but has binding user ID → perform binding
/// 6. If no binding → auto-register new user
#[allow(clippy::too_many_arguments)]
pub async fn handle_callback(
    pool: &crate::db::Pool,
    registry: &OAuthProviderRegistry,
    provider_name: &str,
    code: &str,
    state: &str,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
    aspect_engine: &AspectEngine,
) -> AppResult<OAuthCallbackResult> {
    let provider = registry.get(provider_name).ok_or_else(|| {
        AppError::BadRequest(format!("unsupported OAuth provider: {provider_name}"))
    })?;

    let state_id = crate::types::snowflake_id::parse_id(state)?;
    let oauth_state = oauth::consume_state(pool, state_id)
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
        let user = crate::models::user::find_by_id(pool, account.user_id, None)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;

        let login_resp = create_login_response_for_user(
            &user,
            pool,
            jwt_secret,
            jwt_access_expires,
            jwt_refresh_expires,
        )
        .await?;

        update_oauth_account(pool, account.id, &token_resp, &user_info).await?;

        return Ok(OAuthCallbackResult::LoginSuccess(Box::new(login_resp)));
    }

    if let Some(bind_user_id) = oauth_state.user_id {
        do_bind_oauth(pool, bind_user_id, provider_name, &token_resp, &user_info).await?;

        let user = crate::models::user::find_by_id(pool, bind_user_id, None)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;

        let login_resp = create_login_response_for_user(
            &user,
            pool,
            jwt_secret,
            jwt_access_expires,
            jwt_refresh_expires,
        )
        .await?;

        return Ok(OAuthCallbackResult::LoginSuccess(Box::new(login_resp)));
    }

    if let Some(email) = &user_info.email {
        let cred = crate::models::user_credential::find_by_auth_type_and_identifier(
            pool,
            crate::models::user_credential::AuthType::Email,
            email,
        )
        .await?;
        if let Some(cred) = cred {
            let user = crate::models::user::find_by_id(pool, cred.user_id, None)
                .await?
                .ok_or_else(|| AppError::not_found("user"))?;
            do_bind_oauth(pool, user.id, provider_name, &token_resp, &user_info).await?;

            let login_resp = create_login_response_for_user(
                &user,
                pool,
                jwt_secret,
                jwt_access_expires,
                jwt_refresh_expires,
            )
            .await?;

            aspect_engine.emit(Event::UserLoggedIn {
                user: user.clone(),
                success: true,
            });

            return Ok(OAuthCallbackResult::LoginSuccess(Box::new(login_resp)));
        }
    }

    let user = auto_register_user(pool, provider_name, &user_info, aspect_engine).await?;

    do_bind_oauth(pool, user.id, provider_name, &token_resp, &user_info).await?;

    let login_resp = create_login_response_for_user(
        &user,
        pool,
        jwt_secret,
        jwt_access_expires,
        jwt_refresh_expires,
    )
    .await?;

    Ok(OAuthCallbackResult::LoginSuccess(Box::new(login_resp)))
}

/// Unbind an OAuth account
pub async fn unbind_oauth(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    provider_name: &str,
) -> AppResult<()> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let user = crate::models::user::find_by_id(pool, user_id, None)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    let cred_count = crate::models::user_credential::count_by_user(pool, user.id).await?;
    if cred_count <= 1 {
        return Err(AppError::BadRequest(
            "cannot unbind: this is the only login method".into(),
        ));
    }

    let deleted = oauth::delete_account(pool, user.id, provider_name).await?;
    if !deleted {
        return Err(AppError::not_found("oauth binding"));
    }

    let creds = crate::models::user_credential::find_by_user_id(pool, user.id).await?;
    for cred in creds {
        if cred.auth_type == crate::models::user_credential::AuthType::Oauth {
            crate::models::user_credential::delete_by_id(pool, cred.id).await?;
            break;
        }
    }

    Ok(())
}

/// Get the list of OAuth providers bound to the user
pub async fn list_bindings(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<Vec<OAuthBindingInfo>> {
    let user_id = auth.ensure_snowflake_user_id()?;
    let user = crate::models::user::find_by_id(pool, user_id, None)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let accounts = oauth::find_by_user_id(pool, user.id).await?;
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

/// Binding info
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, serde::Serialize)]
pub struct OAuthBindingInfo {
    pub provider: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email: Option<String>,
    pub created_at: Timestamp,
}

/// Generate JWT + refresh token with full user data
async fn create_login_response_for_user(
    user: &crate::models::user::User,
    pool: &crate::db::Pool,
    jwt_secret: &str,
    jwt_access_expires: u64,
    jwt_refresh_expires: u64,
) -> AppResult<LoginResponse> {
    let user_role = user.role;
    let access_token = crate::services::auth::generate_access_token_internal(
        user.id,
        user_role,
        user.tenant_id
            .as_deref()
            .unwrap_or(crate::constants::DEFAULT_TENANT),
        jwt_secret,
        jwt_access_expires,
    )?;

    let refresh_token_str = crate::services::auth::generate_refresh_token_string_internal()?;
    let expires_at = Utc::now() + chrono::Duration::seconds(jwt_refresh_expires as i64);

    crate::models::refresh_token::create_token(
        pool,
        user.id,
        &refresh_token_str,
        &expires_at.to_rfc3339(),
    )
    .await?;

    Ok(LoginResponse {
        access_token,
        refresh_token: refresh_token_str,
        expires_in: jwt_access_expires,
        user: crate::dto::UserResponse::from_user_with_contacts(pool, user.clone()).await?,
    })
}

/// Auto-register a new user
async fn auto_register_user(
    pool: &crate::db::Pool,
    provider_name: &str,
    user_info: &OAuthUserInfo,
    aspect_engine: &AspectEngine,
) -> AppResult<crate::models::user::User> {
    let base_username = user_info.display_name.clone().unwrap_or_else(|| {
        format!(
            "{provider_name}_{}",
            &user_info.provider_user_id[..8.min(user_info.provider_user_id.len())]
        )
    });

    let username = ensure_unique_username(pool, &base_username).await?;
    let email = user_info.email.clone().unwrap_or_default();

    let user = crate::models::user::create(
        pool,
        &CreateUserCmd {
            username,
            registered_via: crate::models::user::RegisteredVia::Oauth,
            role: None,
        },
        None,
    )
    .await?;

    if let Some(avatar) = &user_info.avatar_url {
        crate::models::user::update_avatar(pool, user.id, avatar).await?;
    }

    if !email.is_empty() {
        crate::models::user_credential::create(
            pool,
            user.id,
            crate::models::user_credential::AuthType::Email,
            &email,
            "",
            true,
        )
        .await?;
    }

    let user = crate::models::user::find_by_id(pool, user.id, None)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;

    aspect_engine.emit(Event::UserRegistered(user.clone()));

    Ok(user)
}

/// Ensure username is unique, appending a suffix on collision
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

/// Sanitize username (keep only alphanumeric and underscore)
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

/// Perform OAuth binding (upsert)
async fn do_bind_oauth(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
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
                id: account.id,
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

        let oauth_identifier = format!("{provider_name}:{}", user_info.provider_user_id);
        let oauth_data = serde_json::json!({
            "email": user_info.email,
            "display_name": user_info.display_name,
        })
        .to_string();
        crate::models::user_credential::create(
            pool,
            user_id,
            crate::models::user_credential::AuthType::Oauth,
            &oauth_identifier,
            &oauth_data,
            true,
        )
        .await?;
    }

    Ok(())
}

/// Update an existing OAuth binding
async fn update_oauth_account(
    pool: &crate::db::Pool,
    account_id: SnowflakeId,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_username_alphanumeric_only() {
        assert_eq!(sanitize_username("JohnDoe123"), "JohnDoe123");
    }

    #[test]
    fn sanitize_username_strips_special_chars() {
        assert_eq!(
            sanitize_username("john.doe@example.com"),
            "johndoeexamplecom"
        );
    }

    #[test]
    fn sanitize_username_keeps_underscore() {
        assert_eq!(sanitize_username("john_doe"), "john_doe");
    }

    #[test]
    fn sanitize_username_strips_leading_trailing_underscores() {
        assert_eq!(sanitize_username("__john__"), "john");
    }

    #[test]
    fn sanitize_username_all_special_returns_default() {
        assert_eq!(sanitize_username("@#$%"), "user");
    }

    #[test]
    fn sanitize_username_empty_returns_default() {
        assert_eq!(sanitize_username(""), "user");
    }

    #[test]
    fn sanitize_username_unicode_stripped() {
        let result = sanitize_username("用户名");
        assert_eq!(result, "user");
    }

    #[test]
    fn sanitize_username_mixed() {
        assert_eq!(sanitize_username("John_Doe-123!"), "John_Doe123");
    }
}
