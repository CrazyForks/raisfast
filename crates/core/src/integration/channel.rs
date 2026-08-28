//! ItgChannel — one row per third-party touchpoint: layer-stack combination,
//! trust config, mapping, routing. The config heart of the plane
//! (integration.md §3.1). "Channel is a data row, not code."

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tokio::sync::RwLock;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// Channel row (`itg_channels`).
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct ItgChannel {
    pub id: SnowflakeId,
    pub tenant_id: String,
    /// Human-readable routing key: `/ingress/{channel_key}`. Not named `key` —
    /// MySQL reserved word (glossary §1).
    pub channel_key: String,
    /// Provider preset id (`generic-hmac`, `payment-wechat`, …).
    pub provider: String,
    pub display_name: String,

    // ── Layer stack (§2) ──────────────────────────────────────────
    /// push | pull | stream | listen
    pub mode: String,
    /// http1 | http2 | ws | mqtt | tcp | imap | smtp | sftp
    pub transport: String,
    /// raw | json-rpc | grpc | soap | mime | csv
    pub framing: String,
    /// json | protobuf | xml | email | cbor | csv
    pub codec: String,
    /// Remote address (pull/stream) or local bind (listen).
    pub endpoint: Option<String>,

    // ── L0 trust ──────────────────────────────────────────────────
    /// hmac-sha256 | challenge | token | jwt | mtls | plugin:<id>
    pub verify_kind: String,
    /// Sealed credentials (AES-256-GCM, base64). Never returned via API.
    #[serde(skip_serializing)]
    pub credentials: Option<String>,
    pub verify_config: Option<Value>,

    // ── L2 normalization ──────────────────────────────────────────
    /// Declarative field mapping (JSONPath rules); empty → plugin required.
    pub mapping: Option<Value>,
    pub normalizer_plugin: Option<String>,

    // ── L3 semantics ──────────────────────────────────────────────
    /// pull only: mark-read | cursor | snapshot
    pub pull_semantics: Option<String>,
    /// Pull connector template: list_path / id_field / param / page_size (§5.3).
    pub pull_config: Option<Value>,
    /// Stream/listen connector template: subscribe calls, heartbeat, framing.
    pub stream_config: Option<Value>,
    /// http-200 | puback | rpc-reply | grpc-status | smtp-2xx | none
    pub ack_kind: String,
    pub redelivery_max: i64,
    pub backpressure: Option<Value>,

    // ── L4 routing ────────────────────────────────────────────────
    /// Target content type (`conversation_message`, …).
    pub target_type: String,
    pub route_extra: Option<Value>,

    // ── Runtime state (Supervisor-maintained) ─────────────────────
    /// idle | connecting | connected | degraded | disabled | error
    pub status: String,
    pub last_error: Option<String>,
    pub lease_owner: Option<String>,

    pub enabled: bool,
    /// Dual-run versioning (§10.5): routing resolves max(version) of
    /// enabled AND NOT shadow rows.
    pub version: i64,
    pub shadow: bool,

    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ItgChannel {
    /// Retry philosophy (§6.4): `external` = provider re-delivers (we fail the
    /// ack); default `internal` = self-scheduled `ingress.retry` backoff.
    #[must_use]
    pub fn is_external_retry(&self) -> bool {
        self.route_extra
            .as_ref()
            .and_then(|r| r.get("retry_mode"))
            .and_then(Value::as_str)
            == Some("external")
    }

    /// Telemetry sampling rate in percent, 0-100 (backpressure.sample_rate,
    /// §10.3). 100 (default) keeps everything.
    #[must_use]
    pub fn sample_rate(&self) -> u64 {
        self.backpressure
            .as_ref()
            .and_then(|b| b.get("sample_rate"))
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(100)
    }

    /// `archive-strict` (route_extra): archiving failures fail the pipeline
    /// instead of degrading (§10.2).
    #[must_use]
    pub fn archive_strict(&self) -> bool {
        self.route_extra
            .as_ref()
            .and_then(|r| r.get("archive-strict"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Cache key: tenant + channel_key (versions collapse to the active row).
    fn cache_key(tenant_id: &str, channel_key: &str) -> String {
        format!("{tenant_id}\u{1}{channel_key}")
    }

    /// Resolve the routing row for a key: enabled, not shadow, max version.
    /// Input list is all rows for (tenant, channel_key).
    #[must_use]
    pub fn resolve_active(rows: &[ItgChannel]) -> Option<ItgChannel> {
        rows.iter()
            .filter(|c| c.enabled && !c.shadow)
            .max_by_key(|c| c.version)
            .cloned()
    }
}

/// Model-level CRUD (pool-based; pipeline-owned transactions arrive in M1).
pub mod model {
    use super::ItgChannel;
    use crate::errors::app_error::{AppError, AppResult};
    use crate::types::snowflake_id::SnowflakeId;

    pub async fn insert(pool: &crate::db::Pool, ch: &ItgChannel) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        raisfast_derive::crud_insert!(
            pool,
            "itg_channels",
            [
                "id" => ch.id,
                "channel_key" => &ch.channel_key,
                "provider" => &ch.provider,
                "display_name" => &ch.display_name,
                "mode" => &ch.mode,
                "transport" => &ch.transport,
                "framing" => &ch.framing,
                "codec" => &ch.codec,
                "endpoint" => ch.endpoint.as_deref(),
                "verify_kind" => &ch.verify_kind,
                "verify_config" => ch.verify_config.as_ref(),
                "credentials" => ch.credentials.as_deref(),
                "mapping" => ch.mapping.as_ref(),
                "normalizer_plugin" => ch.normalizer_plugin.as_deref(),
                "pull_semantics" => ch.pull_semantics.as_deref(),
                "pull_config" => ch.pull_config.as_ref(),
                "stream_config" => ch.stream_config.as_ref(),
                "ack_kind" => &ch.ack_kind,
                "redelivery_max" => ch.redelivery_max,
                "backpressure" => ch.backpressure.as_ref(),
                "target_type" => &ch.target_type,
                "route_extra" => ch.route_extra.as_ref(),
                "status" => &ch.status,
                "enabled" => ch.enabled,
                "version" => ch.version,
                "shadow" => ch.shadow,
                "created_at" => now,
                "updated_at" => now
            ],
            tenant: Some(ch.tenant_id.as_str())
        )?;
        Ok(())
    }

    /// All versions of a channel key within a tenant (routing then picks the
    /// active row; dual-run keeps shadow rows queryable).
    pub async fn find_by_key(
        pool: &crate::db::Pool,
        tenant_id: &str,
        channel_key: &str,
    ) -> AppResult<Vec<ItgChannel>> {
        Ok(
            raisfast_derive::crud_find_all!(pool, "itg_channels", ItgChannel,
                where: ("channel_key", channel_key),
                tenant: Some(tenant_id))?,
        )
    }

    pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<ItgChannel> {
        Ok(raisfast_derive::crud_find_one!(pool, "itg_channels", ItgChannel, where: ("id", id))?)
    }

    /// All rows (every version, every state) — cache refresh input.
    /// Hand-written SQL: `crud_find_all!` requires a `where:` section and we
    /// genuinely want the full table (channels are few).
    pub async fn find_all(pool: &crate::db::Pool) -> AppResult<Vec<ItgChannel>> {
        const CHANNEL_COLS: &str = "id, tenant_id, channel_key, provider, display_name, mode, \
             transport, framing, codec, endpoint, verify_kind, verify_config, credentials, \
             mapping, normalizer_plugin, pull_semantics, pull_config, stream_config, \
             ack_kind, redelivery_max, backpressure, target_type, route_extra, status, \
             last_error, lease_owner, enabled, version, shadow, created_at, updated_at";
        let sql = format!("SELECT {CHANNEL_COLS} FROM itg_channels");
        let rows: Vec<ItgChannel> =
            sqlx::query_as::<crate::db::pool::Db, ItgChannel>(crate::db::safe_sql(&sql))
                .fetch_all(pool)
                .await?;
        Ok(rows)
    }

    /// Flip runtime status (Supervisor path; CAS-free, single writer per channel).
    pub async fn update_status(
        pool: &crate::db::Pool,
        id: SnowflakeId,
        status: &str,
        last_error: Option<&str>,
    ) -> AppResult<()> {
        let now = crate::utils::tz::now_utc();
        let result = raisfast_derive::crud_update!(
            pool, "itg_channels",
            bind: ["status" => status, "last_error" => last_error, "updated_at" => now],
            where: ("id", id)
        )?;
        AppError::expect_affected(&result, "itg_channel")?;
        Ok(())
    }

    pub async fn delete_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
        let result = raisfast_derive::crud_delete!(pool, "itg_channels", where: ("id", id))?;
        AppError::expect_affected(&result, "itg_channel")?;
        Ok(())
    }
}

/// Cached channel store: hot lookups for the ingress endpoints.
///
/// Cache is primed at startup and refreshed on mutation (admin CRUD) —
/// channels are few and config-changes are rare, so read-mostly.
pub struct ItgChannelStore {
    pool: crate::db::Pool,
    cache: RwLock<HashMap<String, Arc<ItgChannel>>>,
}

impl ItgChannelStore {
    /// Create an empty store (call [`Self::refresh`] before first use).
    #[must_use]
    pub fn new(pool: crate::db::Pool) -> Self {
        Self {
            pool,
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Reload the active-row cache from the DB (startup + after admin writes).
    ///
    /// # Errors
    ///
    /// Returns `AppError` on query failure.
    pub async fn refresh(&self) -> AppResult<()> {
        let all = model::find_all(&self.pool).await?;
        let mut grouped: HashMap<String, Vec<ItgChannel>> = HashMap::new();
        for ch in all {
            grouped
                .entry(ItgChannel::cache_key(&ch.tenant_id, &ch.channel_key))
                .or_default()
                .push(ch);
        }
        let mut cache = self.cache.write().await;
        cache.clear();
        for (key, rows) in grouped {
            if let Some(active) = ItgChannel::resolve_active(&rows) {
                cache.insert(key, Arc::new(active));
            }
        }
        Ok(())
    }

    /// Resolve the routing row for a tenant + channel key (cached).
    ///
    /// # Errors
    ///
    /// `AppError::NotFound` when no enabled non-shadow version exists.
    pub async fn get(&self, tenant_id: &str, channel_key: &str) -> AppResult<Arc<ItgChannel>> {
        self.cache
            .read()
            .await
            .get(&ItgChannel::cache_key(tenant_id, channel_key))
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "integration channel '{channel_key}' not found or disabled"
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::id::new_snowflake_id;

    fn channel(version: i64, enabled: bool, shadow: bool) -> ItgChannel {
        ItgChannel {
            id: new_snowflake_id(),
            tenant_id: "default".into(),
            channel_key: "test-ch".into(),
            provider: "generic-hmac".into(),
            display_name: "Test".into(),
            mode: "push".into(),
            transport: "http1".into(),
            framing: "raw".into(),
            codec: "json".into(),
            endpoint: None,
            verify_kind: "hmac-sha256".into(),
            verify_config: None,
            credentials: None,
            mapping: None,
            normalizer_plugin: None,
            pull_semantics: None,
            pull_config: None,
            stream_config: None,
            ack_kind: "http-200".into(),
            redelivery_max: 5,
            backpressure: None,
            target_type: "conversation_message".into(),
            route_extra: None,
            status: "idle".into(),
            last_error: None,
            lease_owner: None,
            enabled,
            version,
            shadow,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    #[test]
    fn resolve_active_picks_max_enabled_non_shadow() {
        let rows = vec![
            channel(1, true, false),
            channel(2, true, false),
            channel(3, true, true),   // shadow: never routed
            channel(4, false, false), // disabled: never routed
        ];
        let active = ItgChannel::resolve_active(&rows).expect("active row exists");
        assert_eq!(active.version, 2);
    }

    #[test]
    fn resolve_active_none_when_only_shadow() {
        let rows = vec![channel(3, true, true)];
        assert!(ItgChannel::resolve_active(&rows).is_none());
    }

    #[test]
    fn cache_key_separates_tenants() {
        assert_ne!(
            ItgChannel::cache_key("a", "k"),
            ItgChannel::cache_key("b", "k")
        );
    }
}
