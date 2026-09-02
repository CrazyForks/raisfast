//! OAuth2 authorization-code admin flow for api-clients (oauth2-egress.md §3).
//!
//! 1. `start` — build the provider auth_url (state stored briefly for CSRF).
//! 2. `callback` — exchange `code` → tokens, persist (sealed), redirect admin.
//! 3. `status` / `revoke` — inspect / clear the persisted token.
//!
//! The dynamic token is stored per (client_key, tenant) in `itg_oauth_tokens`
//! (sealed); egress resolution + auto-refresh live in `token::resolve_auth_code_token`.

use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::integration::api_client::ItgApiClient;
use crate::integration::oauth_token::{self, OauthToken};
use crate::middleware::auth::{AuthUser, TokenAction};

/// state → (client_key, expiry). Short-lived, single-use (CSRF).
static STATES: std::sync::LazyLock<dashmap::DashMap<String, (String, Instant)>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

const STATE_TTL: Duration = Duration::from_secs(300);

/// POST /admin/integration/api-clients/{id}/oauth/start
/// Returns `{auth_url}` — the frontend opens it in a new window.
pub async fn oauth_start(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Update)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;
    let creds = unseal_client_creds(&state, &client)?;
    let Some(creds) = creds else {
        return Err(AppError::BadRequest(
            "api-client has no oauth2-auth-code credentials".into(),
        ));
    };
    if !crate::integration::token::is_auth_code(&creds) {
        return Err(AppError::BadRequest(
            "api-client auth kind is not oauth2-auth-code".into(),
        ));
    }
    let auth_url = creds
        .get("auth_url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("oauth2-auth-code: missing auth_url".into()))?;
    let client_id = creds
        .get("client_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let scope = creds
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let redirect_uri = client.redirect_uri()?;

    let state_token = crate::utils::id::new_id().to_string();
    STATES.insert(
        state_token.clone(),
        (client.client_key.clone(), Instant::now() + STATE_TTL),
    );
    STATES.retain(|_, (_, exp)| Instant::now() < *exp);

    let mut url = reqwest::Url::parse(auth_url)
        .map_err(|e| AppError::BadRequest(format!("invalid auth_url: {e}")))?;
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("state", &state_token);
    if !scope.is_empty() {
        url.query_pairs_mut().append_pair("scope", scope);
    }
    // Provider-specific extra authorize params (e.g. Google needs
    // `access_type=offline` to return a refresh_token). String values only.
    if let Some(extra) = creds.get("extra_params").and_then(Value::as_object) {
        for (key, val) in extra {
            if let Some(s) = val.as_str() {
                url.query_pairs_mut().append_pair(key, s);
            }
        }
    }
    Ok(ApiResponse::success(
        serde_json::json!({ "auth_url": url.to_string() }),
    ))
}

/// GET /api/v1/integration/oauth/callback?code=..&state=..
/// Public (provider callback, no JWT). Exchanges the code, persists tokens,
/// then redirects to the admin frontend.
pub async fn oauth_callback(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> AppResult<axum::response::Redirect> {
    let code = query.get("code").cloned().unwrap_or_default();
    let state_token = query.get("state").cloned().unwrap_or_default();
    // DashMap::remove returns (key, value); the entry is keyed by the state
    // token, so the first tuple element is the key itself.
    let Some((_, (client_key, expiry))) = STATES.remove(&state_token) else {
        return Err(AppError::BadRequest(
            "oauth callback: invalid or expired state".into(),
        ));
    };
    if Instant::now() > expiry {
        return Err(AppError::BadRequest(
            "oauth callback: invalid or expired state".into(),
        ));
    }
    if code.is_empty() {
        return Err(AppError::BadRequest("oauth callback: missing code".into()));
    }

    let client = crate::integration::api_client::model::find_by_key(
        &state.pool,
        crate::constants::DEFAULT_TENANT,
        &client_key,
    )
    .await?
    .ok_or_else(|| AppError::NotFound(format!("api-client '{client_key}' not found")))?;
    let creds = unseal_client_creds(&state, &client)?
        .ok_or_else(|| AppError::BadRequest("client has no oauth credentials".into()))?;
    if !crate::integration::token::is_auth_code(&creds) {
        return Err(AppError::BadRequest(
            "client auth kind is not oauth2-auth-code".into(),
        ));
    }
    let token_url = creds
        .get("token_url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("oauth2-auth-code: missing token_url".into()))?;
    let redirect_uri = client.redirect_uri()?;

    let resp = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth http client: {e}")))?
        .post(token_url)
        // GitHub returns form-encoded by default; JSON only with Accept.
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            (
                "client_id",
                creds
                    .get("client_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            (
                "client_secret",
                creds
                    .get("client_secret")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
        ])
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth token exchange: {e}")))?;
    let status = resp.status();
    let body: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("oauth token body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "oauth token exchange failed ({status}): {body}"
        )));
    }
    // GitHub (and several providers) return 200 + {"error": ...} for a bad
    // code; treat any `error` member as a failure instead of storing a blank.
    if body.get("error").is_some() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "oauth token exchange failed: {body}"
        )));
    }

    let vault = vault(&state)?;
    let token_path = creds
        .get("access_token_path")
        .and_then(Value::as_str)
        .unwrap_or("access_token");
    let access = body
        .get(token_path)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let refresh = body
        .get("refresh_token")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expire_secs = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(3600);
    let scope = creds
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    oauth_token::upsert(
        &state.pool,
        &OauthToken {
            client_key: client_key.clone(),
            tenant_id: crate::constants::DEFAULT_TENANT.to_string(),
            access_token: Some(vault.seal(access)?),
            refresh_token: (!refresh.is_empty())
                .then(|| vault.seal(refresh))
                .transpose()?,
            expires_at: Some(crate::utils::tz::now_utc() + chrono::Duration::seconds(expire_secs)),
            scope: Some(scope),
        },
    )
    .await?;
    crate::integration::token::invalidate(&client_key);

    Ok(axum::response::Redirect::to(&format!(
        "/admin/api-clients?oauth={client_key}"
    )))
}

/// GET /admin/integration/api-clients/{id}/oauth/status
pub async fn oauth_status(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;
    let row = oauth_token::find(
        &state.pool,
        &client.client_key,
        crate::constants::DEFAULT_TENANT,
    )
    .await?;
    let Some(row) = row else {
        return Ok(ApiResponse::success(
            serde_json::json!({ "authorized": false }),
        ));
    };
    Ok(ApiResponse::success(serde_json::json!({
        "authorized": true,
        "expires_at": row.expires_at.map(|t| t.to_rfc3339()),
        "scope": row.scope,
    })))
}

/// POST /admin/integration/api-clients/{id}/oauth/revoke
pub async fn oauth_revoke(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Update)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;
    oauth_token::delete(
        &state.pool,
        &client.client_key,
        crate::constants::DEFAULT_TENANT,
    )
    .await?;
    crate::integration::token::invalidate(&client.client_key);
    Ok(ApiResponse::success(serde_json::json!({ "ok": true })))
}

fn vault(state: &AppState) -> AppResult<&crate::integration::vault::Vault> {
    state
        .integration
        .as_ref()
        .and_then(|p| p.vault())
        .ok_or_else(|| AppError::BadRequest("vault sealed (set INTEGRATION_VAULT_KEY)".into()))
}

fn unseal_client_creds(state: &AppState, client: &ItgApiClient) -> AppResult<Option<Value>> {
    let Some(sealed) = client.credentials.as_deref() else {
        return Ok(None);
    };
    let json = vault(state)?.unseal(sealed)?;
    Ok(Some(serde_json::from_str(&json).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("credential json: {e}"))
    })?))
}
