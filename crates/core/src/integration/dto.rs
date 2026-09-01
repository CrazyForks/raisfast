//! Integration admin DTOs — typed wire shapes for the admin API (§11) and
//! the TS export consumed by the admin SPA. Credentials never appear in
//! responses; only the `has_credentials` boolean.

use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::integration::api_client::ItgApiClient;
use crate::integration::batch::BatchStats;
use crate::integration::channel::ItgChannel;
use crate::integration::egress::EgressLogRow;
use crate::integration::supervisor::ChannelHealth;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

// ── Channel requests ─────────────────────────────────────────────────

/// POST /admin/integration/channels body.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    pub channel_key: String,
    /// App ownership (channel-app-ownership.md §2). Platform admin may set an
    /// installed app_id (or omit for a platform/global channel). Plugin host
    /// API derives this from the caller's manifest instead of trusting input.
    pub app_id: Option<String>,
    pub provider: String,
    #[serde(default)]
    pub display_name: String,
    pub mode: String,
    pub transport: String,
    pub framing: String,
    pub codec: String,
    pub endpoint: Option<String>,
    pub verify_kind: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub verify_config: Option<Value>,
    /// Plaintext credentials JSON — sealed into the vault on write.
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub credentials: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub mapping: Option<Value>,
    pub pull_semantics: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub pull_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub stream_config: Option<Value>,
    #[serde(default = "default_redelivery_max")]
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub redelivery_max: i64,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub backpressure: Option<Value>,
    pub target_type: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub route_extra: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// PUT /admin/integration/channels/{id} body (partial update).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub display_name: Option<String>,
    pub endpoint: Option<String>,
    /// Protocol switch (raw|json-rpc|dispatch|pb-frame) — changing the wire
    /// protocol of a channel is a legit ops action.
    pub framing: Option<String>,
    pub verify_kind: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub verify_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub credentials: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub mapping: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub pull_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub stream_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub redelivery_max: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub backpressure: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub route_extra: Option<Value>,
    pub enabled: Option<bool>,
}

fn default_redelivery_max() -> i64 {
    5
}

fn default_true() -> bool {
    true
}

fn default_empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

// ── Channel responses ────────────────────────────────────────────────

/// Channel row as returned by the admin API — every `itg_channels` field
/// except `credentials`, plus `has_credentials`.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ChannelResponse {
    pub id: SnowflakeId,
    pub tenant_id: String,
    /// App ownership: NULL = platform/global channel (channel-app-ownership.md §2).
    pub app_id: Option<String>,
    /// Human-readable routing key: `/ingress/{channel_key}`.
    pub channel_key: String,
    pub provider: String,
    pub display_name: String,
    pub mode: String,
    pub transport: String,
    pub framing: String,
    pub codec: String,
    pub endpoint: Option<String>,
    pub verify_kind: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub verify_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub mapping: Option<Value>,
    pub normalizer_plugin: Option<String>,
    pub pull_semantics: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub pull_config: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub stream_config: Option<Value>,
    pub ack_kind: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub redelivery_max: i64,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub backpressure: Option<Value>,
    pub target_type: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub route_extra: Option<Value>,
    /// idle | connecting | connected | degraded | disabled | error.
    pub status: String,
    pub last_error: Option<String>,
    pub lease_owner: Option<String>,
    pub enabled: bool,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version: i64,
    pub shadow: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Sealed credentials exist but are never echoed back.
    pub has_credentials: bool,
}

impl From<&ItgChannel> for ChannelResponse {
    fn from(ch: &ItgChannel) -> Self {
        Self {
            id: ch.id,
            tenant_id: ch.tenant_id.clone(),
            app_id: ch.app_id.clone(),
            channel_key: ch.channel_key.clone(),
            provider: ch.provider.clone(),
            display_name: ch.display_name.clone(),
            mode: ch.mode.clone(),
            transport: ch.transport.clone(),
            framing: ch.framing.clone(),
            codec: ch.codec.clone(),
            endpoint: ch.endpoint.clone(),
            verify_kind: ch.verify_kind.clone(),
            verify_config: ch.verify_config.clone(),
            mapping: ch.mapping.clone(),
            normalizer_plugin: ch.normalizer_plugin.clone(),
            pull_semantics: ch.pull_semantics.clone(),
            pull_config: ch.pull_config.clone(),
            stream_config: ch.stream_config.clone(),
            ack_kind: ch.ack_kind.clone(),
            redelivery_max: ch.redelivery_max,
            backpressure: ch.backpressure.clone(),
            target_type: ch.target_type.clone(),
            route_extra: ch.route_extra.clone(),
            status: ch.status.clone(),
            last_error: ch.last_error.clone(),
            lease_owner: ch.lease_owner.clone(),
            enabled: ch.enabled,
            version: ch.version,
            shadow: ch.shadow,
            created_at: ch.created_at,
            updated_at: ch.updated_at,
            has_credentials: ch.credentials.is_some(),
        }
    }
}

/// One channel health card (DB status + supervisor metrics + batch stats).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ChannelHealthCard {
    pub channel_id: SnowflakeId,
    pub channel_key: String,
    pub mode: String,
    pub transport: String,
    pub enabled: bool,
    pub status: String,
    pub last_error: Option<String>,
    pub supervisor: Option<ChannelHealth>,
    pub telemetry_batch: Option<BatchStats>,
}

/// POST .../channels/{id}/test-mapping body.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct TestMappingRequest {
    /// Sample raw body (JSON text).
    pub sample: String,
}

/// test-mapping preview — the normalized envelope the pipeline would route.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct TestMappingResponse {
    pub matched: bool,
    pub external_id: Option<String>,
    pub sender: Option<String>,
    pub kind: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub payload: Option<Value>,
    pub reason: Option<String>,
}

/// test-connection result — pull channels probe reachability; others carry a
/// note explaining there is nothing to dial.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct TestConnectionResponse {
    pub mode: String,
    pub note: Option<String>,
    pub reachable: Option<bool>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub status: Option<i64>,
}

// ── Receipts & trace ─────────────────────────────────────────────────

/// Receipt row as shown in the admin list (no envelope snapshot).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ReceiptSummaryResponse {
    pub id: SnowflakeId,
    pub channel_id: SnowflakeId,
    pub external_id: String,
    pub kind: String,
    /// received | retrying | delivered | dead | duplicate.
    pub status: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub attempts: i64,
    pub next_retry_at: Option<Timestamp>,
    pub raw_ref: Option<String>,
    pub target_id: Option<SnowflakeId>,
    pub received_at: Timestamp,
    pub delivered_at: Option<Timestamp>,
}

/// GET /admin/integration/receipts payload.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ReceiptListResponse {
    pub items: Vec<ReceiptSummaryResponse>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub total: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub page: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub page_size: i64,
}

/// GET /admin/integration/receipts/{id} — full detail with the envelope
/// snapshot and the step timeline.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ReceiptDetailResponse {
    pub id: SnowflakeId,
    pub channel_id: SnowflakeId,
    pub channel_key: Option<String>,
    pub external_id: String,
    pub kind: String,
    pub status: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub attempts: i64,
    pub next_retry_at: Option<Timestamp>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub envelope: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub steps: Option<Value>,
    pub target_id: Option<SnowflakeId>,
}

/// GET /admin/integration/receipts/{id}/trace — lifecycle replay view:
/// first-pass steps, async chain (jobs/retries/replays) and the egress
/// calls sharing the trace id.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct TraceResponse {
    pub trace_id: SnowflakeId,
    pub status: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown[]"))]
    pub first_pass: Vec<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown[]"))]
    pub async_chain: Vec<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub pending_count: i64,
    /// "Done" = no pending async steps (§10.7 completion rule).
    pub complete: bool,
    pub egress: Vec<EgressLogRow>,
}

/// POST /admin/integration/receipts/{id}/replay result.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ReplayResponse {
    pub replayed: bool,
    /// upsert | dry-run.
    pub mode: String,
    pub target_id: Option<SnowflakeId>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub report: Option<Value>,
}

// ── API clients (L5 egress) ──────────────────────────────────────────

/// POST /admin/integration/api-clients body.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct CreateApiClientRequest {
    pub client_key: String,
    #[serde(default)]
    pub display_name: String,
    pub base_url: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub auth: Option<Value>,
    /// Plaintext credentials JSON `{"secret": "..."}` — sealed on write.
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub credentials: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub rate_limit: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub ops: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// PUT /admin/integration/api-clients/{id} body (partial update).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct UpdateApiClientRequest {
    pub display_name: Option<String>,
    pub base_url: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub auth: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub credentials: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub rate_limit: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub ops: Option<Value>,
    pub enabled: Option<bool>,
}

/// API client row as returned by the admin API — every `itg_api_clients`
/// field except `credentials`, plus `has_credentials`.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct ApiClientResponse {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub client_key: String,
    pub display_name: String,
    pub base_url: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub auth: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub rate_limit: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub ops: Option<Value>,
    pub enabled: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub has_credentials: bool,
}

impl From<&ItgApiClient> for ApiClientResponse {
    fn from(c: &ItgApiClient) -> Self {
        Self {
            id: c.id,
            tenant_id: c.tenant_id.clone(),
            client_key: c.client_key.clone(),
            display_name: c.display_name.clone(),
            base_url: c.base_url.clone(),
            auth: c.auth.clone(),
            rate_limit: c.rate_limit.clone(),
            ops: c.ops.clone(),
            enabled: c.enabled,
            created_at: c.created_at,
            updated_at: c.updated_at,
            has_credentials: c.credentials.is_some(),
        }
    }
}

/// POST .../api-clients/{id}/test-call body.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct TestCallRequest {
    pub op: String,
    #[serde(default = "default_empty_object")]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub input: Value,
}

/// test-call result — one real egress op (logged like any egress).
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct TestCallResponse {
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub status: u16,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub output: Value,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub tokens_in: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub tokens_out: Option<i64>,
    pub model: Option<String>,
    pub log_id: SnowflakeId,
}

/// GET /admin/integration/egress-log payload.
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct EgressLogListResponse {
    pub items: Vec<EgressLogRow>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub count: i64,
}

#[cfg(feature = "export-types")]
crate::export_types!(
    CreateChannelRequest,
    UpdateChannelRequest,
    ChannelResponse,
    ChannelHealthCard,
    TestMappingRequest,
    TestMappingResponse,
    TestConnectionResponse,
    ReceiptSummaryResponse,
    ReceiptListResponse,
    ReceiptDetailResponse,
    TraceResponse,
    ReplayResponse,
    CreateApiClientRequest,
    UpdateApiClientRequest,
    ApiClientResponse,
    TestCallRequest,
    TestCallResponse,
    EgressLogListResponse,
    ChannelHealth,
    BatchStats,
    EgressLogRow,
);
