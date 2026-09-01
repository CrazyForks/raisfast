//! Integration Plane — unified boundary between RaisFast and third-party systems.
//!
//! Design doc: `dev-docs/integration/integration.md` (authoritative; naming per
//! `dev-docs/integration/glossary.md`).
//!
//! Composition:
//! - [`envelope`] — `InboundEnvelope` / `InboundKind`
//! - [`channel`] — `ItgChannel` model + cached store (config heart)
//! - [`verify`] — L0 trust checks (hmac / token / challenge)
//! - [`framing`] — L2 bytes → structured value (raw+json in this phase)
//! - [`mapping`] — declarative field mapping (zero-code normalizer)
//! - [`receipt`] — idempotency + trace model (`itg_receipts`)
//! - [`pipeline`] — Verify→Normalize→Dedup→Route→Ack, one transaction
//! - [`routes`] — `/api/v1/ingress/{channel_key}` endpoints
//! - [`vault`] — credential sealing (AES-256-GCM)
//! - [`trace`] — `TRACE_CTX` task-local + step-timeline recorder
//! - [`api_client`] — declarative outbound API clients (`itg_api_clients`)
//! - [`egress`] — L5 outbound executor + `itg_egress_log`
//! - [`token`] — OAuth client-credentials token provider (cached)

pub mod admin;
pub mod api_client;
pub mod batch;
pub mod channel;
pub mod connector;
pub mod cursor;
pub mod dto;
pub mod egress;
pub mod envelope;
pub mod framing;
pub mod mapping;
pub mod pipeline;
pub mod receipt;
pub mod routes;
pub mod supervisor;
pub mod token;
pub mod trace;
pub mod vault;
pub mod verify;

pub use channel::{ItgChannel, ItgChannelStore};
pub use egress::{EgressReceipt, EgressRequest};
pub use envelope::{InboundEnvelope, InboundKind};
pub use pipeline::{Pipeline, PipelineOutcome, RetryResult};
pub use trace::TraceCtx;

/// Process-wide shared integration handle — set once at startup, read by
/// worker handlers (`ingress.retry`/`ingress.pull`) that are constructed
/// before the AppState exists. `None` when the integration plane is disabled.
static SHARED: std::sync::OnceLock<std::sync::Arc<IntegrationPlane>> = std::sync::OnceLock::new();

/// Install the shared integration handle (called once from `build_app_state`).
pub fn set_shared(integration: std::sync::Arc<IntegrationPlane>) {
    let _ = SHARED.set(integration);
}

/// Access the shared integration handle, if initialized and enabled.
#[must_use]
pub fn shared() -> Option<std::sync::Arc<IntegrationPlane>> {
    SHARED.get().cloned()
}

/// Convenience: the shared inbound pipeline handle.
#[must_use]
pub fn shared_pipeline() -> Option<std::sync::Arc<Pipeline>> {
    shared().map(|p| p.pipeline_arc())
}

use crate::config::app::IntegrationConfig;
use crate::db::Pool;

/// Assembled handle to the Integration Plane.
///
/// Built once at startup ([`IntegrationPlane::init`]) and shared across handlers.
pub struct IntegrationPlane {
    supervisor: std::sync::OnceLock<std::sync::Arc<supervisor::IngressSupervisor>>,
    pool: crate::db::Pool,
    alert_emitter: crate::event::EventEmitter,
    config: IntegrationConfig,
    channels: ItgChannelStore,
    pipeline: std::sync::Arc<Pipeline>,
    limiter: routes::IngressRateLimiter,
    vault: Option<vault::Vault>,
    egress: egress::EgressExecutor,
}

impl IntegrationPlane {
    /// Assemble the plane and prime the channel cache.
    ///
    /// # Errors
    ///
    /// Returns `AppError` if the channel cache cannot be primed from the DB.
    pub async fn init(
        pool: Pool,
        config: IntegrationConfig,
        storage_root: String,
        registry: std::sync::Arc<crate::content_type::ContentTypeRegistry>,
        emitter: crate::event::EventEmitter,
        jwt_secret: String,
    ) -> crate::errors::app_error::AppResult<Self> {
        let vault = config
            .vault_key
            .as_ref()
            .map(|secret| vault::Vault::from_secret(secret))
            .transpose()?;
        let pool_handle = pool.clone();
        let alert_emitter = emitter.clone();
        let channels = ItgChannelStore::new(pool.clone());
        channels.refresh().await?;
        let pipeline = std::sync::Arc::new(Pipeline::new(
            pool,
            storage_root,
            registry,
            emitter.clone(),
            vault.clone(),
            Some(jwt_secret.clone()),
        ));
        pipeline.spawn_batch_flusher();
        Ok(Self {
            supervisor: std::sync::OnceLock::new(),
            egress: egress::EgressExecutor::new(pool_handle.clone(), vault.clone(), config.clone()),
            pool: pool_handle,
            alert_emitter,
            config,
            channels,
            pipeline,
            limiter: routes::IngressRateLimiter::new(),
            vault,
        })
    }

    /// Shared pool (pull connector and models).
    #[must_use]
    pub fn pool(&self) -> &crate::db::Pool {
        &self.pool
    }

    /// Plane configuration (vault key, limits, retention).
    #[must_use]
    pub fn config(&self) -> &IntegrationConfig {
        &self.config
    }

    /// Channel store (cached lookups by tenant + channel_key).
    #[must_use]
    pub fn channels(&self) -> &ItgChannelStore {
        &self.channels
    }

    /// The inbound pipeline.
    #[must_use]
    pub fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// Clonable pipeline handle (worker handlers).
    #[must_use]
    pub fn pipeline_arc(&self) -> std::sync::Arc<Pipeline> {
        self.pipeline.clone()
    }

    /// Per-channel ingress rate limiter.
    #[must_use]
    pub fn limiter(&self) -> &routes::IngressRateLimiter {
        &self.limiter
    }

    /// Install + start the supervisor (once, at server startup).
    #[must_use]
    pub fn ensure_supervisor(
        self: &std::sync::Arc<Self>,
    ) -> std::sync::Arc<supervisor::IngressSupervisor> {
        self.supervisor
            .get_or_init(|| supervisor::IngressSupervisor::start(std::sync::Arc::clone(self)))
            .clone()
    }

    /// Supervisor handle, if started.
    #[must_use]
    pub fn supervisor(&self) -> Option<std::sync::Arc<supervisor::IngressSupervisor>> {
        self.supervisor.get().cloned()
    }

    /// Telemetry batch stats (health API source).
    #[must_use]
    pub fn telemetry_batch_stats(
        &self,
    ) -> std::collections::HashMap<i64, crate::integration::batch::BatchStats> {
        self.pipeline.batcher.stats_snapshot()
    }

    /// Wake the supervisor after admin channel writes (hot enable/disable).
    pub fn wake_supervisor(&self) {
        if let Some(sup) = self.supervisor() {
            sup.wake();
        }
    }

    /// Emit an `integration.alert` event (observability alerts, §10.2).
    pub fn emit_alert(&self, alert_type: &str, data: serde_json::Value) {
        self.emit_event(alert_type, data);
    }

    /// Emit a custom `integration.*` event (SSE fan-out). The event type
    /// should carry the `integration.` prefix so `filter=integration.*`
    /// subscribers receive it.
    pub fn emit_event(&self, event_type: &str, data: serde_json::Value) {
        self.alert_emitter.emit(crate::event::Event::Custom {
            source: "integration".into(),
            event_type: event_type.to_string(),
            data,
        });
    }

    /// Credential vault handle (None when sealed — no key configured).
    #[must_use]
    pub fn vault(&self) -> Option<&vault::Vault> {
        self.vault.as_ref()
    }

    /// The outbound executor (L5).
    #[must_use]
    pub fn egress(&self) -> &egress::EgressExecutor {
        &self.egress
    }

    /// One outbound api-client call — the single egress entry point for
    /// services, worker handlers and plugins (§9.1). Resolves the op template,
    /// injects sealed auth, rate-limits, logs to `itg_egress_log`; ambient
    /// `TRACE_CTX` attaches automatically when present.
    ///
    /// # Errors
    ///
    /// See [`egress::EgressExecutor::call`].
    pub async fn call_api(
        &self,
        client_key: impl Into<String>,
        op: impl Into<String>,
        input: serde_json::Value,
    ) -> crate::errors::app_error::AppResult<EgressReceipt> {
        self.egress
            .call(EgressRequest::new(client_key, op, input))
            .await
    }

    /// [`Self::call_api`] with an explicit trace id — for callers outside any
    /// `TRACE_CTX` scope (e.g. a job handler recovering the id from its
    /// payload).
    ///
    /// # Errors
    ///
    /// See [`egress::EgressExecutor::call`].
    pub async fn call_api_traced(
        &self,
        trace_id: i64,
        client_key: impl Into<String>,
        op: impl Into<String>,
        input: serde_json::Value,
    ) -> crate::errors::app_error::AppResult<EgressReceipt> {
        self.egress
            .call(EgressRequest::new(client_key, op, input).with_trace(trace_id))
            .await
    }

    /// Inbound body size limit (bytes).
    #[must_use]
    pub fn body_limit(&self) -> usize {
        self.config.ingress_body_limit
    }
}
