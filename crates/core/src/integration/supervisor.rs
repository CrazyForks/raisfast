//! IngressSupervisor — supervised long-lived connection tasks for
//! stream/listen channels (integration.md §5.2). The third runtime form
//! besides request-driven (axum) and schedule-driven (worker jobs).
//!
//! Lifecycle: startup scan + admin-write notify + 5s fallback poll →
//! spawn/abort one ConnectionTask per channel. Each task runs the connector
//! loop with exponential backoff + jitter, persists state transitions to
//! `itg_channels.status`, and emits `integration.channel_state` events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use serde_json::json;
use tokio::sync::{Mutex, Notify};

use crate::db::driver::DbDriver;
use crate::integration::IntegrationPlane;
use crate::integration::channel::{self, ItgChannel};

/// In-memory health snapshot per channel (M5 health API source).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ChannelHealth {
    pub channel_id: i64,
    pub channel_key: String,
    /// connecting | connected | backoff | stopped | error
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    pub reconnects: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

struct TaskEntry {
    handle: tokio::task::JoinHandle<()>,
}

/// A stream/listen connector: `run` returns on disconnect/failure; the
/// supervisor owns reconnection. Implementations push frames through the
/// [`ConnectionSink`].
#[async_trait::async_trait]
pub trait StreamConnector: Send + Sync {
    async fn run(&self, ch: Arc<ItgChannel>, sink: ConnectionSink) -> anyhow::Result<()>;
}

/// Per-connection pipeline access handed to connectors.
#[derive(Clone)]
pub struct ConnectionSink {
    plane: Arc<IntegrationPlane>,
}

impl ConnectionSink {
    /// Push one frame (raw bytes) through the full pipeline.
    pub async fn submit(
        &self,
        ch: &ItgChannel,
        body: Vec<u8>,
    ) -> crate::integration::pipeline::PipelineOutcome {
        let owned = Arc::new(ch.clone());
        self.plane.pipeline().run_stream_frame(&owned, body).await
    }
}

type ConnectorFactory = Arc<dyn Fn() -> Box<dyn StreamConnector> + Send + Sync>;

/// Supervisor handle. `start()` spawns the loop; dropping the handle does
/// NOT stop it — call [`IngressSupervisor::shutdown`] (server owns it).
pub struct IngressSupervisor {
    pool: crate::db::Pool,
    plane: Arc<IntegrationPlane>,
    running: Mutex<HashMap<i64, TaskEntry>>,
    notify: Notify,
    health: DashMap<i64, ChannelHealth>,
    factories: Mutex<HashMap<String, ConnectorFactory>>,
    loop_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown: tokio::sync::watch::Sender<bool>,
}

impl IngressSupervisor {
    /// Start the supervisor for the plane. Idempotent per process
    /// (server calls once; tests may own the returned handle).
    #[must_use]
    pub fn start(plane: Arc<IntegrationPlane>) -> Arc<Self> {
        let sup = Arc::new(Self {
            pool: plane.pool().clone(),
            plane,
            running: Mutex::new(HashMap::new()),
            notify: Notify::new(),
            health: DashMap::new(),
            factories: Mutex::new(HashMap::new()),
            loop_handle: Mutex::new(None),
            shutdown: tokio::sync::watch::Sender::new(false),
        });
        let sup_loop = Arc::clone(&sup);
        let mut shutdown_rx = sup.shutdown.subscribe();
        // Built-in stream connectors (ws now; mqtt/tcp in M3) — seeded via a
        // short task because `start` is sync (OnceLock init at server boot).
        {
            let sup_seed = Arc::clone(&sup);
            tokio::spawn(async move {
                let mut factories = sup_seed.factories.lock().await;
                #[cfg(feature = "integration-stream")]
                {
                    use crate::integration::connector;
                    factories.entry("ws".into()).or_insert_with(|| {
                        Arc::new(|| Box::new(connector::ws_client::WsClientConnector))
                    });
                    factories.entry("mqtt".into()).or_insert_with(|| {
                        Arc::new(|| Box::new(connector::mqtt_client::MqttClientConnector))
                    });
                    factories.entry("tcp".into()).or_insert_with(|| {
                        Arc::new(|| Box::new(connector::tcp_listen::TcpListenConnector))
                    });
                }
                drop(factories);
                sup_seed.wake();
            });
        }
        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    _ = sup_loop.notify.notified() => {},
                    _ = ticker.tick() => {},
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            sup_loop.stop_all().await;
                            break;
                        }
                    }
                }
                if let Err(err) = sup_loop.rescan().await {
                    tracing::error!(error = %err, "supervisor rescan failed");
                }
            }
        });
        if let Ok(mut slot) = sup.loop_handle.try_lock()
            && slot.is_none()
        {
            *slot = Some(handle);
        }
        sup
    }

    /// Wake the rescan loop immediately (called after admin channel writes).
    pub fn wake(&self) {
        self.notify.notify_waiters();
    }

    /// Stop everything (server shutdown / tests).
    pub async fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        if let Ok(mut slot) = self.loop_handle.try_lock()
            && let Some(handle) = slot.take()
        {
            let _ = handle.await;
        }
        self.stop_all().await;
    }

    async fn stop_all(&self) {
        let mut running = self.running.lock().await;
        for (_, entry) in running.drain() {
            entry.handle.abort();
        }
        for mut health in self.health.iter_mut() {
            health.state = "stopped".into();
        }
    }

    /// Register a connector factory by transport name. Built-in transports
    /// (ws/mqtt/tcp) are wired by their milestone PRs; tests register `mock`.
    pub async fn register_connector(&self, transport: &str, factory: ConnectorFactory) {
        self.factories
            .lock()
            .await
            .insert(transport.to_string(), factory);
        self.wake();
    }

    /// Current health snapshots (M5 health API source).
    #[must_use]
    pub fn health_snapshot(&self) -> Vec<ChannelHealth> {
        self.health.iter().map(|h| h.clone()).collect()
    }

    /// Diff running tasks against enabled stream/listen channels.
    ///
    /// # Errors
    ///
    /// Returns `AppError` on the channel query failure.
    pub async fn rescan(self: &Arc<Self>) -> crate::errors::app_error::AppResult<()> {
        const CHANNEL_SCAN_COLS: &str = "id, tenant_id, channel_key, provider, display_name, mode, transport, framing, \
             codec, endpoint, verify_kind, verify_config, credentials, mapping, \
             normalizer_plugin, pull_semantics, pull_config, stream_config, ack_kind, \
             redelivery_max, backpressure, target_type, route_extra, status, last_error, \
             lease_owner, enabled, version, shadow, created_at, updated_at";
        let sql = format!(
            "SELECT {CHANNEL_SCAN_COLS} FROM itg_channels WHERE mode IN ('stream', 'listen')"
        );
        let rows: Vec<ItgChannel> = sqlx::query_as(crate::db::safe_sql(&sql))
            .fetch_all(&self.pool)
            .await?;
        let desired: HashMap<i64, ItgChannel> = rows
            .into_iter()
            .filter(|c| c.enabled && !c.shadow)
            .map(|c| (c.id.0, c))
            .collect();

        let sup = Arc::clone(self);
        let mut running = self.running.lock().await;
        // Abort tasks whose channels vanished / were disabled.
        let stale: Vec<i64> = running
            .keys()
            .copied()
            .filter(|id| !desired.contains_key(id))
            .collect();
        for id in stale {
            if let Some(entry) = running.remove(&id) {
                entry.handle.abort();
            }
            if let Some(mut health) = self.health.get_mut(&id) {
                health.state = "stopped".into();
            }
            tracing::info!(
                channel_id = id,
                "supervisor: task stopped (disabled/removed)"
            );
        }
        // Spawn tasks for new channels.
        let spawned: Vec<i64> = desired
            .keys()
            .copied()
            .filter(|id| !running.contains_key(id))
            .collect();
        for id in &spawned {
            let Some(ch) = desired.get(id) else { continue };
            let entry = Self::spawn_task(&sup, ch.clone());
            running.insert(*id, entry);
        }
        drop(running);
        if !spawned.is_empty() {
            tracing::info!(count = spawned.len(), "supervisor: tasks spawned");
        }
        Ok(())
    }

    fn spawn_task(sup: &Arc<Self>, ch: ItgChannel) -> TaskEntry {
        let sup = Arc::clone(sup);
        let ch = Arc::new(ch);
        let handle = tokio::spawn(async move {
            sup.connection_loop(ch).await;
        });
        TaskEntry { handle }
    }

    /// Per-channel connection state machine with exponential backoff + jitter.
    async fn connection_loop(self: Arc<Self>, ch: Arc<ItgChannel>) {
        let id = ch.id.0;
        let lease = crate::utils::id::random_hex(8);
        let mut backoff_secs: u64 = 1;
        let mut reconnects: u64 = 0;

        let factory = {
            let factories = self.factories.lock().await;
            factories.get(&ch.transport).map(Arc::clone)
        };
        let Some(factory) = factory else {
            self.record_state(
                id,
                &ch.channel_key,
                "error",
                reconnects,
                Some(format!(
                    "no connector registered for transport '{}'",
                    ch.transport
                )),
            )
            .await;
            let _ = channel::model::update_status(
                &self.pool,
                ch.id,
                "error",
                Some(&format!("no connector for transport '{}'", ch.transport)),
            )
            .await;
            return;
        };

        let _ = channel::model::update_status(&self.pool, ch.id, "connecting", None).await;
        let shutdown_rx = self.shutdown.subscribe();
        loop {
            if *shutdown_rx.borrow() {
                break;
            }
            self.record_state(id, &ch.channel_key, "connecting", reconnects, None)
                .await;
            // Claim the lease (single-instance placeholder, §5.2).
            let _ = set_lease(&self.pool, ch.id, &lease).await;

            let sink = ConnectionSink {
                plane: Arc::clone(&self.plane),
            };
            let connector = factory();
            let result = connector.run(Arc::clone(&ch), sink).await;
            let err_msg = match result {
                Ok(()) => {
                    tracing::info!(channel = %ch.channel_key, "connector closed gracefully");
                    None
                }
                Err(err) => {
                    tracing::warn!(channel = %ch.channel_key, error = %err, "connector failed");
                    Some(err.to_string())
                }
            };
            reconnects += 1;
            let jitter = random_jitter_ms();
            let wait = Duration::from_millis(backoff_secs * 1000 + jitter);
            self.record_state(id, &ch.channel_key, "backoff", reconnects, err_msg)
                .await;
            let _ = channel::model::update_status(&self.pool, ch.id, "degraded", None).await;
            let mut shutdown_rx = self.shutdown.subscribe();
            tokio::select! {
                _ = tokio::time::sleep(wait) => {},
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        break;
                    }
                }
            }
            backoff_secs = (backoff_secs * 2).min(300); // cap 5min (§5.2)
        }
        self.record_state(id, &ch.channel_key, "stopped", reconnects, None)
            .await;
    }

    async fn record_state(
        &self,
        id: i64,
        channel_key: &str,
        state: &str,
        reconnects: u64,
        error: Option<String>,
    ) {
        let connected_at = (state == "connected")
            .then(crate::utils::tz::now_utc)
            .map(|t| t.to_rfc3339());
        // Sticky last_error: cleared only by a successful connection.
        let last_error = error.clone().or_else(|| {
            self.health
                .get(&id)
                .filter(|_| state != "connected")
                .and_then(|h| h.last_error.clone())
        });
        self.health.insert(
            id,
            ChannelHealth {
                channel_id: id,
                channel_key: channel_key.to_string(),
                state: state.to_string(),
                connected_at,
                reconnects,
                last_error,
            },
        );
        self.plane.emit_alert(
            "integration.channel_state",
            json!({
                "channel_id": id,
                "channel": channel_key,
                "state": state,
                "reconnects": reconnects,
                "error": error,
            }),
        );
    }
}

async fn set_lease(
    pool: &crate::db::Pool,
    id: crate::types::snowflake_id::SnowflakeId,
    lease: &str,
) -> crate::errors::app_error::AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "UPDATE itg_channels SET lease_owner = {}, updated_at = {} WHERE id = {}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(lease)
        .bind(now)
        .bind(*id)
        .execute(pool)
        .await?;
    Ok(())
}

fn random_jitter_ms() -> u64 {
    let mut buf = [0u8; 4];
    if getrandom::fill(&mut buf).is_ok() {
        u64::from(u32::from_le_bytes(buf) % 500)
    } else {
        250
    }
}
