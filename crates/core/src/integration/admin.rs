//! Admin API — channel CRUD, receipts/trace queries, test endpoints
//! (integration.md §11). All handlers are `ensure_admin` + scoped to the
//! `integration` domain; credentials never echo back (only `has_credentials`).

use crate::db::driver::DbDriver;
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::integration::channel::{self, ItgChannel};
use crate::integration::dto::{
    ApiClientResponse, ChannelHealthCard, ChannelResponse, CreateApiClientRequest,
    CreateChannelRequest, EgressLogListResponse, ReceiptDetailResponse, ReceiptListResponse,
    ReceiptSummaryResponse, TestCallRequest, TestCallResponse, TestConnectionResponse,
    TestMappingRequest, TestMappingResponse, TraceResponse, UpdateApiClientRequest,
    UpdateChannelRequest,
};
use crate::integration::receipt;
use crate::integration::vault::Vault;
use crate::middleware::auth::{AuthUser, TokenAction};
use crate::utils::pagination::PaginationParams;

// ── Validation ───────────────────────────────────────────────────────

/// Layer-stack legality matrix for this phase (integration.md §2, M1/M2
/// subset). Rejects meaningless or not-yet-supported combinations at save
/// time instead of runtime.
fn validate_stack(req: &CreateChannelRequest) -> Result<(), AppError> {
    const MODE_TRANSPORTS: &[(&str, &[&str])] = &[
        ("push", &["http1", "http2"]),
        #[cfg(feature = "integration-imap")]
        ("pull", &["http1", "http2", "imap"]),
        #[cfg(not(feature = "integration-imap"))]
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
        // Declarative protocol profile (command/type discriminator frames).
        ("dispatch", "json") if req.mode == "stream" && req.transport == "ws" => true,
        // Protobuf envelope frames (pbbp2 wire shape, config semantics).
        ("pb-frame", "json") if req.mode == "stream" && req.transport == "ws" => true,
        // RFC5322/MIME email via the imap connector (integration.md §2).
        #[cfg(feature = "integration-imap")]
        ("mime", "email") if req.transport == "imap" => true,
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
        if req.transport == "imap" {
            // Mark-read semantics: the mailbox is the state — no local
            // cursor, pull_config optional (folder/ssl/batch/idle defaults).
            if req.pull_semantics.as_deref() != Some("mark-read") {
                return Err(AppError::BadRequest(
                    "imap pull requires pull_semantics = 'mark-read'".into(),
                ));
            }
            if !req.endpoint.as_deref().unwrap_or("").starts_with("imap://")
                && !req
                    .endpoint
                    .as_deref()
                    .unwrap_or("")
                    .starts_with("imaps://")
            {
                return Err(AppError::BadRequest(
                    "imap pull requires an imap:// or imaps:// endpoint".into(),
                ));
            }
            return validate_mapping_and_ok(req);
        }
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
    validate_mapping_and_ok(req)
}

fn validate_mapping_and_ok(req: &CreateChannelRequest) -> Result<(), AppError> {
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
) -> AppResult<ApiResponse<Vec<ChannelResponse>>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let rows = channel::model::find_all(&state.pool).await?;
    Ok(ApiResponse::success(
        rows.iter().map(ChannelResponse::from).collect(),
    ))
}

pub async fn create_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateChannelRequest>,
) -> AppResult<ApiResponse<ChannelResponse>> {
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
    Ok(ApiResponse::success(ChannelResponse::from(&ch)))
}

pub async fn update_channel(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateChannelRequest>,
) -> AppResult<ApiResponse<ChannelResponse>> {
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
    if let Some(framing) = &req.framing {
        ch.framing = framing.clone();
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
            "framing" => &ch.framing,
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
    Ok(ApiResponse::success(ChannelResponse::from(&ch)))
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
) -> AppResult<ApiResponse<ChannelResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    Ok(ApiResponse::success(ChannelResponse::from(&ch)))
}

// ── Test endpoints ───────────────────────────────────────────────────

/// POST .../channels/{id}/test-mapping — compile + apply the channel's
/// mapping against a sample body; returns the normalized preview. Zero writes.
pub async fn test_mapping(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TestMappingRequest>,
) -> AppResult<ApiResponse<TestMappingResponse>> {
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
    let resp = match preview {
        Some(n) => TestMappingResponse {
            matched: true,
            external_id: Some(n.external_id),
            sender: n.sender,
            kind: Some(n.kind.as_str().to_string()),
            payload: Some(n.payload),
            reason: None,
        },
        None => TestMappingResponse {
            matched: false,
            external_id: None,
            sender: None,
            kind: None,
            payload: None,
            reason: Some("when-condition not satisfied".into()),
        },
    };
    Ok(ApiResponse::success(resp))
}

/// POST .../channels/{id}/test-connection — pull channels fetch one page
/// (dry: items are NOT routed); push channels echo their verify expectations.
pub async fn test_connection(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<TestConnectionResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    if ch.mode != "pull" {
        return Ok(ApiResponse::success(TestConnectionResponse {
            mode: ch.mode,
            note: Some("push channels verify on first delivery — no connection to test".into()),
            reachable: None,
            status: None,
        }));
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
    Ok(ApiResponse::success(TestConnectionResponse {
        mode: ch.mode,
        note: None,
        reachable: Some(true),
        status: Some(i64::from(resp.status().as_u16())),
    }))
}

// ── Health (P2-M5: supervisor metrics + batch stats + DB status) ─────

fn health_body(
    ch: &ItgChannel,
    sup: Option<&crate::integration::supervisor::ChannelHealth>,
    batch: Option<&crate::integration::batch::BatchStats>,
) -> ChannelHealthCard {
    ChannelHealthCard {
        channel_id: ch.id,
        channel_key: ch.channel_key.clone(),
        mode: ch.mode.clone(),
        transport: ch.transport.clone(),
        enabled: ch.enabled,
        status: ch.status.clone(),
        last_error: ch.last_error.clone(),
        supervisor: sup.cloned(),
        telemetry_batch: batch.cloned(),
    }
}

/// GET /admin/integration/channels/health — aggregate health cards.
pub async fn channels_health(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<ChannelHealthCard>>> {
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
    let cards: Vec<ChannelHealthCard> = channels
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
) -> AppResult<ApiResponse<ChannelHealthCard>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let ch = channel::model::find_by_id(&state.pool, id).await?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let sup = plane.supervisor().and_then(|s| {
        s.health_snapshot()
            .into_iter()
            .find(|h| h.channel_id == id.0)
    });
    let batch = plane.telemetry_batch_stats().get(&id.0).cloned();
    Ok(ApiResponse::success(health_body(
        &ch,
        sup.as_ref(),
        batch.as_ref(),
    )))
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
    // NOTE: not `#[serde(flatten)] PaginationParams` — flatten feeds serde a
    // string-only map and numeric query params (`page_size=3`) stop parsing.
    #[serde(default = "crate::utils::pagination::default_page")]
    pub page: i64,
    #[serde(default = "crate::utils::pagination::default_page_size")]
    pub page_size: i64,
}

/// GET /admin/integration/receipts — papsed, filtered.
pub async fn list_receipts(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<ReceiptListParams>,
) -> AppResult<ApiResponse<ReceiptListResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let mut pagination = PaginationParams::from_options(Some(params.page), Some(params.page_size));
    pagination.sanitize();

    // Typed binds: PG rejects `bigint = text` on loose string binds, so ids
    // bind as i64 and status as text — collected in SQL order.
    #[derive(Default)]
    struct ReceiptBinds {
        channel_id: Option<i64>,
        status: Option<String>,
        trace_id: Option<i64>,
    }
    let mut clauses: Vec<String> = Vec::new();
    let mut binds = ReceiptBinds::default();
    let mut next_ph = 1_usize;
    if let Some(ch) = &params.channel_id {
        let id = crate::types::snowflake_id::parse_id(ch)?;
        clauses.push(format!("channel_id = {}", crate::db::Driver::ph(next_ph)));
        next_ph += 1;
        binds.channel_id = Some(id.0);
    }
    if let Some(status) = &params.status {
        clauses.push(format!("status = {}", crate::db::Driver::ph(next_ph)));
        next_ph += 1;
        binds.status = Some(status.clone());
    }
    if let Some(trace) = &params.trace_id {
        let id = crate::types::snowflake_id::parse_id(trace)?;
        clauses.push(format!("id = {}", crate::db::Driver::ph(next_ph)));
        binds.trace_id = Some(id.0);
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
        let mut q = sqlx::query_scalar::<crate::db::pool::Db, i64>(crate::db::safe_sql(&count_sql));
        if let Some(c) = binds.channel_id {
            q = q.bind(c);
        }
        if let Some(ref s) = binds.status {
            q = q.bind(s);
        }
        if let Some(t) = binds.trace_id {
            q = q.bind(t);
        }
        q.fetch_one(&state.pool).await?
    };

    let page = pagination.page.max(1);
    let page_size = pagination.page_size;
    let offset = (page - 1) * page_size;
    let page_sql = format!("{base} ORDER BY id DESC LIMIT {page_size} OFFSET {offset}");
    // Timestamps decode as `Timestamp` (PG TIMESTAMPTZ rejects String decode).
    let mut q = sqlx::query_as::<
        crate::db::pool::Db,
        (
            i64,
            i64,
            String,
            String,
            String,
            i64,
            Option<crate::utils::tz::Timestamp>,
            Option<String>,
            Option<i64>,
            crate::utils::tz::Timestamp,
            Option<crate::utils::tz::Timestamp>,
        ),
    >(crate::db::safe_sql(&page_sql));
    if let Some(c) = binds.channel_id {
        q = q.bind(c);
    }
    if let Some(ref s) = binds.status {
        q = q.bind(s);
    }
    if let Some(t) = binds.trace_id {
        q = q.bind(t);
    }
    let rows = q.fetch_all(&state.pool).await?;

    let items: Vec<ReceiptSummaryResponse> = rows
        .iter()
        .map(|r| ReceiptSummaryResponse {
            id: crate::types::snowflake_id::SnowflakeId(r.0),
            channel_id: crate::types::snowflake_id::SnowflakeId(r.1),
            external_id: r.2.clone(),
            kind: r.3.clone(),
            status: r.4.clone(),
            attempts: r.5,
            next_retry_at: r.6,
            raw_ref: r.7.clone(),
            target_id: r.8.map(crate::types::snowflake_id::SnowflakeId),
            received_at: r.9,
            delivered_at: r.10,
        })
        .collect();
    Ok(ApiResponse::success(ReceiptListResponse {
        items,
        total: count,
        page,
        page_size,
    }))
}

/// GET /admin/integration/receipts/{id} — full detail (envelope + timeline).
pub async fn get_receipt(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ReceiptDetailResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let row = receipt::find_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("receipt {id} not found")))?;
    let channel = channel::model::find_by_id(&state.pool, row.channel_id)
        .await
        .ok();
    Ok(ApiResponse::success(ReceiptDetailResponse {
        id: row.id,
        channel_id: row.channel_id,
        channel_key: channel.map(|c| c.channel_key),
        external_id: row.external_id,
        kind: row.kind,
        status: row.status,
        attempts: row.attempts,
        next_retry_at: row.next_retry_at,
        envelope: row.envelope,
        steps: row.steps,
        target_id: row.target_id,
    }))
}

/// GET /admin/integration/receipts/{id}/trace — async chain embedded in the
/// step timeline (jobs / retries / replays) plus the egress calls sharing
/// this trace id (`itg_egress_log`, §10.7).
pub async fn get_trace(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<TraceResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let row = receipt::find_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("receipt {id} not found")))?;
    let steps = row.steps.clone().unwrap_or(Value::Array(Vec::new()));
    let arr = steps.as_array().cloned().unwrap_or_default();

    let first_pass: Vec<Value> = arr
        .iter()
        .filter(|s| {
            s["step"].as_str().is_some_and(|n| {
                [
                    "queue",
                    "verify",
                    "normalize",
                    "dedup",
                    "route",
                    "ack",
                    "archive",
                ]
                .contains(&n)
            })
        })
        .cloned()
        .collect();
    let async_chain: Vec<Value> = arr
        .iter()
        .filter(|s| {
            s["step"].as_str().is_some_and(|n| {
                n.starts_with("job:")
                    || n.starts_with("replay#")
                    || n == "pipeline-pass"
                    || n.starts_with("retry#")
            })
        })
        .cloned()
        .collect();
    let pending_count = async_chain
        .iter()
        .filter(|s| s["status"] == "pending")
        .count() as i64;

    let egress = crate::integration::egress::list_log(&state.pool, Some(id), None, 100).await?;

    Ok(ApiResponse::success(TraceResponse {
        trace_id: row.id,
        status: row.status,
        first_pass,
        async_chain,
        pending_count,
        complete: pending_count == 0,
        egress,
    }))
}

// Imports used by handler signatures.
use axum::extract::{Path, Query, State};

// ── API clients (L5 egress, MVP-M0) ──────────────────────────────────

pub async fn list_api_clients(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<ApiClientResponse>>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let rows = crate::integration::api_client::model::find_all(&state.pool).await?;
    Ok(ApiResponse::success(
        rows.iter().map(ApiClientResponse::from).collect(),
    ))
}

pub async fn create_api_client(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateApiClientRequest>,
) -> AppResult<ApiResponse<ApiClientResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Create)?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    crate::integration::api_client::validate(&req.base_url, req.auth.as_ref(), req.ops.as_ref())?;
    if req.client_key.is_empty() || req.client_key.contains('/') {
        return Err(AppError::BadRequest(
            "client_key must be a non-empty path segment (no '/')".into(),
        ));
    }
    let sealed = seal_credentials(plane.vault(), req.credentials.as_ref())?;

    if crate::integration::api_client::model::find_by_key(
        &state.pool,
        crate::constants::DEFAULT_TENANT,
        &req.client_key,
    )
    .await?
    .is_some()
    {
        return Err(AppError::BadRequest(format!(
            "client_key '{}' already exists",
            req.client_key
        )));
    }

    let client = crate::integration::api_client::ItgApiClient {
        id: crate::utils::id::new_snowflake_id(),
        tenant_id: crate::constants::DEFAULT_TENANT.to_string(),
        client_key: req.client_key.clone(),
        display_name: if req.display_name.is_empty() {
            req.client_key.clone()
        } else {
            req.display_name.clone()
        },
        base_url: req.base_url.clone(),
        auth: req.auth.clone(),
        credentials: sealed,
        rate_limit: req.rate_limit.clone(),
        ops: req.ops.clone(),
        enabled: req.enabled,
        created_at: crate::utils::tz::now_utc(),
        updated_at: crate::utils::tz::now_utc(),
    };
    crate::integration::api_client::model::insert(&state.pool, &client).await?;
    Ok(ApiResponse::success(ApiClientResponse::from(&client)))
}

pub async fn get_api_client(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<ApiClientResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;
    Ok(ApiResponse::success(ApiClientResponse::from(&client)))
}

pub async fn update_api_client(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateApiClientRequest>,
) -> AppResult<ApiResponse<ApiClientResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Update)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let mut client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;

    if let Some(name) = &req.display_name {
        client.display_name = name.clone();
    }
    if let Some(base_url) = &req.base_url {
        client.base_url = base_url.clone();
    }
    if let Some(auth_cfg) = &req.auth {
        client.auth = Some(auth_cfg.clone());
    }
    if let Some(rate_limit) = &req.rate_limit {
        client.rate_limit = Some(rate_limit.clone());
    }
    if let Some(ops) = &req.ops {
        client.ops = Some(ops.clone());
    }
    if let Some(creds) = &req.credentials {
        client.credentials = seal_credentials(plane.vault(), Some(creds))?;
    }
    if let Some(enabled) = req.enabled {
        client.enabled = enabled;
    }
    crate::integration::api_client::validate(
        &client.base_url,
        client.auth.as_ref(),
        client.ops.as_ref(),
    )?;
    client.updated_at = crate::utils::tz::now_utc();

    let now = client.updated_at;
    let result = raisfast_derive::crud_update!(
        &state.pool, "itg_api_clients",
        bind: [
            "display_name" => &client.display_name,
            "base_url" => &client.base_url,
            "auth" => client.auth.as_ref(),
            "credentials" => client.credentials.as_deref(),
            "rate_limit" => client.rate_limit.as_ref(),
            "ops" => client.ops.as_ref(),
            "enabled" => client.enabled,
            "updated_at" => now
        ],
        where: ("id", id)
    )?;
    AppError::expect_affected(&result, "itg_api_client")?;
    Ok(ApiResponse::success(ApiClientResponse::from(&client)))
}

pub async fn delete_api_client(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Delete)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    crate::integration::api_client::model::delete_by_id(&state.pool, id).await?;
    Ok(ApiResponse::success(()))
}

/// POST .../api-clients/{id}/test-call — fire one op against the client's
/// real endpoint; the call is logged like any egress (no trace).
pub async fn test_call(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<TestCallRequest>,
) -> AppResult<ApiResponse<TestCallResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let client = crate::integration::api_client::model::find_by_id(&state.pool, id).await?;
    let plane = state
        .integration
        .as_ref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("integration plane disabled")))?;
    let receipt = plane
        .call_api(client.client_key.clone(), req.op, req.input)
        .await?;
    Ok(ApiResponse::success(TestCallResponse {
        status: receipt.status,
        output: receipt.output,
        tokens_in: receipt.tokens_in,
        tokens_out: receipt.tokens_out,
        model: receipt.model,
        log_id: receipt.log_id,
    }))
}

#[derive(Deserialize)]
pub struct EgressLogParams {
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub client_key: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

/// GET /admin/integration/egress-log — outbound call log (filterable by
/// trace_id for the receipt → egress chain).
pub async fn list_egress_log(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<EgressLogParams>,
) -> AppResult<ApiResponse<EgressLogListResponse>> {
    auth.ensure_admin()?;
    auth.ensure_scope("integration", TokenAction::Read)?;
    let trace_id = match &params.trace_id {
        Some(t) => Some(crate::types::snowflake_id::parse_id(t)?),
        None => None,
    };
    let limit = params.limit.unwrap_or(50).min(500);
    let rows = crate::integration::egress::list_log(
        &state.pool,
        trace_id,
        params.client_key.as_deref(),
        limit,
    )
    .await?;
    let count = rows.len() as i64;
    Ok(ApiResponse::success(EgressLogListResponse {
        items: rows,
        count,
    }))
}
