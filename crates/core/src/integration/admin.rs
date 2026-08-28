//! Admin API — channel CRUD, receipts/trace queries, test endpoints
//! (integration.md §11). All handlers are `ensure_admin` + scoped to the
//! `integration` domain; credentials never echo back (only `has_credentials`).

use axum::Json;
use serde::Deserialize;
use serde_json::{Value, json};
use crate::db::driver::DbDriver;

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::integration::channel::{self, ItgChannel};
use crate::integration::receipt;
use crate::integration::vault::Vault;
use crate::middleware::auth::{AuthUser, TokenAction};
use crate::utils::pagination::PaginationParams;
use crate::AppState;

// ── DTOs ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateChannelRequest {
    pub channel_key: String,
    pub provider: String,
    #[serde(default)]
    pub display_name: String,
    pub mode: String,
    pub transport: String,
    pub framing: String,
    pub codec: String,
    pub endpoint: Option<String>,
    pub verify_kind: String,
    pub verify_config: Option<Value>,
    /// Plaintext credentials JSON — sealed into the vault on write.
    pub credentials: Option<Value>,
    pub mapping: Option<Value>,
    pub pull_semantics: Option<String>,
    pub pull_config: Option<Value>,
    pub stream_config: Option<Value>,
    #[serde(default = "default_redelivery_max")]
    pub redelivery_max: i64,
    pub backpressure: Option<Value>,
    pub target_type: String,
    pub route_extra: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_redelivery_max() -> i64 {
    5
}

fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct UpdateChannelRequest {
    pub display_name: Option<String>,
    pub endpoint: Option<String>,
    pub verify_kind: Option<String>,
    pub verify_config: Option<Value>,
    pub credentials: Option<Value>,
    pub mapping: Option<Value>,
    pub pull_config: Option<Value>,
    pub stream_config: Option<Value>,
    pub redelivery_max: Option<i64>,
    pub backpressure: Option<Value>,
    pub route_extra: Option<Value>,
    pub enabled: Option<bool>,
}

// ── Serialization (never leaks credentials) ──────────────────────────

fn channel_to_json(ch: &ItgChannel) -> Value {
    let mut v = serde_json::to_value(ch).unwrap_or(Value::Null);
    if let Some(obj) = v.as_object_mut() {
        obj.remove("credentials");
        obj.insert(
            "has_credentials".into(),
            Value::Bool(ch.credentials.is_some()),
        );
    }
    v
}

// ── Validation ───────────────────────────────────────────────────────

/// Layer-stack legality matrix for this phase (integration.md §2, M1/M2
/// subset). Rejects meaningless or not-yet-supported combinations at save
/// time instead of runtime.
fn validate_stack(req: &CreateChannelRequest) -> Result<(), AppError> {
    const MODE_TRANSPORTS: &[(&str, &[&str])] = &[
        ("push", &["http1", "http2"]),
        ("pull", &["http1", "http2"]),
        ("stream", &["ws", "mqtt"]),
        ("listen", &["tcp"]),
    ];
    let allowed = MODE_TRANSPORTS
        .iter()
        .find(|(m, _)| *m == req.mode)
        .map(|(_, ts)| *ts);
    let Some(allowed) = allowed else {
        return Err(AppError::BadRequest(format!(
            "mode '{}' not supported (push | pull | stream | listen)",
            req.mode
        )));
    };
    if !allowed.contains(&req.transport.as_str()) {
        return Err(AppError::BadRequest(format!(
            "mode '{}' allows transports {:?} — got '{}'",
            req.mode, allowed, req.transport
        )));
    }
    let framing_ok = match (req.framing.as_str(), req.codec.as_str()) {
        ("raw", "json") => true,
        ("json-rpc", "json") if req.mode == "stream" && req.transport == "ws" => true,
        _ => false,
    };
    if !framing_ok {
        return Err(AppError::BadRequest(
            "framing+codec must be raw+json (or json-rpc+json on ws stream)".into(),
        ));
    }
    let supported_verify = ["hmac-sha256", "token", "challenge", "none"];
    if !supported_verify.contains(&req.verify_kind.as_str()) {
        return Err(AppError::BadRequest(format!(
            "verify_kind '{}' not supported (hmac-sha256 | token | challenge | none)",
            req.verify_kind
        )));
    }
    if req.channel_key.contains('/') || req.channel_key.is_empty() {
        return Err(AppError::BadRequest(
            "channel_key must be a non-empty path segment (no '/')".into(),
        ));
    }
    if req.mode == "stream" {
        if req.endpoint.as_deref().unwrap_or("").is_empty() {
            return Err(AppError::BadRequest(
                "stream requires a ws:// or mqtts:// endpoint".into(),
            ));
        }
        if req.stream_config.is_none() {
            return Err(AppError::BadRequest(
                "stream requires stream_config (subscribe/heartbeat/topics)".into(),
            ));
        }
    }
    if req.mode == "listen" && req.endpoint.as_deref().unwrap_or("").is_empty() {
        return Err(AppError::BadRequest(
            "listen requires a host:port bind endpoint".into(),
        ));
    }
    if req.mode == "pull" {
        if req.pull_semantics.as_deref() != Some("cursor") {
            return Err(AppError::BadRequest(
                "pull requires pull_semantics = 'cursor' in this phase".into(),
            ));
        }
        if req.endpoint.as_deref().unwrap_or("").is_empty() {
            return Err(AppError::BadRequest(
                "pull requires an http(s) endpoint".into(),
            ));
        }
        if req.pull_config.is_none() {
            return Err(AppError::BadRequest(
                "pull requires pull_config (list_path / id_field / param)".into(),
            ));
        }
    }
    if let Some(mapping) = &req.mapping {
        crate::integration::mapping::compile(mapping)?;
    }
    Ok(())
}

fn seal_credentials(
    vault: Option<&Vault>,
    credentials: Option<&Value>,
) -> Result<Option<String>, AppError> {
    let Some(creds) = credentials else {
        return Ok(None);
    };
    let Some(vault) = vault else {
        return Err(AppError::BadRequest(
            "credentials provided but vault sealed — set INTEGRATION_VAULT_KEY".into(),
        ));
    };
    Ok(Some(vault.seal(&creds.to_string())?))
}

// ── Channel CRUD ─────────────────────────────────────────────────────

pub async fn list_channels(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<Value>>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let rows = channel::model::find_all(&state.pool).await?;
    Ok(ApiResponse::success(
        rows.iter().map(channel_to_json).collect(),
    ))
}

pub async fn create_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateChannelRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Create)?;
    validate_stack(&req)?;

    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let sealed = seal_credentials(plane.vault(), req.credentials.as_ref())?;

    // Active-row uniqueness: reject when an enabled non-shadow version of the
    // key already exists in this tenant (dual-run adds version rows later).
    let existing = channel::model::find_by_key(
        &state.pool,
        crate::constants::DEFAULT_TENANT,
        &req.channel_key,
    )
    .await?;
    if ItgChannel::resolve_active(&existing).is_some() {
        return Err(AppError::BadRequest(format!(
            "channel_key '{}' already has an active version in the default tenant",
            req.channel_key
        )));
    }
    let next_version = existing.iter().map(|c| c.version).max().unwrap_or(0) + 1;

    let now = crate::utils::tz::now_utc();
    let ch = ItgChannel {
        id: crate::utils::id::new_snowflake_id(),
        tenant_id: crate::constants::DEFAULT_TENANT.to_string(),
        channel_key: req.channel_key.clone(),
        provider: req.provider.clone(),
        display_name: if req.display_name.is_empty() {
            req.channel_key.clone()
        } else {
            req.display_name.clone()
        },
        mode: req.mode.clone(),
        transport: req.transport.clone(),
        framing: req.framing.clone(),
        codec: req.codec.clone(),
        endpoint: req.endpoint.clone(),
        verify_kind: req.verify_kind.clone(),
        verify_config: req.verify_config.clone(),
        credentials: sealed,
        mapping: req.mapping.clone(),
        normalizer_plugin: None,
        pull_semantics: req.pull_semantics.clone(),
        pull_config: req.pull_config.clone(),
        stream_config: req.stream_config.clone(),
        ack_kind: if req.mode == "pull" {
            "none".to_string()
        } else {
            "http-200".to_string()
        },
        redelivery_max: req.redelivery_max,
        backpressure: req.backpressure.clone(),
        target_type: req.target_type.clone(),
        route_extra: req.route_extra.clone(),
        status: "idle".to_string(),
        last_error: None,
        lease_owner: None,
        enabled: req.enabled,
        version: next_version,
        shadow: false,
        created_at: now,
        updated_at: now,
    };
    channel::model::insert(&state.pool, &ch).await?;
    plane.channels().refresh().await?;
    plane.wake_supervisor();
    Ok(ApiResponse::success(channel_to_json(&ch)))
}

pub async fn update_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Update)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let mut ch = channel::model::find_by_id(&state.pool, id).await?;

    if let Some(name) = &req.display_name {
        ch.display_name = name.clone();
    }
    if let Some(endpoint) = &req.endpoint {
        ch.endpoint = Some(endpoint.clone());
    }
    if let Some(kind) = &req.verify_kind {
        ch.verify_kind = kind.clone();
    }
    if let Some(config) = &req.verify_config {
        ch.verify_config = Some(config.clone());
    }
    if let Some(mapping) = &req.mapping {
        crate::integration::mapping::compile(mapping)?;
        ch.mapping = Some(mapping.clone());
    }
    if let Some(pull_config) = &req.pull_config {
        ch.pull_config = Some(pull_config.clone());
    }
    if let Some(stream_config) = &req.stream_config {
        ch.stream_config = Some(stream_config.clone());
    }
    if let Some(max) = req.redelivery_max {
        ch.redelivery_max = max;
    }
    if let Some(bp) = &req.backpressure {
        ch.backpressure = Some(bp.clone());
    }
    if let Some(extra) = &req.route_extra {
        ch.route_extra = Some(extra.clone());
    }
    if let Some(creds) = &req.credentials {
        ch.credentials = seal_credentials(plane.vault(), Some(creds))?;
    }
    if let Some(enabled) = req.enabled {
        ch.enabled = enabled;
    }
    ch.updated_at = crate::utils::tz::now_utc();

    let now = ch.updated_at;
    let result = raisfast_derive::crud_update!(
        &state.pool, "itg_channels",
        bind: [
            "display_name" => &ch.display_name,
            "endpoint" => ch.endpoint.as_deref(),
            "verify_kind" => &ch.verify_kind,
            "verify_config" => ch.verify_config.as_ref(),
            "credentials" => ch.credentials.as_deref(),
            "mapping" => ch.mapping.as_ref(),
            "pull_config" => ch.pull_config.as_ref(),
            "stream_config" => ch.stream_config.as_ref(),
            "redelivery_max" => ch.redelivery_max,
            "backpressure" => ch.backpressure.as_ref(),
            "route_extra" => ch.route_extra.as_ref(),
            "enabled" => ch.enabled,
            "updated_at" => now
        ],
        where: ("id", id)
    )?;
    AppError::expect_affected(&result, "itg_channel")?;
    plane.channels().refresh().await?;
    plane.wake_supervisor();
    Ok(ApiResponse::success(channel_to_json(&ch)))
}

pub async fn delete_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Delete)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    channel::model::delete_by_id(&state.pool, id).await?;
    if let Some(plane) = state.integration.as_ref() {
        plane.channels().refresh().await?;
        plane.wake_supervisor();
    }
    Ok(ApiResponse::success(()))
}

/// GET .../channels/{id} — single channel detail (no credentials).
pub async fn get_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    Ok(ApiResponse::success(channel_to_json(&ch)))
}

// ── Test endpoints ───────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TestMappingRequest {
    /// Sample raw body (JSON text).
    pub sample: String,
}

/// POST .../channels/{id}/test-mapping — compile + apply the channel's
/// mapping against a sample body; returns the normalized preview. Zero writes.
pub async fn test_mapping(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TestMappingRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    let input = crate::integration::framing::decode(&ch.framing, &ch.codec, req.sample.as_bytes())?;
    let Some(mapping_def) = &ch.mapping else {
        return Err(AppError::BadRequest(
            "channel has no mapping to test — configure it first".into(),
        ));
    };
    let plan = crate::integration::mapping::compile(mapping_def)?;
    let preview = plan.apply(&input)?;
    let preview = match preview {
        Some(n) => json!({
            "matched": true,
            "external_id": n.external_id,
            "sender": n.sender,
            "kind": n.kind.as_str(),
            "payload": n.payload,
        }),
        None => json!({"matched": false, "reason": "when-condition not satisfied"}),
    };
    Ok(ApiResponse::success(preview))
}

/// POST .../channels/{id}/test-connection — pull channels fetch one page
/// (dry: items are NOT routed); push channels echo their verify expectations.
pub async fn test_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    if ch.mode != "pull" {
        return Ok(ApiResponse::success(json!({
            "mode": ch.mode,
            "note": "push channels verify on first delivery — no connection to test",
        })));
    }
    let Some(endpoint) = ch.endpoint.as_deref() else {
        return Err(AppError::BadRequest("pull channel without endpoint".into()));
    };
    let url = reqwest::Url::parse(endpoint)
        .map_err(|e| AppError::BadRequest(format!("invalid endpoint: {e}")))?;
    let resp = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("http client: {e}")))?
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("connection failed: {e}")))?;
    Ok(ApiResponse::success(json!({
        "reachable": true,
        "status": resp.status().as_u16(),
    })))
}

// ── Health (P2-M5: supervisor metrics + batch stats + DB status) ─────

fn health_body(
    ch: &ItgChannel,
    sup: Option<&crate::integration::supervisor::ChannelHealth>,
    batch: Option<&crate::integration::batch::BatchStats>,
) -> Value {
    json!({
        "channel_id": ch.id, "channel_key": ch.channel_key,
        "mode": ch.mode, "transport": ch.transport,
        "enabled": ch.enabled, "status": ch.status,
        "last_error": ch.last_error,
        "supervisor": sup,
        "telemetry_batch": batch,
    })
}

/// GET /admin/integration/channels/health — aggregate health cards.
pub async fn channels_health(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<Value>>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let channels = channel::model::find_all(&state.pool).await?;
    let sup = plane.supervisor().map(|s| s.health_snapshot());
    let sup_map: std::collections::HashMap<i64, _> = sup
        .map(|v| v.into_iter().map(|h| (h.channel_id, h)).collect())
        .unwrap_or_default();
    let batch_map = plane.telemetry_batch_stats();
    let cards: Vec<Value> = channels
        .iter()
        .map(|ch| health_body(ch, sup_map.get(&ch.id.0), batch_map.get(&ch.id.0)))
        .collect();
    Ok(ApiResponse::success(cards))
}

/// GET /admin/integration/channels/{id}/health — single channel detail.
pub async fn channel_health(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let sup = plane
        .supervisor()
        .and_then(|s| s.health_snapshot().into_iter().find(|h| h.channel_id == id.0));
    let batch = plane.telemetry_batch_stats().get(&id.0).cloned();
    Ok(ApiResponse::success(health_body(&ch, sup.as_ref(), batch.as_ref())))
}

// ── Receipts & trace ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ReceiptListParams {
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(flatten)]
    pub pagination: PaginationParams,
}

/// GET /admin/integration/receipts — papsed, filtered.
pub async fn list_receipts(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<ReceiptListParams>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    params.pagination.sanitize();

    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(ch) = &params.channel_id {
        let id = crate::types::snowflake_id::parse_id(ch)?;
        clauses.push(format!("channel_id = {}", crate::db::Driver::ph(binds.len() + 1)));
        binds.push(id.to_string());
    }
    if let Some(status) = &params.status {
        clauses.push(format!("status = {}", crate::db::Driver::ph(binds.len() + 1)));
        binds.push(status.clone());
    }
    if let Some(trace) = &params.trace_id {
        let id = crate::types::snowflake_id::parse_id(trace)?;
        clauses.push(format!("id = {}", crate::db::Driver::ph(binds.len() + 1)));
        binds.push(id.to_string());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };

    let base = format!(
        "SELECT id, channel_id, external_id, kind, status, attempts, next_retry_at, \
         raw_ref, target_id, received_at, delivered_at FROM itg_receipts{where_sql}"
    );
    let count_sql = format!("SELECT COUNT(*) FROM itg_receipts{where_sql}");

    let count: i64 = {
        let mut q = sqlx::query_scalar::<crate::db::pool::Db, i64>(
            crate::db::safe_sql(&count_sql),
        );
        for b in &binds {
            q = q.bind(b);
        }
        q.fetch_one(&state.pool).await?
    };

    let page = params.pagination.page.max(1);
    let page_size = params.pagination.page_size;
    let offset = (page - 1) * page_size;
    let page_sql = format!(
        "{base} ORDER BY id DESC LIMIT {page_size} OFFSET {offset}"
    );
    let mut q = sqlx::query_as::<crate::db::pool::Db, (i64, i64, String, String, String, i64, Option<String>, Option<String>, Option<i64>, String, Option<String>)>(
        crate::db::safe_sql(&page_sql),
    );
    for b in &binds {
        q = q.bind(b);
    }
    let rows = q.fetch_all(&state.pool).await?;

    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.0, "channel_id": r.1, "external_id": r.2, "kind": r.3,
                "status": r.4, "attempts": r.5, "next_retry_at": r.6,
                "raw_ref": r.7, "target_id": r.8, "received_at": r.9, "delivered_at": r.10,
            })
        })
        .collect();
    Ok(ApiResponse::success(json!({
        "items": items,
        "total": count,
        "page": page,
        "page_size": page_size,
    })))
}

/// GET /admin/integration/receipts/{id} — full detail (envelope + timeline).
pub async fn get_receipt(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let row = receipt::find_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("receipt {id} not found")))?;
    let channel = channel::model::find_by_id(&state.pool, row.channel_id).await.ok();
    Ok(ApiResponse::success(json!({
        "id": row.id, "channel_id": row.channel_id,
        "channel_key": channel.map(|c| c.channel_key),
        "external_id": row.external_id,
        "kind": row.kind,
        "status": row.status,
        "attempts": row.attempts,
        "next_retry_at": row.next_retry_at,
        "envelope": row.envelope,
        "steps": row.steps,
        "target_id": row.target_id,
    })))
}

/// GET /admin/integration/receipts/{id}/trace — async chain embedded in the
/// step timeline (jobs / retries / replays). `itg_egress_log` joins in P3.
pub async fn get_trace(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let row = receipt::find_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("receipt {id} not found")))?;
    let steps = row.steps.clone().unwrap_or(Value::Array(Vec::new()));
    let arr = steps.as_array().cloned().unwrap_or_default();

    let first_pass: Vec<&Value> = arr
        .iter()
        .filter(|s| {
            s["step"]
                .as_str()
                .is_some_and(|n| ["queue", "verify", "normalize", "dedup", "route", "ack", "archive"]
                    .contains(&n))
        })
        .collect();
    let async_chain: Vec<&Value> = arr
        .iter()
        .filter(|s| {
            s["step"].as_str().is_some_and(|n| {
                n.starts_with("job:") || n.starts_with("replay#") || n == "pipeline-pass"
                    || n.starts_with("retry#")
            })
        })
        .collect();
    let pending: Vec<&Value> = async_chain
        .iter()
        .copied()
        .filter(|s| s["status"] == "pending")
        .collect();

    Ok(ApiResponse::success(json!({
        "trace_id": row.id.0,
        "status": row.status,
        "first_pass": first_pass,
        "async_chain": async_chain,
        "pending_count": pending.len(),
        "complete": pending.is_empty(),
    })))
}

// Imports used by handler signatures.
use axum::extract::{Path, Query, State};
