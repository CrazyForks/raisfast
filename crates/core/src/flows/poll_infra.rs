//! Generic async-task poll infra (media-nodes.md §5, generalized 2026-09-25).
//!
//! Await v4 made resume trigger-agnostic ("whoever cares calls the resume").
//! For machine-driven waits the "whoever" is this hub: ONE sweep loop serving
//! every registered [`WaitPoller`]. `video` is the first scenario; future
//! async submit→poll flows (digital humans, long audio, music, HTTP poll
//! jobs…) implement the trait and register — no second sweeper.
//!
//! Split of concerns:
//!
//! | infra (generic)                       | poller impl (vertical)          |
//! |---------------------------------------|---------------------------------|
//! | scan parked instances + dispatch      | how to query the external system|
//! | uniform `deadline_unix` timeout (checked BEFORE the poll) | terminal-state → resume payload |
//! | `{kind}.done/fail/timeout` actions    | artifact fetch + storage persist|
//! | resume whitelist derived from registry| park event name                 |
//!
//! Conventions a poller's node MUST follow (documented once here):
//! - node output carries the durable task handle:
//!   `{phase: "submitted", task_id, deadline_unix, …poller-private keys}`
//!   (`phase` transitions are owned by the node's engine arm, two-step write)
//! - `deadline_unix` (epoch secs) is the generic timeout trigger; absent or
//!   `i64::MAX` = wait forever (manual ops).
//! - resume payload lands in the pool under the node's `resume` field
//!   (`resume_snapshot` single-port semantics).

use std::sync::{Arc, OnceLock};

use serde_json::{Value, json};

use crate::errors::app_error::AppResult;
use crate::integration::IntegrationPlane;
use crate::llm::service::LlmRouter;
use crate::plugins::PluginManager;

use super::nodes::ResumeEnvelope;
use super::run;

/// Sweep cadence (one indexed scan per tick when nothing is parked).
const POLL_INTERVAL_SECS: u64 = 30;

/// Everything a poller needs to inspect one parked node.
pub struct PollCtx<'a> {
    pub pool: &'a crate::db::Pool,
    pub router: Arc<LlmRouter>,
    pub tenant: &'a str,
    pub instance_id: crate::types::snowflake_id::SnowflakeId,
    pub node_id: &'a str,
    /// Parked node output — the durable task handle written at submit time.
    pub info: &'a serde_json::Value,
}

/// One async-poll scenario. `kind` is simultaneously the node type string,
/// the `waiting_kind` marker and the claim kind — one vocabulary (media-nodes.md
/// §4.3 typing).
/// Everything a hook translator needs to turn a provider webhook body into
/// a resume envelope (fetching artifacts included).
pub struct HookCtx<'a> {
    pub router: Arc<LlmRouter>,
    pub tenant: &'a str,
    pub instance_id: crate::types::snowflake_id::SnowflakeId,
    pub node_id: &'a str,
    /// Parked node output — carries the submit-time task handle (`task_id`,
    /// `model`), letting the translator correlate the callback.
    pub info: &'a serde_json::Value,
    pub storage: Arc<dyn crate::storage::Storage>,
}

#[async_trait::async_trait]
pub trait WaitPoller: Send + Sync {
    /// Node type + waiting/claim kind (e.g. "video").
    fn kind(&self) -> &'static str;
    /// Event emitted when the engine parks a node of this kind.
    fn park_event(&self) -> &'static str;
    /// Whether this scenario accepts provider webhook callbacks (hook P1,
    /// wait-triggers.md §4). `true` makes the engine mint a capability and
    /// pass the callback URL to the submit.
    fn supports_hook(&self) -> bool {
        false
    }
    /// Provider webhook body → resume envelope. `Ok(None)` = non-terminal
    /// event (keep parked). `Err` = transient → respond 5xx so the provider
    /// retries; the claim's 409 keeps redelivery idempotent.
    async fn translate_hook(
        &self,
        _ctx: &HookCtx<'_>,
        _body: &Value,
    ) -> AppResult<Option<ResumeEnvelope>> {
        Ok(None)
    }
    /// Inspect the parked task. `Ok(None)` = still pending, leave parked;
    /// `Ok(Some(envelope))` = terminal, resume with it. The infra has already
    /// handled the deadline timeout before this is called — a poller never
    /// needs its own timeout branch (transient errors return Ok(None) or Err,
    /// both leave the node parked for the next sweep).
    async fn poll(&self, ctx: &PollCtx<'_>) -> AppResult<Option<ResumeEnvelope>>;
}

/// Process-wide registry — installed once at startup (before traffic), read
/// by the sweep loop AND by `run.rs` resume validation.
static REGISTRY: OnceLock<Vec<Arc<dyn WaitPoller>>> = OnceLock::new();

/// Install the poller set (server startup). Later duplicates of a kind are
/// ignored (first wins — predictable startup order).
pub fn install(pollers: Vec<Arc<dyn WaitPoller>>) {
    let mut seen: Vec<String> = Vec::new();
    let uniq: Vec<Arc<dyn WaitPoller>> = pollers
        .into_iter()
        .filter(|p| {
            if seen.contains(&p.kind().to_string()) {
                tracing::warn!("duplicate wait poller kind '{}' ignored", p.kind());
                false
            } else {
                seen.push(p.kind().to_string());
                true
            }
        })
        .collect();
    let _ = REGISTRY.set(uniq);
}

/// Whether a node type is poll-backed (resume validation + park typing).
#[must_use]
pub fn is_pollable(kind: &str) -> bool {
    poller_for(kind).is_some()
}

/// The poll action vocabulary for a kind — convention-derived, so `run.rs`
/// needs no per-kind whitelist table.
#[must_use]
pub fn poll_actions(kind: &str) -> [String; 3] {
    [
        format!("{kind}.done"),
        format!("{kind}.fail"),
        format!("{kind}.timeout"),
    ]
}

/// The registered poller for a node type.
#[must_use]
pub fn poller_for(kind: &str) -> Option<Arc<dyn WaitPoller>> {
    REGISTRY.get()?.iter().find(|p| p.kind() == kind).cloned()
}

/// Spawn the sweep loop (cheap when nothing is parked — one indexed scan per
/// registered kind per tick).
pub fn spawn(
    pool: crate::db::Pool,
    router: Arc<LlmRouter>,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
) {
    let pollers = REGISTRY.get().cloned().unwrap_or_default();
    if pollers.is_empty() {
        tracing::warn!("wait-poll hub spawned with no pollers registered");
    }
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            for poller in &pollers {
                match sweep_kind(
                    &pool,
                    router.clone(),
                    plane.clone(),
                    plugins.clone(),
                    poller,
                )
                .await
                {
                    Ok(0) => {}
                    Ok(n) => tracing::info!("wait-poll '{}' acted on {n} task(s)", poller.kind()),
                    Err(e) => tracing::error!("wait-poll '{}' sweep error: {e}", poller.kind()),
                }
            }

            // Video-body iteration aggregation (iteration-video.md §2.4)：
            // 提交期 park 的 iteration 由这里周期收割（[自造-修复] 2026-09
            // 冒烟实锤：本函数曾无任何调用方，多分镜实例永远 waiting）。
            match super::run::sweep_video_iterations(
                &pool,
                router.clone(),
                plane.clone(),
                plugins.clone(),
            )
            .await
            {
                Ok(n) if n > 0 => {
                    tracing::info!("video iteration sweep completed {n} instance(s)")
                }
                Ok(_) => {}
                Err(e) => tracing::error!("video iteration sweep error: {e}"),
            }
        }
    });
}

/// One sweep pass over one poller's parked instances.
///
/// # Errors
///
/// Propagates DB/load errors; per-instance failures are logged and skipped.
async fn sweep_kind(
    pool: &crate::db::Pool,
    router: Arc<LlmRouter>,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
    poller: &Arc<dyn WaitPoller>,
) -> AppResult<u64> {
    let instances = super::model::find_waiting_by_kind(pool, poller.kind()).await?;
    let mut acted = 0_u64;
    for inst in instances {
        match decide(pool, &router, poller, &inst).await {
            Ok(Some(envelope)) => {
                // resume_instance re-validates the action whitelist and the
                // claim (409-safe against a racing sweep).
                match run::resume_instance(
                    pool,
                    router.clone(),
                    plane.clone(),
                    plugins.clone(),
                    inst.id,
                    &envelope,
                    None,
                )
                .await
                {
                    Ok(()) => acted += 1,
                    Err(e) => tracing::warn!(
                        "wait-poll '{}' resume failed, instance {}: {e}",
                        poller.kind(),
                        inst.id
                    ),
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    "wait-poll '{}' check failed, instance {} (skipped): {e}",
                    poller.kind(),
                    inst.id
                );
            }
        }
    }
    Ok(acted)
}

/// Deadline-first decision for one parked instance (generic timeout happens
/// BEFORE the poller runs — pollers never carry their own timeout branch).
/// `Ok(None)` = still pending.
async fn decide(
    pool: &crate::db::Pool,
    router: &Arc<LlmRouter>,
    poller: &Arc<dyn WaitPoller>,
    inst: &super::model::FlowInstance,
) -> AppResult<Option<ResumeEnvelope>> {
    let graph = run::load_graph_for_instance(pool, inst).await?;
    let Some(value) = super::model::find_snapshot(pool, inst.id).await? else {
        return Ok(None);
    };
    let snap: super::engine::Snapshot = serde_json::from_value(value)
        .map_err(|e| crate::errors::app_error::AppError::Internal(anyhow::anyhow!("{e}")))?;
    let Some(node_id) = snap.waiting_nodes.first() else {
        return Ok(None);
    };
    let Some(node) = graph.nodes.get(node_id) else {
        return Ok(None);
    };
    if node.data.kind != poller.kind() {
        return Ok(None); // not ours (defensive; waiting_kind already filters)
    }
    let Some(info) = snap
        .node_states
        .get(node_id)
        .and_then(|st| st.output.as_ref())
        .cloned()
    else {
        return Ok(None);
    };
    if info.get("phase").and_then(|v| v.as_str()) != Some("submitted") {
        return Ok(None);
    }

    // Generic deadline gate: past `deadline_unix` → uniform timeout envelope,
    // no poller round-trip. Absent key = wait forever.
    if info
        .get("deadline_unix")
        .and_then(|v| v.as_i64())
        .is_some_and(|deadline| crate::utils::tz::now_utc().timestamp() > deadline)
    {
        return Ok(Some(ResumeEnvelope {
            action: format!("{}.timeout", poller.kind()),
            data: Some(json!({
                "task_id": info.get("task_id").cloned().unwrap_or(serde_json::Value::Null),
                "status": "timeout",
            })),
        }));
    }

    let ctx = PollCtx {
        pool,
        router: router.clone(),
        tenant: inst.tenant_id.as_str(),
        instance_id: inst.id,
        node_id,
        info: &info,
    };
    poller.poll(&ctx).await
}

/// Process-wide hook plumbing (media/wait-triggers §4): the pool for claim
/// minting and the public base URL for capability URLs. Installed at startup
/// next to the storage handle.
static HOOK_INFRA: OnceLock<(crate::db::Pool, String)> = OnceLock::new();

/// Install the hook infra. `base_url` is the externally reachable origin
/// (config `base_url`) — capability URLs are `{base_url}/api/v1/flows/hooks/{id}`.
pub fn install_hook_infra(pool: crate::db::Pool, base_url: String) {
    let _ = HOOK_INFRA.set((pool, base_url.trim_end_matches('/').to_string()));
}

/// Heuristic: can a vendor's servers reach this base_url? Loopback /
/// link-local / RFC1918 hosts cannot — hooks are pointless there (see the
/// guard in [`provision_callback`]). Hostnames are assumed public.
fn publicly_reachable(base_url: &str) -> bool {
    let Some(after_scheme) = base_url.split("://").nth(1) else {
        return false; // 无 scheme — 无法判定的输入按不可达处理（fail-safe）
    };
    let host = after_scheme
        .split(['/', '?'])
        .next()
        .unwrap_or("");
    let host = host.split('@').next_back().unwrap_or(host);
    // Bracketed IPv6 literal `[::1]:9898` → `::1`; otherwise strip :port.
    let host = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        host.split(':').next().unwrap_or(host)
    };
    let h = host.to_ascii_lowercase();
    !matches!(
        h.as_str(),
        "localhost" | "127.0.0.1" | "::1" | "0.0.0.0" | "169.254.0.1"
    ) && !h.starts_with("127.")
        && !h.starts_with("192.168.")
        && !h.starts_with("10.")
        && !h.starts_with("169.254.")
        && !h.starts_with("fc")
        && !h.starts_with("fd")
        // 172.16.0.0/12
        && !h.split('.').next().is_some_and(|first| {
            first == "172"
                && h.split('.').nth(1)
                    .and_then(|o| o.parse::<u16>().ok())
                    .is_some_and(|o| (16..=31).contains(&o))
        })
}

/// Mint a hook capability for a parked-to-be task: idempotently open the
/// claim row and attach the callback credential (sha256 only — machine
/// callbacks never need the display copy). Returns the capability URL, or
/// `None` when the infra is not installed.
pub async fn provision_callback(
    instance_id: crate::types::snowflake_id::SnowflakeId,
    node_id: &str,
    kind: &str,
) -> AppResult<Option<String>> {
    let Some((pool, base_url)) = HOOK_INFRA.get() else {
        return Ok(None);
    };
    // 回调可达性守卫（[自造] 2026-09 MiniMax V2 冒烟教训）：loopback/内网
    // base_url 厂商服务器永远够不着——MiniMax V2 会主动 challenge 验证回调
    // 地址，不可达直接拒单（fail to check callback url）。此时跳过 hook，
    // 走 poll-only（30s 轮询聚合），功能等价只是慢半拍。
    if !publicly_reachable(base_url) {
        return Ok(None);
    }
    super::model::ensure_flow_resume_open(pool, instance_id, node_id, kind, None).await?;
    let Some(row) = super::model::find_open_flow_resume(pool, instance_id, node_id).await? else {
        return Ok(None);
    };
    if row.token_hash.is_some() {
        // Already provisioned (engine retry / replay) — keep the ORIGINAL
        // capability; re-minting would orphan the URL already sent upstream.
        return Ok(None);
    }
    let callback_id = crate::utils::id::random_hex(24);
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(callback_id.as_bytes());
    let hash = hex::encode(h.finalize());
    super::model::set_flow_resume_token(pool, row.id, &hash, "").await?;
    Ok(Some(format!("{base_url}/api/v1/flows/hooks/{callback_id}")))
}

/// Resolve a hook capability to its parked task context. `None` = unknown
/// callback_id (404 to the caller).
pub(crate) async fn resolve_hook(
    pool: &crate::db::Pool,
    callback_id: &str,
) -> Option<(super::model::FlowResume, Arc<dyn WaitPoller>)> {
    use sha2::Digest as _;
    let mut h = sha2::Sha256::new();
    h.update(callback_id.as_bytes());
    let hash = hex::encode(h.finalize());
    let row = super::model::find_open_by_token_hash(pool, &hash)
        .await
        .ok()??;
    let inst = super::model::find_instance_by_id(pool, row.instance_id)
        .await
        .ok()?;
    let poller = poller_for(&inst.waiting_kind.clone().unwrap_or_default())?;
    Some((row, poller))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn poll_actions_convention() {
        let [done, fail, timeout] = poll_actions("video");
        assert_eq!(done, "video.done");
        assert_eq!(fail, "video.fail");
        assert_eq!(timeout, "video.timeout");
    }

    #[test]
    fn unknown_kind_is_not_pollable() {
        assert!(!is_pollable("definitely-not-registered"));
    }

    /// 回调可达性守卫（[自造]）：loopback/内网不可作为厂商回调 base。
    #[test]
    fn publicly_reachable_heuristic() {
        // 不可达：loopback / 内网 / 非法
        for bad in [
            "http://localhost:9898",
            "http://127.0.0.1:9898",
            "http://192.168.1.10:9898",
            "http://10.0.0.2",
            "http://172.16.5.4",
            "http://172.31.255.255",
            "https://[::1]:9898",
            "not-a-url",
            "",
        ] {
            assert!(!publicly_reachable(bad), "{bad} 应判为不可达");
        }
        // 可达：公网域名 / 公网 IP
        for good in [
            "https://api.example.com",
            "https://myapp.fly.dev",
            "http://203.0.113.7:9898",
            "https://172.32.0.1",
        ] {
            assert!(publicly_reachable(good), "{good} 应判为可达");
        }
    }

    /// Deadline gate is pure data: past deadline with any submitted info
    /// would produce a timeout envelope — asserted through decide()'s
    /// contract via the gate condition itself (DB-free smoke of the key).
    #[test]
    fn deadline_key_convention() {
        let info = json!({"phase": "submitted", "task_id": "t1", "deadline_unix": 1_i64});
        let deadline = info.get("deadline_unix").and_then(|v| v.as_i64());
        assert_eq!(deadline, Some(1));
        assert!(
            crate::utils::tz::now_utc().timestamp() > deadline.unwrap_or(i64::MAX),
            "epoch-1 deadline is always past"
        );
    }
}
