//! Admin handlers for the llm foundation (design §12): channel CRUD, the
//! write-only key-pool sub-resource, model-directory CRUD and the provider
//! preset registry. All routes are admin-scoped via `reg_route!`.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::llm::crypto;
use crate::llm::models::channel::{self, ChannelChanges, LlmKeyEntry, LlmKeyStatus, NewChannel};
use crate::llm::models::model::{self, ModelChanges};
use crate::llm::service::LlmRouter;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::{SnowflakeId, parse_id};

use std::str::FromStr as _;

/// Register routes. Paths are prefixed `/api/v1` by `reg_route!`.
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/providers",
        get,
        list_providers,
        "system",
        "admin/llm/providers",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels",
        get,
        list_channels,
        "system",
        "admin/llm/channels",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels",
        post,
        create_channel,
        "system",
        "admin/llm/channels",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}",
        get,
        get_channel,
        "system",
        "admin/llm/channels",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}",
        put,
        update_channel,
        "system",
        "admin/llm/channels",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}",
        delete,
        delete_channel,
        "system",
        "admin/llm/channels",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/keys",
        put,
        replace_keys,
        "system",
        "admin/llm/channels/keys",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/keys/{index}/enable",
        post,
        enable_key,
        "system",
        "admin/llm/channels/keys",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/keys/{index}/disable",
        post,
        disable_key,
        "system",
        "admin/llm/channels/keys",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/enable",
        post,
        enable_channel,
        "system",
        "admin/llm/channels/status",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/disable",
        post,
        disable_channel,
        "system",
        "admin/llm/channels/status",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models",
        get,
        list_models,
        "system",
        "admin/llm/models",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models",
        post,
        create_model,
        "system",
        "admin/llm/models",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models/{id}",
        get,
        get_model,
        "system",
        "admin/llm/models",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models/{id}",
        put,
        update_model,
        "system",
        "admin/llm/models",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/logs",
        get,
        list_logs,
        "system",
        "admin/llm/logs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/logs/stats",
        get,
        logs_stats,
        "system",
        "admin/llm/logs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/channels/{id}/test",
        post,
        test_channel,
        "system",
        "admin/llm/channels/test",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/group-ratios",
        get,
        get_group_ratios,
        "system",
        "admin/llm/group-ratios",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/group-ratios",
        put,
        put_group_ratios,
        "system",
        "admin/llm/group-ratios",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens",
        get,
        list_own_tokens,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens",
        post,
        create_own_token,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/models",
        get,
        selectable_models,
        "system",
        "llm/models",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens/{id}",
        put,
        update_own_token,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens/{id}",
        delete,
        delete_own_token,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/tokens",
        get,
        admin_list_tokens,
        "system",
        "admin/llm/tokens",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/tokens",
        post,
        admin_create_token,
        "system",
        "admin/llm/tokens",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens/{id}/enable",
        post,
        enable_token,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/llm/tokens/{id}/disable",
        post,
        disable_token,
        "system",
        "llm/tokens",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models/{id}/enable",
        post,
        enable_model,
        "system",
        "admin/llm/models/status",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models/{id}/disable",
        post,
        disable_model,
        "system",
        "admin/llm/models/status",
        "admin"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/llm/models/{id}",
        delete,
        delete_model,
        "system",
        "admin/llm/models",
        "admin"
    )
}

// ---------- DTOs ----------

/// Key input for create / pool replacement (plaintext in, encrypted at rest).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct KeyInput {
    pub key: String,
    #[serde(default)]
    pub max_concurrency: Option<i32>,
}

/// Masked key view — plaintext never leaves the service (§11.4).
#[derive(Debug, Serialize)]
pub struct KeyView {
    pub index: usize,
    pub status: String,
    pub disabled_reason: Option<String>,
    pub disabled_at: Option<String>,
    pub max_concurrency: Option<i32>,
    pub masked_key: String,
}

/// Channel response (no plaintext keys).
#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub key_mode: String,
    pub status: String,
    pub models: String,
    pub model_mapping: Option<serde_json::Value>,
    pub priority: i64,
    pub weight: i32,
    pub groups: String,
    pub auto_ban: bool,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
    pub config: Option<serde_json::Value>,
    pub used_quota: i64,
    pub cost_mode: String,
    pub cost_discount: f64,
    pub monthly_cost: Option<f64>,
    pub test_model: Option<String>,
    pub response_time: Option<i32>,
    pub keys: Vec<KeyView>,
    pub created_at: String,
    pub updated_at: String,
}

impl ChannelResponse {
    fn from_row(row: &channel::LlmChannel) -> Self {
        let entries = channel::parse_keys(row);
        Self {
            id: row.id,
            tenant_id: row.tenant_id.clone(),
            name: row.name.clone(),
            provider: row.provider.clone(),
            base_url: row.base_url.clone(),
            key_mode: row.key_mode.as_str().to_owned(),
            status: row.status.as_str().to_owned(),
            models: row.models.clone(),
            model_mapping: row.model_mapping.clone(),
            priority: row.priority,
            weight: row.weight,
            groups: row.channel_groups.clone(),
            auto_ban: row.auto_ban,
            param_override: row.param_override.clone(),
            header_override: row.header_override.clone(),
            config: row.config.clone(),
            used_quota: row.used_quota,
            cost_mode: row.cost_mode.as_str().to_owned(),
            cost_discount: row.cost_discount,
            monthly_cost: row.monthly_cost,
            test_model: row.test_model.clone(),
            response_time: row.response_time,
            keys: entries
                .iter()
                .enumerate()
                .map(|(index, e)| KeyView {
                    index,
                    status: e.status.as_str().to_owned(),
                    disabled_reason: e.disabled_reason.clone(),
                    disabled_at: e.disabled_at.clone(),
                    max_concurrency: e.max_concurrency,
                    masked_key: crypto::mask_key(&e.key),
                })
                .collect(),
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        }
    }
}

/// Create-channel payload (`initial_keys` required — at least one key).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateChannelReq {
    pub name: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub key_mode: Option<String>,
    pub models: String,
    #[serde(default)]
    pub model_mapping: Option<serde_json::Value>,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub weight: i32,
    #[serde(default = "default_groups")]
    pub groups: String,
    #[serde(default = "default_true_fn")]
    pub auto_ban: bool,
    #[serde(default)]
    pub param_override: Option<serde_json::Value>,
    #[serde(default)]
    pub header_override: Option<serde_json::Value>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub cost_mode: Option<String>,
    #[serde(default = "default_cost_discount")]
    pub cost_discount: f64,
    #[serde(default)]
    pub monthly_cost: Option<f64>,
    #[serde(default)]
    pub test_model: Option<String>,
    pub initial_keys: Vec<KeyInput>,
}

fn default_provider() -> String {
    "generic".to_owned()
}

fn default_groups() -> String {
    "default".to_owned()
}

fn default_true_fn() -> bool {
    true
}

fn default_cost_discount() -> f64 {
    1.0
}

/// Update-channel payload — keys are ignored here (write-only sub-resource,
/// §11.5) and logged if present.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateChannelReq {
    pub name: String,
    pub provider: String,
    pub base_url: String,
    #[serde(default)]
    pub key_mode: Option<String>,
    pub models: String,
    #[serde(default)]
    pub model_mapping: Option<serde_json::Value>,
    #[serde(default)]
    pub priority: i64,
    #[serde(default)]
    pub weight: i32,
    #[serde(default = "default_groups")]
    pub groups: String,
    #[serde(default = "default_true_fn")]
    pub auto_ban: bool,
    #[serde(default)]
    pub param_override: Option<serde_json::Value>,
    #[serde(default)]
    pub header_override: Option<serde_json::Value>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    #[serde(default)]
    pub cost_mode: Option<String>,
    #[serde(default = "default_cost_discount")]
    pub cost_discount: f64,
    #[serde(default)]
    pub monthly_cost: Option<f64>,
    #[serde(default)]
    pub test_model: Option<String>,
    #[serde(default)]
    pub keys: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ReplaceKeysReq {
    pub keys: Vec<KeyInput>,
}

fn parse_key_mode(raw: &Option<String>) -> AppResult<channel::LlmKeyMode> {
    match raw.as_deref() {
        None => Ok(channel::LlmKeyMode::Polling),
        Some("polling") => Ok(channel::LlmKeyMode::Polling),
        Some("random") => Ok(channel::LlmKeyMode::Random),
        Some(other) => Err(AppError::BadRequest(format!(
            "invalid key_mode: {other} (polling|random)"
        ))),
    }
}

fn parse_cost_mode(raw: &Option<String>) -> AppResult<channel::LlmCostMode> {
    match raw.as_deref() {
        None | Some("usage") => Ok(channel::LlmCostMode::Usage),
        Some("fixed") => Ok(channel::LlmCostMode::Fixed),
        Some(other) => Err(AppError::BadRequest(format!(
            "invalid cost_mode: {other} (usage|fixed)"
        ))),
    }
}

fn validate_base_url(url: &str) -> AppResult<()> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "base_url must start with http:// or https://".to_owned(),
        ))
    }
}

fn normalize_models(models: &str) -> String {
    models
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

fn encrypt_inputs(inputs: &[KeyInput]) -> AppResult<Vec<LlmKeyEntry>> {
    inputs
        .iter()
        .map(|k| {
            if k.key.trim().is_empty() {
                return Err(AppError::BadRequest("empty api key".to_owned()));
            }
            Ok(LlmKeyEntry {
                key: crypto::encrypt(k.key.trim())?,
                status: LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: k.max_concurrency,
            })
        })
        .collect()
}

fn channel_not_found() -> AppError {
    AppError::NotFound("llm_channel".to_owned())
}

async fn reload(router: &LlmRouter, id: SnowflakeId) {
    if let Err(err) = router.reload_channel(id).await {
        tracing::error!(%err, channel = %id, "reload llm channel after write failed");
    }
}

// ---------- providers ----------

/// Provider preset registry for admin UI dropdowns.
#[utoipa::path(get, path = "/api/v1/admin/llm/providers", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Provider presets"))
)]
pub async fn list_providers(
    auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    Ok(ApiResponse::success(
        json!({ "providers": crate::llm::registry::registry() }),
    ))
}

// ---------- channels ----------

/// List channels of the tenant.
#[utoipa::path(get, path = "/api/v1/admin/llm/channels", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Channels (masked keys)"))
)]
pub async fn list_channels(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<ChannelResponse>>> {
    auth.ensure_admin()?;
    let rows = channel::list_channels(&state.pool, auth.tenant_id()).await?;
    Ok(ApiResponse::success(
        rows.iter().map(ChannelResponse::from_row).collect(),
    ))
}

/// Create a channel with its initial key pool.
#[utoipa::path(post, path = "/api/v1/admin/llm/channels", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Channel created"))
)]
pub async fn create_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateChannelReq>,
) -> AppResult<ApiResponse<ChannelResponse>> {
    auth.ensure_admin()?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("channel name is required".to_owned()));
    }
    validate_base_url(&body.base_url)?;
    let models = normalize_models(&body.models);
    if models.is_empty() {
        return Err(AppError::BadRequest(
            "at least one model is required".to_owned(),
        ));
    }
    if body.initial_keys.is_empty() {
        return Err(AppError::BadRequest(
            "at least one initial key is required".to_owned(),
        ));
    }
    let entries = encrypt_inputs(&body.initial_keys)?;
    let row = channel::create_channel(
        &state.pool,
        auth.tenant_id(),
        NewChannel {
            name: body.name.trim().to_owned(),
            provider: body.provider,
            base_url: body.base_url.trim_end_matches('/').to_owned(),
            api_keys: channel::keys_value(&entries),
            key_mode: parse_key_mode(&body.key_mode)?,
            models,
            model_mapping: body.model_mapping,
            priority: body.priority,
            weight: body.weight,
            channel_groups: body.groups,
            auto_ban: body.auto_ban,
            param_override: body.param_override,
            header_override: body.header_override,
            config: body.config,
            cost_mode: parse_cost_mode(&body.cost_mode)?,
            cost_discount: body.cost_discount,
            monthly_cost: body.monthly_cost,
            test_model: body.test_model,
        },
    )
    .await?;
    reload(&state.llm_router, row.id).await;
    Ok(ApiResponse::success(ChannelResponse::from_row(&row)))
}

/// Get one channel.
#[utoipa::path(get, path = "/api/v1/admin/llm/channels/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Channel detail"))
)]
pub async fn get_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ChannelResponse>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let row = channel::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(channel_not_found)?;
    Ok(ApiResponse::success(ChannelResponse::from_row(&row)))
}

/// Update editable channel columns (keys ignored — use the keys sub-resource).
#[utoipa::path(put, path = "/api/v1/admin/llm/channels/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Channel updated"))
)]
pub async fn update_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateChannelReq>,
) -> AppResult<ApiResponse<ChannelResponse>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    if body.keys.is_some() {
        tracing::warn!(channel = %id, "ignoring keys in channel update; use PUT /keys");
    }
    validate_base_url(&body.base_url)?;
    let models = normalize_models(&body.models);
    if models.is_empty() {
        return Err(AppError::BadRequest(
            "at least one model is required".to_owned(),
        ));
    }
    channel::update_channel(
        &state.pool,
        auth.tenant_id(),
        id,
        ChannelChanges {
            name: body.name.trim().to_owned(),
            provider: body.provider,
            base_url: body.base_url.trim_end_matches('/').to_owned(),
            key_mode: parse_key_mode(&body.key_mode)?,
            models,
            model_mapping: body.model_mapping,
            priority: body.priority,
            weight: body.weight,
            channel_groups: body.groups,
            auto_ban: body.auto_ban,
            param_override: body.param_override,
            header_override: body.header_override,
            config: body.config,
            cost_mode: parse_cost_mode(&body.cost_mode)?,
            cost_discount: body.cost_discount,
            monthly_cost: body.monthly_cost,
            test_model: body.test_model,
        },
    )
    .await?;
    let row = channel::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(channel_not_found)?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(ChannelResponse::from_row(&row)))
}

/// Delete a channel.
#[utoipa::path(delete, path = "/api/v1/admin/llm/channels/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Channel deleted"))
)]
pub async fn delete_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    channel::delete_channel(&state.pool, id, auth.tenant_id()).await?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(()))
}

/// Replace the whole key pool (write-only credential sub-resource, §11.5).
#[utoipa::path(put, path = "/api/v1/admin/llm/channels/{id}/keys", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Channel ID")),
    responses((status = 200, description = "Key pool replaced"))
)]
pub async fn replace_keys(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReplaceKeysReq>,
) -> AppResult<ApiResponse<ChannelResponse>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    if body.keys.is_empty() {
        return Err(AppError::BadRequest(
            "at least one key is required".to_owned(),
        ));
    }
    let entries = encrypt_inputs(&body.keys)?;
    let _lock = state.llm_router.persist_lock(id).await;
    channel::replace_keys(&state.pool, auth.tenant_id(), id, &entries).await?;
    let row = channel::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(channel_not_found)?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(ChannelResponse::from_row(&row)))
}

/// Manually re-enable one key (§6.3).
#[utoipa::path(post, path = "/api/v1/admin/llm/channels/{id}/keys/{index}/enable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path), ("index" = u32, Path)),
    responses((status = 200, description = "Key enabled")))]
pub async fn enable_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, index)): Path<(String, String)>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let (id, index) = parse_key_path(&id, &index)?;
    let _lock = state.llm_router.persist_lock(id).await;
    channel::update_key_status(
        &state.pool,
        auth.tenant_id(),
        id,
        index,
        LlmKeyStatus::Active,
        Some("manual enable"),
    )
    .await?;
    if let Some(row) = channel::find_by_id(&state.pool, id, auth.tenant_id()).await?
        && row.status == channel::LlmChannelStatus::AutoDisabled
        && channel::parse_keys(&row)
            .iter()
            .any(|k| k.status == LlmKeyStatus::Active)
    {
        channel::update_status(
            &state.pool,
            auth.tenant_id(),
            id,
            channel::LlmChannelStatus::Enabled,
        )
        .await?;
    }
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(()))
}

/// Manually disable one key (symmetric counterpart of enable, §12).
#[utoipa::path(post, path = "/api/v1/admin/llm/channels/{id}/keys/{index}/disable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path), ("index" = u32, Path)),
    responses((status = 200, description = "Key disabled")))]
pub async fn disable_key(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, index)): Path<(String, String)>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let (id, index) = parse_key_path(&id, &index)?;
    let _lock = state.llm_router.persist_lock(id).await;
    channel::update_key_status(
        &state.pool,
        auth.tenant_id(),
        id,
        index,
        LlmKeyStatus::Disabled,
        Some("manual disable"),
    )
    .await?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(()))
}

/// Manually enable a channel (manual_disabled → enabled).
#[utoipa::path(post, path = "/api/v1/admin/llm/channels/{id}/enable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Channel enabled")))]
pub async fn enable_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    channel::update_status(
        &state.pool,
        auth.tenant_id(),
        id,
        channel::LlmChannelStatus::Enabled,
    )
    .await?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(()))
}

/// Manually disable a channel (→ manual_disabled; never auto-recovered, §6.3).
#[utoipa::path(post, path = "/api/v1/admin/llm/channels/{id}/disable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Channel disabled")))]
pub async fn disable_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    channel::update_status(
        &state.pool,
        auth.tenant_id(),
        id,
        channel::LlmChannelStatus::ManualDisabled,
    )
    .await?;
    reload(&state.llm_router, id).await;
    Ok(ApiResponse::success(()))
}

fn parse_key_path(id: &str, index: &str) -> AppResult<(SnowflakeId, usize)> {
    let id = parse_id(id)?;
    let index: usize = index
        .parse()
        .map_err(|_| AppError::BadRequest("invalid key index".to_owned()))?;
    Ok((id, index))
}

// ---------- model directory ----------

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ModelReq {
    pub name: String,
    #[serde(default = "default_model_type")]
    pub model_type: String,
    #[serde(default = "default_price_mode")]
    pub price_mode: String,
    #[serde(default = "default_price")]
    pub input_price: f64,
    #[serde(default = "default_price")]
    pub output_price: f64,
    #[serde(default)]
    pub cache_read_price: Option<f64>,
    #[serde(default)]
    pub cache_write_price: Option<f64>,
    #[serde(default)]
    pub call_price: Option<f64>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default = "default_model_status")]
    pub status: String,
}

fn default_model_type() -> String {
    "chat".to_owned()
}

fn default_price_mode() -> String {
    "token".to_owned()
}

fn default_price() -> f64 {
    1.0
}

fn default_model_status() -> String {
    "active".to_owned()
}

impl ModelReq {
    fn into_changes(self) -> AppResult<ModelChanges> {
        use crate::llm::models::model::{LlmModelStatus, LlmModelType, LlmPriceMode};
        use std::str::FromStr;
        let model_type = LlmModelType::from_str(&self.model_type).map_err(|_| {
            AppError::BadRequest(format!("invalid model_type: {}", self.model_type))
        })?;
        let price_mode = LlmPriceMode::from_str(&self.price_mode).map_err(|_| {
            AppError::BadRequest(format!("invalid price_mode: {}", self.price_mode))
        })?;
        let status = LlmModelStatus::from_str(&self.status)
            .map_err(|_| AppError::BadRequest(format!("invalid status: {}", self.status)))?;
        Ok(ModelChanges {
            name: self.name.trim().to_owned(),
            model_type,
            price_mode,
            input_price: self.input_price,
            output_price: self.output_price,
            cache_read_price: self.cache_read_price,
            cache_write_price: self.cache_write_price,
            call_price: self.call_price,
            params: self.params,
            status,
        })
    }
}

#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct ListModelsQuery {
    pub model_type: Option<String>,
}

/// List model-directory rows (optional `?model_type=` filter).
#[utoipa::path(get, path = "/api/v1/admin/llm/models", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Model directory")))]
pub async fn list_models(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListModelsQuery>,
) -> AppResult<ApiResponse<Vec<model::LlmModel>>> {
    auth.ensure_admin()?;
    let model_type = match q.model_type.as_deref() {
        Some(t) => Some(
            crate::llm::models::model::LlmModelType::from_str(t)
                .map_err(|_| AppError::BadRequest(format!("invalid model_type: {t}")))?,
        ),
        None => None,
    };
    let rows = model::list_models(&state.pool, auth.tenant_id(), model_type.as_ref()).await?;
    Ok(ApiResponse::success(rows))
}

/// Create a model-directory row.
#[utoipa::path(post, path = "/api/v1/admin/llm/models", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Model created")))]
pub async fn create_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ModelReq>,
) -> AppResult<ApiResponse<model::LlmModel>> {
    auth.ensure_admin()?;
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("model name is required".to_owned()));
    }
    let row = model::create_model(&state.pool, auth.tenant_id(), body.into_changes()?).await?;
    state.llm_router.invalidate_model_cache().await;
    Ok(ApiResponse::success(row))
}

/// Get one model-directory row.
#[utoipa::path(get, path = "/api/v1/admin/llm/models/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Model detail")))]
pub async fn get_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<model::LlmModel>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let row = model::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::NotFound("llm_model".to_owned()))?;
    Ok(ApiResponse::success(row))
}

/// Update a model-directory row (pricing changes invalidate the cache, §7.1).
#[utoipa::path(put, path = "/api/v1/admin/llm/models/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Model updated")))]
pub async fn update_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ModelReq>,
) -> AppResult<ApiResponse<model::LlmModel>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    model::update_model(&state.pool, auth.tenant_id(), id, body.into_changes()?).await?;
    state.llm_router.invalidate_model_cache().await;
    let row = model::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::NotFound("llm_model".to_owned()))?;
    Ok(ApiResponse::success(row))
}

/// Delete a model-directory row.
#[utoipa::path(delete, path = "/api/v1/admin/llm/models/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Model deleted")))]
pub async fn delete_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    model::delete_model(&state.pool, id, auth.tenant_id()).await?;
    state.llm_router.invalidate_model_cache().await;
    Ok(ApiResponse::success(()))
}

/// Enable a model (switch toggle — re-enters the active directory).
#[utoipa::path(post, path = "/api/v1/admin/llm/models/{id}/enable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Model enabled")))]
pub async fn enable_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    set_model_status(auth, state, &id, model::LlmModelStatus::Active).await
}

/// Disable a model (switch toggle — leaves the active directory).
#[utoipa::path(post, path = "/api/v1/admin/llm/models/{id}/disable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Model disabled")))]
pub async fn disable_model(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    set_model_status(auth, state, &id, model::LlmModelStatus::Disabled).await
}

async fn set_model_status(
    auth: AuthUser,
    state: AppState,
    id_str: &str,
    status: model::LlmModelStatus,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    let id = parse_id(id_str)?;
    model::update_status(&state.pool, auth.tenant_id(), id, status).await?;
    state.llm_router.invalidate_model_cache().await;
    Ok(ApiResponse::success(()))
}

// ---------- user self-service tokens (§12) ----------

/// Models selectable for a token allowlist (§8.1 semantics, JWT-authed):
/// ALL models declared by enabled channels of the caller's tenant, split
/// into `models` (priced/relay-able — the /v1/models pool) and `unpriced`
/// (not in the active directory: relay rejects them until priced, so the
/// picker shows them disabled instead of silently hiding them).
#[utoipa::path(get, path = "/api/v1/llm/models", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Selectable + unpriced model names"))
)]
pub async fn selectable_models(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let _ = auth.ensure_snowflake_user_id()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_owned();
    let cache = state.llm_router.cache_snapshot();
    let mut names: Vec<String> = cache
        .channels
        .values()
        .filter(|ch| ch.tenant == tenant && ch.status == channel::LlmChannelStatus::Enabled)
        .flat_map(|ch| ch.models.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    let mut models = Vec::new();
    let mut unpriced = Vec::new();
    for name in names {
        if cache.model_info(&tenant, &name).is_some() {
            models.push(name);
        } else {
            unpriced.push(name);
        }
    }
    Ok(ApiResponse::success(serde_json::json!({
        "models": models,
        "unpriced": unpriced,
    })))
}

/// Token view (decrypted key for reveal/copy — api_token pattern; masked
/// client-side in the UI).
#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    /// Owner username (admin list view; resolved from `users`).
    pub username: Option<String>,
    pub name: String,
    pub status: String,
    pub remain_quota: crate::types::quota::Quota,
    pub used_quota: crate::types::quota::Quota,
    pub unlimited_quota: bool,
    pub expired_at: Option<String>,
    pub allowed_models: Option<String>,
    pub group: String,
    pub created_at: String,
    pub key: Option<String>,
}

impl TokenResponse {
    fn from_row(row: &crate::llm::models::token::LlmToken, username: Option<String>) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            username,
            name: row.name.clone(),
            status: row.status.as_str().to_owned(),
            remain_quota: row.remain_quota,
            used_quota: row.used_quota,
            unlimited_quota: row.unlimited_quota,
            expired_at: row.expired_at.map(|t| t.to_rfc3339()),
            allowed_models: row.allowed_models.clone(),
            group: row
                .token_group
                .clone()
                .unwrap_or_else(|| "default".to_owned()),
            created_at: row.created_at.to_rfc3339(),
            key: row.key_enc.as_deref().and_then(crypto::decrypt),
        }
    }
}

/// Create-token payload (unlimited is admin-only and rejected here, §9.2).
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTokenReq {
    pub name: String,
    #[serde(default)]
    pub remain_quota: crate::types::quota::Quota,
    #[serde(default)]
    pub unlimited_quota: bool,
    #[serde(default)]
    pub expired_at: Option<String>,
    #[serde(default)]
    pub allowed_models: Option<String>,
    #[serde(default)]
    pub allowed_ips: Option<String>,
    #[serde(default)]
    pub token_group: Option<String>,
    /// Owner override (admin route only; ignored by `/llm/tokens`).
    #[serde(default)]
    pub user_id: Option<String>,
}

/// Update-token payload.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct UpdateTokenReq {
    pub name: String,
    #[serde(default)]
    pub remain_quota: Option<crate::types::quota::Quota>,
    #[serde(default)]
    pub allowed_models: Option<String>,
    #[serde(default)]
    pub allowed_ips: Option<String>,
    #[serde(default)]
    pub expired_at: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub token_group: Option<String>,
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListLogsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub channel_id: Option<String>,
    pub token_id: Option<String>,
    pub model_name: Option<String>,
    /// Filter by owner: case-insensitive username substring or exact user id.
    pub username: Option<String>,
}

/// List the caller's sk- tokens.
#[utoipa::path(get, path = "/api/v1/llm/tokens", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Own tokens")))]
pub async fn list_own_tokens(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<TokenResponse>>> {
    let user = auth.ensure_snowflake_user_id()?;
    let rows = crate::llm::models::token::list_by_user(&state.pool, auth.tenant_id(), user).await?;
    Ok(ApiResponse::success(
        rows.iter()
            .map(|row| TokenResponse::from_row(row, None))
            .collect(),
    ))
}

/// Query params for the admin token list.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListTokensQuery {
    /// Filter by owner: case-insensitive username substring or exact user id.
    pub username: Option<String>,
}

/// Admin: list every token in the tenant with owner usernames.
#[utoipa::path(get, path = "/api/v1/admin/llm/tokens", tag = "llm",
    security(("bearer_auth" = [])),
    params(("username" = Option<String>, Query, description = "Owner username substring or user id")),
    responses((status = 200, description = "All tokens with owner usernames")))]
pub async fn admin_list_tokens(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListTokensQuery>,
) -> AppResult<ApiResponse<Vec<TokenResponse>>> {
    auth.ensure_admin()?;
    let filter = q
        .username
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let rows = crate::llm::models::token::list_all(&state.pool, auth.tenant_id()).await?;
    let ids: Vec<SnowflakeId> = rows.iter().map(|r| r.user_id).collect();
    let names = crate::models::user::find_usernames_by_ids(&state.pool, &ids).await?;
    let matches = |row: &crate::llm::models::token::LlmToken| match &filter {
        Some(f) => {
            let id = row.user_id.0.to_string();
            names
                .get(&row.user_id.0)
                .is_some_and(|n| n.to_lowercase().contains(f))
                || id == *f
        }
        None => true,
    };
    Ok(ApiResponse::success(
        rows.iter()
            .filter(|row| matches(row))
            .map(|row| TokenResponse::from_row(row, names.get(&row.user_id.0).cloned()))
            .collect(),
    ))
}

/// Shared mint path for own-create and admin-create (§12): hash + encrypt the
/// fresh key and insert the row for `user`.
async fn mint_token(
    state: &AppState,
    tenant_id: Option<&str>,
    user: SnowflakeId,
    body: &CreateTokenReq,
) -> AppResult<serde_json::Value> {
    if body.name.trim().is_empty() {
        return Err(AppError::BadRequest("token name is required".to_owned()));
    }
    let plain = crate::llm::relay::generate_sk();
    let expired_at = crate::utils::tz::parse_rfc3339_opt(body.expired_at.as_deref());
    let row = crate::llm::models::token::create_token(
        &state.pool,
        tenant_id,
        user,
        body.name.trim(),
        &plain,
        body.remain_quota,
        body.unlimited_quota,
        expired_at,
        body.allowed_models.as_deref().filter(|s| !s.is_empty()),
        body.allowed_ips.as_deref().filter(|s| !s.is_empty()),
        body.token_group.as_deref().filter(|s| !s.is_empty()),
    )
    .await?;
    Ok(serde_json::json!({
        "token": TokenResponse::from_row(&row, None),
        "key": plain,
    }))
}

/// Create a sk- token; the plaintext key appears exactly once in the response.
#[utoipa::path(post, path = "/api/v1/llm/tokens", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Token created (plaintext shown once)"))
)]
pub async fn create_own_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTokenReq>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let user = auth.ensure_snowflake_user_id()?;
    if body.unlimited_quota {
        auth.ensure_admin()?;
    }
    let value = mint_token(&state, auth.tenant_id(), user, &body).await?;
    Ok(ApiResponse::success(value))
}

/// Admin: create a sk- token on behalf of the given `user_id` (defaults to
/// the caller when omitted).
#[utoipa::path(post, path = "/api/v1/admin/llm/tokens", tag = "llm",
    security(("bearer_auth" = [])),
    request_body = CreateTokenReq,
    responses((status = 200, description = "Token created for the target user")))]
pub async fn admin_create_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateTokenReq>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    let user = match body.user_id.as_deref() {
        Some(s) => {
            let id = parse_id(s)?;
            crate::models::user::find_by_id(&state.pool, id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::BadRequest(format!("user not found: {s}")))?
                .id
        }
        None => auth.ensure_snowflake_user_id()?,
    };
    let value = mint_token(&state, auth.tenant_id(), user, &body).await?;
    Ok(ApiResponse::success(value))
}

/// Load a token scoped to the caller: owner or admin (shared by
/// update/delete/enable/disable).
async fn own_or_admin_token(
    auth: &AuthUser,
    state: &AppState,
    id_str: &str,
) -> AppResult<crate::llm::models::token::LlmToken> {
    let user = auth.ensure_snowflake_user_id()?;
    let id = parse_id(id_str)?;
    let existing = crate::llm::models::token::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::NotFound("llm_token".to_owned()))?;
    if existing.user_id != user && !auth.is_admin() {
        return Err(AppError::ForbiddenOwnership);
    }
    Ok(existing)
}

/// Update an own token (admins may update any token in the tenant).
#[utoipa::path(put, path = "/api/v1/llm/tokens/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Token updated"))
)]
pub async fn update_own_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateTokenReq>,
) -> AppResult<ApiResponse<()>> {
    let existing = own_or_admin_token(&auth, &state, &id).await?;
    use crate::llm::models::token::LlmTokenStatus;
    let status = match body.enabled {
        Some(true) => LlmTokenStatus::Enabled,
        Some(false) => LlmTokenStatus::Disabled,
        None => existing.status,
    };
    let ph = crate::db::Driver::ph;
    use crate::db::driver::DbDriver;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE llm_tokens SET name = {p1}, remain_quota = {p2}, allowed_models = {p3}, \
         allowed_ips = {p4}, expired_at = {p5}, status = {p6}, token_group = {p7}, \
         updated_at = {p8} WHERE id = {p9}",
        p1 = ph(1),
        p2 = ph(2),
        p3 = ph(3),
        p4 = ph(4),
        p5 = ph(5),
        p6 = ph(6),
        p7 = ph(7),
        p8 = ph(8),
        p9 = ph(9)
    );
    let expired_at =
        crate::utils::tz::parse_rfc3339_opt(body.expired_at.as_deref()).or(existing.expired_at);
    let remain = body.remain_quota.unwrap_or(existing.remain_quota);
    let remain = crate::types::quota::Quota(remain.0.max(0));
    let token_group = body
        .token_group
        .clone()
        .filter(|s| !s.is_empty())
        .or(existing.token_group);
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(body.name.trim())
        .bind(remain)
        .bind(body.allowed_models.clone().filter(|s| !s.is_empty()))
        .bind(body.allowed_ips.clone().filter(|s| !s.is_empty()))
        .bind(expired_at)
        .bind(status.as_str())
        .bind(token_group)
        .bind(now)
        .bind(existing.id)
        .execute(&state.pool)
        .await?;
    AppError::expect_affected(&result, "llm_token")?;
    Ok(ApiResponse::success(()))
}

/// Delete an own token (admins may delete any token in the tenant).
#[utoipa::path(delete, path = "/api/v1/llm/tokens/{id}", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Token deleted"))
)]
pub async fn delete_own_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let existing = own_or_admin_token(&auth, &state, &id).await?;
    crate::llm::models::token::delete_token(&state.pool, existing.id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(()))
}

/// Enable a token (owner or admin) — switch toggle in the token list.
#[utoipa::path(post, path = "/api/v1/llm/tokens/{id}/enable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Token enabled")))]
pub async fn enable_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let existing = own_or_admin_token(&auth, &state, &id).await?;
    crate::llm::models::token::update_status(
        &state.pool,
        auth.tenant_id(),
        existing.id,
        crate::llm::models::token::LlmTokenStatus::Enabled,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// Disable a token (owner or admin) — switch toggle in the token list.
#[utoipa::path(post, path = "/api/v1/llm/tokens/{id}/disable", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Token disabled")))]
pub async fn disable_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let existing = own_or_admin_token(&auth, &state, &id).await?;
    crate::llm::models::token::update_status(
        &state.pool,
        auth.tenant_id(),
        existing.id,
        crate::llm::models::token::LlmTokenStatus::Disabled,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

// ---------- admin logs (§12) ----------

/// Paged usage-log query with filters.
#[utoipa::path(get, path = "/api/v1/admin/llm/logs", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Usage logs"))
)]
pub async fn list_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListLogsQuery>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::llm::models::log::LlmLog>>>
{
    auth.ensure_admin()?;
    let page = q.page.unwrap_or(1).max(1);
    let page_size = q.page_size.unwrap_or(20).clamp(1, 100);
    // Username → user ids (substring match; pure digits also try exact id).
    let (user_ids, username_given) = match q.username.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => {
            let mut ids =
                crate::models::user::find_ids_by_username_like(&state.pool, s).await?;
            if let Ok(n) = s.parse::<i64>()
                && let Some(u) =
                    crate::models::user::find_by_id(&state.pool, SnowflakeId(n), None).await?
                && !ids.contains(&u.id)
            {
                ids.push(u.id);
            }
            (ids, true)
        }
        _ => (Vec::new(), false),
    };
    let filters = crate::llm::models::log::LogFilters {
        channel_id: q.channel_id,
        token_id: q.token_id,
        model_name: q.model_name,
        user_ids,
        username_given,
    };
    let (items, total) = crate::llm::models::log::query_paged(
        &state.pool,
        auth.tenant_id(),
        &filters,
        page,
        page_size,
    )
    .await?;
    Ok(ApiResponse::success(
        crate::errors::response::PaginatedData {
            items,
            total,
            page,
            page_size,
        },
    ))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LogStatsQuery {
    /// Group dimension: `day` (default) | `model` | `user` | `channel`.
    pub group_by: Option<String>,
    /// Lookback window when `start`/`end` are omitted (default 30, 1..=365).
    pub days: Option<i64>,
    /// Inclusive window bounds (`YYYY-MM-DD`); override `days`.
    pub start: Option<String>,
    pub end: Option<String>,
    /// Max groups for non-day dimensions; remainder is bucketed as "other".
    pub limit: Option<i64>,
}

/// Aggregated usage/billing stats: group by day / model / user / channel over
/// a date window. Amounts are USD (charge / cost / profit); day mode is
/// zero-filled for a continuous series.
#[utoipa::path(get, path = "/api/v1/admin/llm/logs/stats", tag = "llm",
    security(("bearer_auth" = [])),
    params(
        ("group_by" = Option<String>, Query, description = "day|model|user|channel"),
        ("days" = Option<i64>, Query, description = "Lookback days (default 30)"),
        ("start" = Option<String>, Query, description = "YYYY-MM-DD"),
        ("end" = Option<String>, Query, description = "YYYY-MM-DD"),
        ("limit" = Option<i64>, Query, description = "Top-N for non-day groups"),
    ),
    responses((status = 200, description = "Grouped stats + totals")))]
pub async fn logs_stats(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<LogStatsQuery>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    let group_by = q.group_by.unwrap_or_else(|| "day".to_owned());
    if !matches!(group_by.as_str(), "day" | "model" | "user" | "channel") {
        return Err(AppError::BadRequest(format!(
            "invalid group_by: {group_by} (day|model|user|channel)"
        )));
    }
    let parse_date = |s: &str| {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| AppError::BadRequest(format!("invalid date (want YYYY-MM-DD): {s}")))
    };
    let today = crate::utils::tz::now_utc().date_naive();
    let end = match q.end.as_deref() {
        Some(s) => parse_date(s)?,
        None => today,
    };
    let start = match q.start.as_deref() {
        Some(s) => parse_date(s)?,
        None => end - chrono::Duration::days(q.days.unwrap_or(30).clamp(1, 365) - 1),
    };
    let span_days = (end - start).num_days();
    if span_days < 0 {
        return Err(AppError::BadRequest("start is after end".to_owned()));
    }
    if span_days > 366 {
        return Err(AppError::BadRequest(
            "date range too large (max 366 days)".to_owned(),
        ));
    }
    let start_s = start.format("%Y-%m-%d").to_string();
    let end_s = end.format("%Y-%m-%d").to_string();

    let buckets = crate::llm::models::log::stats_by(
        &state.pool,
        auth.tenant_id(),
        &group_by,
        &start_s,
        &end_s,
    )
    .await?;

    let to_usd = |q: i64| q as f64 / crate::types::quota::QUOTA_PER_USD;
    let bucket_json = |key: &str, label: &str, b: Option<&crate::llm::models::log::StatBucket>| {
        let (requests, prompt, completion, quota, cost) = match b {
            Some(b) => (
                b.requests,
                b.prompt_tokens,
                b.completion_tokens,
                b.quota,
                b.cost_quota,
            ),
            None => (0, 0, 0, 0, 0),
        };
        serde_json::json!({
            "key": key,
            "label": label,
            "requests": requests,
            "prompt_tokens": prompt,
            "completion_tokens": completion,
            "charge_usd": to_usd(quota),
            "cost_usd": to_usd(cost),
            "profit_usd": to_usd(quota - cost),
        })
    };

    let totals = buckets
        .iter()
        .fold((0i64, 0i64, 0i64, 0i64, 0i64), |mut t, b| {
            t.0 += b.requests;
            t.1 += b.prompt_tokens;
            t.2 += b.completion_tokens;
            t.3 += b.quota;
            t.4 += b.cost_quota;
            t
        });

    let data: Vec<serde_json::Value> = if group_by == "day" {
        // Continuous series: every day in [start, end], zero-filled.
        let by_day: std::collections::HashMap<&str, &crate::llm::models::log::StatBucket> =
            buckets.iter().map(|b| (b.key.as_str(), b)).collect();
        let mut data = Vec::with_capacity((span_days + 1) as usize);
        let mut d = start;
        loop {
            let key = d.format("%Y-%m-%d").to_string();
            data.push(bucket_json(&key, &key, by_day.get(key.as_str()).copied()));
            if d == end {
                break;
            }
            d += chrono::Duration::days(1);
        }
        data
    } else {
        let limit = q.limit.unwrap_or(12).clamp(1, 100) as usize;
        let mut data: Vec<serde_json::Value> = Vec::new();
        for (i, b) in buckets.iter().enumerate() {
            if i < limit {
                data.push(bucket_json(&b.key, &b.label, Some(b)));
            } else {
                let rest = &buckets[i..];
                let agg = crate::llm::models::log::StatBucket {
                    key: "__other__".to_owned(),
                    label: "__other__".to_owned(),
                    requests: rest.iter().map(|b| b.requests).sum(),
                    prompt_tokens: rest.iter().map(|b| b.prompt_tokens).sum(),
                    completion_tokens: rest.iter().map(|b| b.completion_tokens).sum(),
                    quota: rest.iter().map(|b| b.quota).sum(),
                    cost_quota: rest.iter().map(|b| b.cost_quota).sum(),
                };
                data.push(bucket_json("__other__", "__other__", Some(&agg)));
                break;
            }
        }
        data
    };

    Ok(ApiResponse::success(serde_json::json!({
        "group_by": group_by,
        "start": start_s,
        "end": end_s,
        "totals": {
            "requests": totals.0,
            "prompt_tokens": totals.1,
            "completion_tokens": totals.2,
            "charge_usd": to_usd(totals.3),
            "cost_usd": to_usd(totals.4),
            "profit_usd": to_usd(totals.3 - totals.4),
        },
        "data": data,
    })))
}

/// Sell-price multipliers by Key billing group (runtime-adjustable option
/// `llm_group_ratios`, stored as a JSON string; unset → {"default":1.0}).
#[utoipa::path(get, path = "/api/v1/admin/llm/group-ratios", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Group ratios")))]
pub async fn get_group_ratios(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    Ok(ApiResponse::success(serde_json::json!({
        "ratios": read_group_ratios(&state.pool).await,
    })))
}

/// Replace the group-ratio map. Keys are arbitrary group names; values must be
/// finite and > 0 (a 0 sell price is a legitimate free group for *models*, but
/// a whole group priced at 0 is almost always a mistake).
#[utoipa::path(put, path = "/api/v1/admin/llm/group-ratios", tag = "llm",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Group ratios replaced")))]
pub async fn put_group_ratios(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<GroupRatiosReq>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    if body.ratios.is_empty() {
        return Err(AppError::BadRequest(
            "at least one group is required".to_owned(),
        ));
    }
    for (name, ratio) in &body.ratios {
        if name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "group name cannot be empty".to_owned(),
            ));
        }
        if !ratio.is_finite() || *ratio < 0.0 {
            return Err(AppError::BadRequest(format!(
                "invalid ratio for group {name}: must be a finite number >= 0"
            )));
        }
    }
    let value = serde_json::to_string(&body.ratios)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize group ratios: {e}")))?;
    crate::models::options::upsert_value(
        &state.pool,
        "llm_group_ratios",
        &serde_json::Value::String(value),
        None,
    )
    .await?;
    Ok(ApiResponse::success(serde_json::json!({
        "ratios": body.ratios,
    })))
}

/// Group-ratio map payload.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct GroupRatiosReq {
    pub ratios: std::collections::BTreeMap<String, f64>,
}

/// Read + parse the `llm_group_ratios` option (JSON string or object),
/// defaulting to `{"default": 1.0}` when unset/malformed.
pub(crate) async fn read_group_ratios(
    pool: &crate::db::Pool,
) -> std::collections::BTreeMap<String, f64> {
    let mut out = std::collections::BTreeMap::new();
    let Ok(Some(row)) = crate::models::options::find_by_key(pool, "llm_group_ratios", None).await
    else {
        out.insert("default".to_owned(), 1.0);
        return out;
    };
    let parsed = match &row.value {
        serde_json::Value::String(s) => serde_json::from_str::<serde_json::Value>(s).ok(),
        other => Some(other.clone()),
    };
    if let Some(serde_json::Value::Object(map)) = parsed {
        for (k, v) in map {
            if let Some(f) = v.as_f64() {
                out.insert(k, f);
            }
        }
    }
    if out.is_empty() {
        out.insert("default".to_owned(), 1.0);
    }
    out
}

// ---------- channel connectivity test (§7.5 manual) ----------

/// Test one channel: pick its first active key, send a minimal chat request
/// against `test_model`, record response time.
#[utoipa::path(post, path = "/api/v1/admin/llm/channels/{id}/test", tag = "llm",
    security(("bearer_auth" = [])),
    params(("id" = String, Path)),
    responses((status = 200, description = "Test result"))
)]
pub async fn test_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_admin()?;
    let id = parse_id(&id)?;
    let row = channel::find_by_id(&state.pool, id, auth.tenant_id())
        .await?
        .ok_or_else(channel_not_found)?;
    let entries = channel::parse_keys(&row);
    let key = entries
        .iter()
        .find(|e| e.status == LlmKeyStatus::Active)
        .and_then(|e| crypto::decrypt(&e.key))
        .ok_or_else(|| AppError::BadRequest("channel has no active key".to_owned()))?;
    let model = row
        .test_model
        .clone()
        .or_else(|| {
            row.models
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if model.is_empty() {
        return Err(AppError::BadRequest(
            "no test_model and no models".to_owned(),
        ));
    }
    let started = std::time::Instant::now();
    let client = crate::llm::relay::shared_client();
    let url = format!("{}/chat/completions", row.base_url.trim_end_matches('/'));
    let body = serde_json::json!({ "model": model, "messages": [{"role":"user","content":"ping"}], "max_tokens": 1 });
    let resp = client
        .post(&url)
        .headers(crate::llm::relay::OpenaiHeaders::for_key(
            &key,
            row.header_override.as_ref(),
        ))
        .json(&body)
        .send()
        .await;
    let elapsed_ms = started.elapsed().as_millis() as i32;
    let (ok, detail) = match resp {
        Ok(r) if r.status().is_success() => (true, format!("{} ok", r.status().as_u16())),
        Ok(r) => (
            false,
            format!(
                "{} {}",
                r.status().as_u16(),
                r.text().await.unwrap_or_default()
            ),
        ),
        Err(e) => (false, format!("transport: {e}")),
    };
    let ph = crate::db::Driver::ph;
    use crate::db::driver::DbDriver;
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE llm_channels SET test_time = {p1}, response_time = {p2} WHERE id = {p3}",
        p1 = ph(1),
        p2 = ph(2),
        p3 = ph(3)
    );
    let _ = sqlx::query(crate::db::safe_sql(&sql))
        .bind(now)
        .bind(elapsed_ms)
        .bind(id)
        .execute(&state.pool)
        .await;
    let log = crate::llm::models::log::NewLog {
        tenant_id: auth.tenant_id().map(str::to_owned),
        source: crate::llm::models::log::LogSource::Test,
        channel_id: Some(id),
        model_name: model.clone(),
        elapsed_ms: Some(elapsed_ms),
        error_message: if ok { None } else { Some(detail.clone()) },
        ..Default::default()
    };
    if let Err(err) = crate::llm::models::log::insert_log(&state.pool, log).await {
        tracing::warn!(%err, "llm test log failed");
    }
    Ok(ApiResponse::success(serde_json::json!({
        "ok": ok, "elapsed_ms": elapsed_ms, "detail": detail, "model": model
    })))
}
