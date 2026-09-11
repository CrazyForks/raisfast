//! LlmRouter — the routing core: channel/key selection, cooldowns, failure
//! classification and last-resort fallback (design §6, §7, §10.1).
//!
//! State layout (all router-level, never inside rebuilt cache structures):
//! - `cursors`: per-channel polling cursors (`AtomicU64`, §6.1)
//! - `cooldowns`: per-key cooldowns with kind + reason (§6.2 three-way)
//! - `transient_fails`: consecutive-failure counters for exponential backoff
//! - `persist_locks`: per-channel serialization for async DB persistence
//!   (§6.2: stale-snapshot overwrite prevention)
//!
//! The cache snapshot sits behind a `std::sync::RwLock<Arc<…>>`: guards are
//! short in-memory sections with no await, so sync selection/report paths
//! (memory is routing-authoritative, §6.2) stay non-async.
//!
//! `execute()` / `SideEffectGuard` (P2) and the wait queue (§7.6) build on
//! this core; P1 ships selection + failure handling only.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Semaphore;

use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::cache::{CachedChannel, ChannelCache, ModelInfo, RouteKey};
use crate::llm::models::channel::{LlmChannelStatus, LlmKeyMode, LlmKeyStatus};
use crate::llm::queue::QueueRegistry;
use crate::types::snowflake_id::SnowflakeId;

/// Global default retry budget (attempts = retry_times + 1, design §7.3).
pub const DEFAULT_RETRY_TIMES: usize = 2;

/// Per-token in-flight cap (design §7.6; options-wired later).
pub const TOKEN_MAX_CONCURRENT: i64 = 10;
/// Per-user in-flight cap (design §7.6; multi-token amplification guard).
pub const USER_MAX_CONCURRENT: i64 = 20;

/// Queue bounds per traffic class (design §7.6 分档).
#[derive(Debug, Clone, Copy)]
pub struct QueueTier {
    pub max_waiting: usize,
    pub max_waiting_bytes: usize,
    pub max_wait_secs: u64,
}

/// External interactive traffic: shallow queue, fast reject.
pub const RELAY_TIER: QueueTier = QueueTier {
    max_waiting: 50,
    max_waiting_bytes: 32 * 1024 * 1024,
    max_wait_secs: 15,
};

/// Internal jobs (agent runs / kb batches): deep queue, patient wait.
pub const INTERNAL_TIER: QueueTier = QueueTier {
    max_waiting: 1000,
    max_waiting_bytes: 512 * 1024 * 1024,
    max_wait_secs: 300,
};

const TRANSIENT_BASE_SECS: u64 = 5;
const TRANSIENT_MAX_SECS: u64 = 60;
const WINDOW_DEFAULT_SECS: u64 = 30 * 60;
const WINDOW_MAX_SECS: u64 = 6 * 60 * 60;
/// A `Retry-After` above this reads as a window limit, not a transient blip.
const WINDOW_RETRY_AFTER_SECS: u64 = 60;

/// Arrears / banned-account keyword defaults; operators can extend at
/// runtime via options (design §7.4).
pub const ARREARS_KEYWORDS: &[&str] = &[
    "your credit balance is too low",
    "you exceeded your current quota",
    "insufficient balance",
    "insufficient_quota",
    "this organization has been disabled",
    "permission denied",
    "the security token included in the request is invalid",
    "operation not allowed",
    "your account is not authorized",
];

/// Window-limit keyword defaults (coding-plan 5h/daily caps, design §6.2).
pub const WINDOW_KEYWORDS: &[&str] = &[
    "usage limit reached",
    "will reset at",
    "limit will reset",
    "rate limit will reset",
    "exceeded your usage limit",
];

/// Cooldown class (design §6.2): transient keys may be last-resorted, window
/// keys carry an explicit recovery time and must not be hard-tried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CooldownKind {
    Transient,
    Window,
}

#[derive(Debug, Clone)]
struct CooldownUntil {
    until: Instant,
    kind: CooldownKind,
    reason: String,
}

/// Per-request retry state (design §7.2): the tried set only guards
/// last-resort — normal selection is deduped implicitly by cooldowns.
#[derive(Debug, Default)]
pub struct RetryState {
    pub tried: HashSet<(SnowflakeId, usize)>,
}

/// Resolution context (design §10.1); `group` stays `None` → `"default"`
/// until group routing ships (§16).
pub struct ResolveCtx<'a> {
    pub tenant: &'a str,
    pub group: Option<&'a str>,
    pub pin_channel: Option<SnowflakeId>,
}

/// One selected upstream endpoint (design §10.1 `ResolvedEndpoint`).
#[derive(Debug, Clone)]
pub struct ResolvedEndpoint {
    pub channel_id: SnowflakeId,
    pub key_index: usize,
    pub base_url: String,
    pub api_key: String,
    pub provider: String,
    pub upstream_model: String,
    pub model: Arc<ModelInfo>,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
}

/// Failure report payload for `report_failure` (classification input).
#[derive(Debug, Clone, Default)]
pub struct UpstreamFailure {
    pub status: Option<u16>,
    pub message: String,
    pub retry_after: Option<Duration>,
}

/// Classified failure (design §6.2, judged in order: window → arrears →
/// status code → transient).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Window,
    Arrears,
    Transient,
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    let h = haystack.to_ascii_lowercase();
    needles.iter().any(|n| h.contains(n))
}

/// Classify an upstream failure (design §6.2 three-way, window semantics take
/// priority over status codes so `403 + reset` never disables a key).
pub fn classify_failure(f: &UpstreamFailure) -> FailureKind {
    if contains_any(&f.message, WINDOW_KEYWORDS)
        || f.retry_after
            .map(|d| d.as_secs() > WINDOW_RETRY_AFTER_SECS)
            .unwrap_or(false)
    {
        return FailureKind::Window;
    }
    let arrears_status = f.status.is_some_and(|s| matches!(s, 401..=403));
    if arrears_status || contains_any(&f.message, ARREARS_KEYWORDS) {
        return FailureKind::Arrears;
    }
    FailureKind::Transient
}

fn rand_u64() -> u64 {
    let mut buf = [0u8; 8];
    if getrandom::fill(&mut buf).is_err() {
        return 0;
    }
    u64::from_le_bytes(buf)
}

/// The routing core. Shared via `Arc<LlmRouter>` on `AppState`.
pub struct LlmRouter {
    pub(crate) pool: Option<Pool>,
    pub(crate) cache: RwLock<Arc<ChannelCache>>,
    cursors: DashMap<SnowflakeId, Arc<AtomicU64>>,
    cooldowns: DashMap<(SnowflakeId, usize), CooldownUntil>,
    transient_fails: DashMap<(SnowflakeId, usize), u32>,
    persist_locks: DashMap<SnowflakeId, Arc<tokio::sync::Mutex<()>>>,
    slots: DashMap<(SnowflakeId, usize), Arc<Semaphore>>,
    queues: QueueRegistry,
    token_inflight: DashMap<SnowflakeId, Arc<AtomicI64>>,
    user_inflight: DashMap<SnowflakeId, Arc<AtomicI64>>,
    latencies: DashMap<(String, String), Arc<std::sync::Mutex<f64>>>,
    pub(crate) providers:
        DashMap<(SnowflakeId, usize), Arc<dyn raisfast_agent::provider::ModelProvider>>,
}

impl LlmRouter {
    /// Build the router and warm the cache from the database (best-effort:
    /// an empty/unreachable DB logs a warning and starts with an empty table).
    pub async fn new(pool: Pool) -> Arc<Self> {
        let router = Arc::new(Self::empty(Some(pool)));
        if let Err(err) = router.reload().await {
            tracing::warn!(%err, "llm router initial cache load failed, starting empty");
        }
        router
    }

    fn empty(pool: Option<Pool>) -> Self {
        Self {
            pool,
            cache: RwLock::new(Arc::new(ChannelCache::default())),
            cursors: DashMap::new(),
            cooldowns: DashMap::new(),
            transient_fails: DashMap::new(),
            persist_locks: DashMap::new(),
            slots: DashMap::new(),
            queues: QueueRegistry::new(),
            token_inflight: DashMap::new(),
            user_inflight: DashMap::new(),
            latencies: DashMap::new(),
            providers: DashMap::new(),
        }
    }

    /// Test constructor: preloaded cache, no DB (persistence disabled).
    pub fn from_cache_for_test(cache: ChannelCache) -> Arc<Self> {
        Arc::new(Self::empty(None).with_cache(cache))
    }

    fn with_cache(mut self, cache: ChannelCache) -> Self {
        self.cache = RwLock::new(Arc::new(cache));
        self
    }

    fn pool(&self) -> AppResult<&Pool> {
        self.pool
            .as_ref()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("llm router has no database pool")))
    }

    /// Reload the whole cache from the database (§7.1 write-path / 5min
    /// fallback rebuild).
    pub async fn reload(&self) -> AppResult<()> {
        let pool = self.pool()?.clone();
        let channels = crate::llm::models::channel::list_channels(&pool, None).await?;
        let models = crate::llm::models::model::list_models(&pool, None, None).await?;
        *self.cache.write().expect("llm cache lock") =
            Arc::new(ChannelCache::build(channels, models));
        Ok(())
    }

    /// Reload one channel entry (incremental write-path invalidation, §7.1);
    /// a vanished row removes the channel and its router-level state (§7.1
    /// cleanup of cursors/cooldowns/persist locks).
    pub async fn reload_channel(&self, id: SnowflakeId) -> AppResult<()> {
        let pool = self.pool()?.clone();
        let row = crate::llm::models::channel::find_by_id(&pool, id, None).await?;
        {
            let mut guard = self.cache.write().expect("llm cache lock");
            let cache = Arc::make_mut(&mut *guard);
            match row {
                Some(row) => {
                    // Capacity edits invalidate cached semaphores/providers;
                    // in-flight permits keep their old Arcs (safe).
                    self.slots.retain(|(ch, _), _| *ch != id);
                    self.providers.retain(|(ch, _), _| *ch != id);
                    let cached = ChannelCache::from_row(&row);
                    cache.channels.insert(id, Arc::new(cached));
                }
                None => {
                    cache.channels.remove(&id);
                    self.cursors.remove(&id);
                    self.persist_locks.remove(&id);
                    self.cooldowns.retain(|(ch, _), _| *ch != id);
                    self.transient_fails.retain(|(ch, _), _| *ch != id);
                    self.slots.retain(|(ch, _), _| *ch != id);
                    self.providers.retain(|(ch, _), _| *ch != id);
                }
            }
            cache.rebuild_routes();
        }
        Ok(())
    }

    /// Directory lookup through the cache (DB row → built-in seed → None).
    pub fn model_info(&self, tenant: &str, name: &str) -> Option<Arc<ModelInfo>> {
        self.cache
            .read()
            .expect("llm cache lock")
            .model_info(tenant, name)
    }

    /// Clone the current cache snapshot (relay handlers read it per request).
    pub fn cache_snapshot(&self) -> Arc<ChannelCache> {
        self.cache.read().expect("llm cache lock").clone()
    }

    /// Resolve a model to one ready endpoint (§7.2 selection at attempt 0 +
    /// §6.1 key pick). Errors: unknown model / no available channel.
    pub fn resolve(&self, ctx: &ResolveCtx<'_>, model: &str) -> AppResult<ResolvedEndpoint> {
        let cache = self.cache.read().expect("llm cache lock").clone();
        let info = cache
            .model_info(ctx.tenant, model)
            .ok_or_else(|| AppError::BadRequest(format!("unknown model: {model}")))?;
        let mut retry = RetryState::default();
        let (channel, key_index) = self.select_channel(&cache, ctx, model, 0, &mut retry)?;
        Ok(ResolvedEndpoint {
            channel_id: channel.id,
            key_index,
            base_url: channel.base_url.clone(),
            api_key: channel.keys[key_index].plain.clone().unwrap_or_default(),
            provider: channel.provider.clone(),
            upstream_model: mapped_model(&channel, model),
            model: info,
            param_override: channel.param_override.clone(),
            header_override: channel.header_override.clone(),
        })
    }

    /// Channel + key selection (design §7.2). Steps: route lookup (public
    /// model names only), alive filtering, priority tier (attempt-indexed,
    /// empty tiers collapse), weighted random in tier, last-resort among
    /// un-tried transient-cooled keys.
    pub fn select_channel(
        &self,
        cache: &ChannelCache,
        ctx: &ResolveCtx<'_>,
        model: &str,
        attempt: usize,
        retry: &mut RetryState,
    ) -> AppResult<(Arc<CachedChannel>, usize)> {
        let route = RouteKey {
            tenant: ctx.tenant.to_owned(),
            group: ctx.group.unwrap_or("default").to_owned(),
            model: model.to_owned(),
        };
        let candidates: Vec<Arc<CachedChannel>> = match ctx.pin_channel {
            Some(pin) => {
                let ch = cache.channels.get(&pin).cloned().ok_or_else(|| {
                    AppError::BadRequest(format!("pinned channel {pin} not found"))
                })?;
                if ch.status != LlmChannelStatus::Enabled {
                    return Err(AppError::BadRequest(format!(
                        "pinned channel {pin} is not enabled"
                    )));
                }
                vec![ch]
            }
            None => cache.candidates(&route).cloned().unwrap_or_default(),
        };
        if candidates.is_empty() {
            return Err(AppError::BadRequest(format!(
                "no channel for model {model}"
            )));
        }

        let alive: Vec<Arc<CachedChannel>> = candidates
            .iter()
            .filter(|ch| ch.status == LlmChannelStatus::Enabled && self.any_ready_key(ch))
            .cloned()
            .collect();

        if alive.is_empty() {
            return self.last_resort(&candidates, retry, model);
        }

        let mut tiers: Vec<i64> = alive.iter().map(|ch| ch.priority).collect();
        tiers.sort_unstable_by(|a, b| b.cmp(a));
        tiers.dedup();
        let tier = tiers[attempt.min(tiers.len() - 1)];
        let tier_channels: Vec<Arc<CachedChannel>> = alive
            .iter()
            .filter(|ch| ch.priority == tier)
            .cloned()
            .collect();

        let channel = weighted_pick(&tier_channels);
        let idx = self.select_key(&channel).ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("selected channel has no ready key"))
        })?;
        Ok((channel, idx))
    }

    /// Pick one key inside a channel (design §6.1): active + not cooled;
    /// polling advances the router-level cursor, random draws uniformly.
    pub fn select_key(&self, channel: &CachedChannel) -> Option<usize> {
        let ready: Vec<usize> = channel
            .keys
            .iter()
            .enumerate()
            .filter(|(i, k)| {
                k.status == LlmKeyStatus::Active
                    && k.plain.is_some()
                    && !self.cooldown_active(channel.id, *i)
            })
            .map(|(i, _)| i)
            .collect();
        if ready.is_empty() {
            return None;
        }
        match channel.key_mode {
            LlmKeyMode::Random => Some(ready[(rand_u64() as usize) % ready.len()]),
            LlmKeyMode::Polling => {
                let cursor = self.cursors.entry(channel.id).or_default().clone();
                let len = channel.keys.len().max(1) as u64;
                let start = cursor.load(Ordering::Relaxed) % len;
                for step in 0..ready.len() as u64 {
                    let candidate = ((start + step) % len) as usize;
                    if ready.contains(&candidate) {
                        cursor.store((candidate as u64 + 1) % len, Ordering::Relaxed);
                        return Some(candidate);
                    }
                }
                Some(ready[0])
            }
        }
    }

    /// Last-resort fallback (design §7.2 step 5): only transient-cooled,
    /// un-tried keys of enabled channels; the earliest-expiring wins; each
    /// key is hard-tried at most once per request. Window-cooled and disabled
    /// keys never participate.
    fn last_resort(
        &self,
        candidates: &[Arc<CachedChannel>],
        retry: &mut RetryState,
        model: &str,
    ) -> AppResult<(Arc<CachedChannel>, usize)> {
        let mut best: Option<(Arc<CachedChannel>, usize, Instant)> = None;
        let now = Instant::now();
        for ch in candidates {
            if ch.status != LlmChannelStatus::Enabled {
                continue;
            }
            for (idx, _) in ch.keys.iter().enumerate() {
                if retry.tried.contains(&(ch.id, idx)) {
                    continue;
                }
                if let Some(entry) = self.cooldowns.get(&(ch.id, idx))
                    && entry.kind == CooldownKind::Transient
                    && entry.until > now
                {
                    let better = best
                        .as_ref()
                        .is_none_or(|(_, _, until)| entry.until < *until);
                    if better {
                        best = Some((ch.clone(), idx, entry.until));
                    }
                }
            }
        }
        match best {
            Some((ch, idx, _)) => {
                retry.tried.insert((ch.id, idx));
                Ok((ch, idx))
            }
            None => Err(AppError::BadRequest(format!(
                "no available channel for model {model}"
            ))),
        }
    }

    /// Whether a key's cooldown is currently active (expired entries are
    /// pruned and their backoff counters reset).
    fn cooldown_active(&self, channel_id: SnowflakeId, idx: usize) -> bool {
        let active = self
            .cooldowns
            .get(&(channel_id, idx))
            .is_some_and(|e| e.until > Instant::now());
        if active {
            return true;
        }
        self.cooldowns.remove(&(channel_id, idx));
        self.transient_fails.remove(&(channel_id, idx));
        false
    }

    fn any_ready_key(&self, ch: &CachedChannel) -> bool {
        ch.keys.iter().enumerate().any(|(i, k)| {
            k.status == LlmKeyStatus::Active && k.plain.is_some() && !self.cooldown_active(ch.id, i)
        })
    }

    /// Report an upstream failure (design §6.2 three-way classification):
    /// memory state mutates synchronously, DB persistence is spawned behind
    /// the per-channel lock.
    pub fn report_failure(&self, channel_id: SnowflakeId, key_index: usize, f: &UpstreamFailure) {
        match classify_failure(f) {
            FailureKind::Window => {
                let mut secs = f
                    .retry_after
                    .map(|d| d.as_secs())
                    .unwrap_or(WINDOW_DEFAULT_SECS)
                    .clamp(1, WINDOW_MAX_SECS);
                secs = secs.max(1);
                self.cooldowns.insert(
                    (channel_id, key_index),
                    CooldownUntil {
                        until: Instant::now() + Duration::from_secs(secs),
                        kind: CooldownKind::Window,
                        reason: f.message.chars().take(200).collect(),
                    },
                );
                tracing::warn!(channel = %channel_id, key_index, reset_secs = secs, "llm key window-limited");
            }
            FailureKind::Arrears => {
                self.set_key_state_mem(channel_id, key_index, LlmKeyStatus::Disabled, &f.message);
                tracing::warn!(channel = %channel_id, key_index, "llm key disabled (arrears/auth)");
                self.spawn_persist_disable(channel_id, key_index, &f.message);
            }
            FailureKind::Transient => {
                let count = {
                    let mut c = self
                        .transient_fails
                        .entry((channel_id, key_index))
                        .or_insert(0);
                    *c += 1;
                    *c
                };
                let mut secs = TRANSIENT_BASE_SECS
                    .saturating_mul(1u64 << (count - 1).min(4))
                    .min(TRANSIENT_MAX_SECS);
                if let Some(ra) = f.retry_after {
                    secs = secs.max(ra.as_secs().clamp(1, TRANSIENT_MAX_SECS));
                }
                self.cooldowns.insert(
                    (channel_id, key_index),
                    CooldownUntil {
                        until: Instant::now() + Duration::from_secs(secs),
                        kind: CooldownKind::Transient,
                        reason: f.message.chars().take(200).collect(),
                    },
                );
            }
        }
    }

    /// Report success: clear the transient backoff counter and any lingering
    /// transient cooldown.
    pub fn report_success(&self, channel_id: SnowflakeId, key_index: usize) {
        self.transient_fails.remove(&(channel_id, key_index));
        if self
            .cooldowns
            .get(&(channel_id, key_index))
            .is_some_and(|e| e.kind == CooldownKind::Transient)
        {
            self.cooldowns.remove(&(channel_id, key_index));
        }
    }

    /// Acquire the per-channel persistence lock (admin write paths share this
    /// serialization point with failure persists, design §6.2).
    pub async fn persist_lock(&self, id: SnowflakeId) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self.persist_locks.entry(id).or_default().clone();
        lock.lock_owned().await
    }

    /// Invalidate the model-directory cache after directory writes (§7.1:
    /// price/status changes take effect immediately, no 5min staleness
    /// window). Full reload — cheap at directory scale.
    pub async fn invalidate_model_cache(&self) {
        if let Err(err) = self.reload().await {
            tracing::error!(%err, "llm directory cache reload failed");
        }
    }

    /// Snapshot of live cooldowns (admin/status endpoints).
    pub fn cooldown_snapshot(&self) -> Vec<(SnowflakeId, usize, CooldownKind, Duration, String)> {
        let now = Instant::now();
        self.cooldowns
            .iter()
            .filter(|e| e.value().until > now)
            .map(|e| {
                (
                    SnowflakeId(*e.key().0),
                    e.key().1,
                    e.value().kind,
                    e.value().until.saturating_duration_since(now),
                    e.value().reason.clone(),
                )
            })
            .collect()
    }

    /// Enable one key in memory (admin action / future health recovery,
    /// design §6.3: a channel with any active key returns to `enabled`).
    pub fn enable_key_mem(&self, channel_id: SnowflakeId, key_index: usize) {
        self.set_key_state_mem(channel_id, key_index, LlmKeyStatus::Active, "");
    }

    fn set_key_state_mem(
        &self,
        channel_id: SnowflakeId,
        key_index: usize,
        status: LlmKeyStatus,
        reason: &str,
    ) {
        {
            let mut guard = self.cache.write().expect("llm cache lock");
            let cache = Arc::make_mut(&mut *guard);
            let Some(channel) = cache.channels.get(&channel_id) else {
                return;
            };
            let mut cloned = (**channel).clone();
            if key_index >= cloned.keys.len() {
                return;
            }
            cloned.keys[key_index].status = status;
            let any_active = cloned.keys.iter().any(|k| k.status == LlmKeyStatus::Active);
            if status == LlmKeyStatus::Disabled && !any_active {
                cloned.status = LlmChannelStatus::AutoDisabled;
                tracing::warn!(channel = %channel_id, "llm channel auto-disabled (all keys dead): {reason}");
            } else if status == LlmKeyStatus::Active
                && cloned.status == LlmChannelStatus::AutoDisabled
            {
                cloned.status = LlmChannelStatus::Enabled;
            }
            cache.channels.insert(channel_id, Arc::new(cloned));
            cache.rebuild_routes();
        }
        if status == LlmKeyStatus::Disabled {
            self.cooldowns.remove(&(channel_id, key_index));
            self.transient_fails.remove(&(channel_id, key_index));
        }
    }

    fn spawn_persist_disable(&self, channel_id: SnowflakeId, key_index: usize, reason: &str) {
        let Some(pool) = self.pool.clone() else {
            return;
        };
        let lock = self.persist_locks.entry(channel_id).or_default().clone();
        let reason = reason.to_owned();
        tokio::spawn(async move {
            let _guard = lock.lock().await;
            if let Err(err) = crate::llm::models::channel::update_key_status(
                &pool,
                None,
                channel_id,
                key_index,
                LlmKeyStatus::Disabled,
                Some(&reason),
            )
            .await
            {
                tracing::error!(%err, channel = %channel_id, "persist llm key disable failed");
            }
        });
    }
}

fn weighted_pick(channels: &[Arc<CachedChannel>]) -> Arc<CachedChannel> {
    if channels.len() == 1 {
        return channels[0].clone();
    }
    let weights: Vec<u64> = channels.iter().map(|ch| ch.weight.max(1) as u64).collect();
    let total: u64 = weights.iter().sum();
    let mut pick = rand_u64() % total.max(1);
    for (ch, w) in channels.iter().zip(weights) {
        if pick < w {
            return ch.clone();
        }
        pick = pick.saturating_sub(w);
    }
    channels[0].clone()
}

/// Apply a channel's model mapping to a public model name (design §7.2:
/// mapping applies only after channel selection; unmapped names pass through).
pub fn mapped_model(channel: &CachedChannel, public: &str) -> String {
    if let Some(serde_json::Value::Object(map)) = &channel.model_mapping
        && let Some(serde_json::Value::String(upstream)) = map.get(public)
    {
        return upstream.clone();
    }
    public.to_owned()
}

#[cfg(test)]
impl LlmRouter {
    /// Test helper mirroring `report_failure`'s arrears memory mutation
    /// without classification.
    fn disable_key_for_test(&self, channel_id: SnowflakeId, key_index: usize) {
        self.set_key_state_mem(channel_id, key_index, LlmKeyStatus::Disabled, "test");
    }
}

/// RAII upstream slot: releases the semaphore permit, decrements the
/// per-token/per-user in-flight counters and fan-out notifies every model
/// queue the channel serves (design §7.6 槽生命周期 — the upstream counts a
/// streaming connection until the stream terminates, so the guard must live
/// until then).
pub struct SlotPermit {
    router: Arc<LlmRouter>,
    tenant: String,
    groups: Vec<String>,
    models: Vec<String>,
    channel_id: SnowflakeId,
    _sem: Option<tokio::sync::OwnedSemaphorePermit>,
    token: Option<SnowflakeId>,
    user: Option<SnowflakeId>,
}

impl Drop for SlotPermit {
    fn drop(&mut self) {
        // permit dropped implicitly; counters decremented; queues woken.
        if let Some(token) = self.token
            && let Some(c) = self.router.token_inflight.get(&token)
        {
            c.fetch_sub(1, Ordering::AcqRel);
        }
        if let Some(user) = self.user
            && let Some(c) = self.router.user_inflight.get(&user)
        {
            c.fetch_sub(1, Ordering::AcqRel);
        }
        self.router
            .queues
            .notify_channel(&self.tenant, &self.groups, &self.models);
        let _ = self.channel_id;
    }
}

/// Slot acquisition failure.
pub enum SlotError {
    /// No route / no candidate at all — bubble the selection error.
    NoRoute(AppError),
    /// Admission rejection: 503 (queue full) or 429 (wait timeout /
    /// caller-level concurrency cap) with an optional Retry-After hint.
    Rejected {
        status: u16,
        message: String,
        retry_after_secs: Option<u64>,
    },
}

/// Inputs of [`LlmRouter::acquire_slot`] (design §7.6).
pub struct SlotRequest<'a> {
    pub cache: &'a ChannelCache,
    pub ctx: &'a ResolveCtx<'a>,
    pub model: &'a str,
    pub tier: QueueTier,
    /// (token_id, user_id) — per-caller concurrency accounting.
    pub caller: Option<(SnowflakeId, SnowflakeId)>,
    pub body_bytes: usize,
    /// Per-request wait budget shared across retry attempts (§7.6).
    pub deadline: Instant,
}

fn route_key_of(ctx: &ResolveCtx<'_>, model: &str) -> RouteKey {
    RouteKey {
        tenant: ctx.tenant.to_owned(),
        group: ctx.group.unwrap_or("default").to_owned(),
        model: model.to_owned(),
    }
}

impl LlmRouter {
    fn try_inflight(
        map: &DashMap<SnowflakeId, Arc<AtomicI64>>,
        id: SnowflakeId,
        limit: i64,
        what: &str,
    ) -> Result<(), SlotError> {
        let counter = map
            .entry(id)
            .or_insert_with(|| Arc::new(AtomicI64::new(0)))
            .clone();
        let now = counter.fetch_add(1, Ordering::AcqRel) + 1;
        if now > limit {
            counter.fetch_sub(1, Ordering::AcqRel);
            return Err(SlotError::Rejected {
                status: 429,
                message: format!("{what} concurrency limit reached ({limit})"),
                retry_after_secs: Some(5),
            });
        }
        Ok(())
    }

    fn rollback_inflight(map: &DashMap<SnowflakeId, Arc<AtomicI64>>, id: Option<SnowflakeId>) {
        if let Some(id) = id
            && let Some(c) = map.get(&id)
        {
            c.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn route_has_active_key(&self, cache: &ChannelCache, route: &RouteKey) -> bool {
        cache.candidates(route).is_some_and(|cands| {
            cands.iter().any(|ch| {
                ch.status == LlmChannelStatus::Enabled
                    && ch.keys.iter().any(|k| k.status == LlmKeyStatus::Active)
            })
        })
    }

    /// Record a request latency sample into the per-(tenant, model) EWMA
    /// (Retry-After estimation source, design §7.6).
    pub fn record_latency(&self, tenant: &str, model: &str, secs: f64) {
        let entry = self
            .latencies
            .entry((tenant.to_owned(), model.to_owned()))
            .or_insert_with(|| Arc::new(std::sync::Mutex::new(30.0)));
        if let Ok(mut ewma) = entry.lock() {
            *ewma = *ewma * 0.8 + secs.clamp(0.05, 600.0) * 0.2;
        }
    }

    /// Average request latency for Retry-After estimates (default 30s).
    pub fn avg_latency_secs(&self, tenant: &str, model: &str) -> f64 {
        self.latencies
            .get(&(tenant.to_owned(), model.to_owned()))
            .and_then(|e| e.lock().ok().map(|g| *g))
            .unwrap_or(30.0)
    }

    /// Total queued waiters (metrics).
    pub fn total_waiting(&self) -> usize {
        self.queues.total_waiting()
    }

    /// Acquire one upstream slot for a request (design §7.6): direct
    /// fast-path when a key has spare capacity, otherwise the FIFO wait
    /// queue under dual bounds. `deadline` is the per-request wait budget
    /// (shared across retry attempts).
    pub async fn acquire_slot(
        self: &Arc<Self>,
        req: &SlotRequest<'_>,
        retry: &mut RetryState,
    ) -> Result<(Arc<CachedChannel>, usize, SlotPermit), SlotError> {
        let SlotRequest {
            cache,
            ctx,
            model,
            tier,
            caller,
            body_bytes,
            deadline,
        } = *req;
        let (token, user) = match caller {
            Some((t, u)) => {
                Self::try_inflight(&self.token_inflight, t, TOKEN_MAX_CONCURRENT, "token")?;
                Self::try_inflight(&self.user_inflight, u, USER_MAX_CONCURRENT, "user")?;
                (Some(t), Some(u))
            }
            None => (None, None),
        };
        let rollback = |token, user| {
            Self::rollback_inflight(&self.token_inflight, token);
            Self::rollback_inflight(&self.user_inflight, user);
        };

        let route = route_key_of(ctx, model);
        let mut spins: u32 = 0;
        loop {
            match self.select_channel(cache, ctx, model, 0, retry) {
                Ok((ch, idx)) => {
                    let capacity = ch.keys[idx].max_concurrency.or(ch.default_max_concurrency);
                    match capacity {
                        None => {
                            let permit = SlotPermit {
                                router: self.clone(),
                                tenant: ch.tenant.clone(),
                                groups: ch.groups.clone(),
                                models: ch.models.clone(),
                                channel_id: ch.id,
                                _sem: None,
                                token,
                                user,
                            };
                            return Ok((ch, idx, permit));
                        }
                        Some(cap) => {
                            let sem = self
                                .slots
                                .entry((ch.id, idx))
                                .or_insert_with(|| Arc::new(Semaphore::new(cap.max(1) as usize)))
                                .clone();
                            if let Ok(permit) = sem.clone().try_acquire_owned() {
                                let guard = SlotPermit {
                                    router: self.clone(),
                                    tenant: ch.tenant.clone(),
                                    groups: ch.groups.clone(),
                                    models: ch.models.clone(),
                                    channel_id: ch.id,
                                    _sem: Some(permit),
                                    token,
                                    user,
                                };
                                return Ok((ch, idx, guard));
                            }
                        }
                    }
                }
                Err(err) => {
                    if !self.route_has_active_key(cache, &route) {
                        rollback(token, user);
                        return Err(SlotError::NoRoute(err));
                    }
                }
            }
            // All keys busy (or raced): enter the FIFO queue for this route.
            let queue = self.queues.get_or_create(&route);
            match queue.enqueue(body_bytes, tier.max_waiting, tier.max_waiting_bytes) {
                Err(full) => {
                    rollback(token, user);
                    return Err(SlotError::Rejected {
                        status: 503,
                        message: format!(
                            "overloaded: {} requests already waiting (max {})",
                            full.waiting, tier.max_waiting
                        ),
                        retry_after_secs: None,
                    });
                }
                Ok(ticket) => {
                    match ticket.wait(deadline).await {
                        Ok(()) => {
                            // Turn taken: retry selection while a slot should
                            // be free; bound hot spins for race safety.
                            spins += 1;
                            if spins > 64 {
                                rollback(token, user);
                                return Err(SlotError::Rejected {
                                    status: 429,
                                    message: "could not acquire a slot after queue turn".to_owned(),
                                    retry_after_secs: Some(2),
                                });
                            }
                        }
                        Err(timeout) => {
                            rollback(token, user);
                            let retry_after = ((timeout.position as f64 + 1.0)
                                * self.avg_latency_secs(ctx.tenant, model))
                            .ceil() as u64;
                            return Err(SlotError::Rejected {
                                status: 429,
                                message: "queue wait budget exhausted".to_owned(),
                                retry_after_secs: Some(retry_after.max(1)),
                            });
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::cache::{CachedKey, ChannelCache};
    use crate::llm::models::channel::{LlmChannel, LlmKeyEntry, keys_value};

    fn row(id: i64, priority: i64, keys: &[&str], models: &str) -> LlmChannel {
        let entries: Vec<LlmKeyEntry> = keys
            .iter()
            .map(|k| LlmKeyEntry {
                key: (*k).to_owned(),
                status: LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: None,
            })
            .collect();
        LlmChannel {
            id: SnowflakeId(id),
            tenant_id: Some("default".to_owned()),
            name: format!("ch-{id}"),
            provider: "openai".to_owned(),
            base_url: "https://example.test/v1".to_owned(),
            api_keys: keys_value(&entries),
            key_mode: LlmKeyMode::Polling,
            status: LlmChannelStatus::Enabled,
            models: models.to_owned(),
            model_mapping: None,
            priority,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            used_quota: 0,
            test_model: None,
            test_time: None,
            response_time: None,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn router(rows: Vec<LlmChannel>) -> Arc<LlmRouter> {
        // Tests bypass AES: build the cache directly from pre-decrypted keys.
        let mut cache = ChannelCache::default();
        for row in rows {
            let mut cached = ChannelCache::from_row(&row);
            cached.keys = row
                .api_keys
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .enumerate()
                        .map(|(i, _)| CachedKey {
                            plain: Some(format!("key-{}-{}", row.id.0, i)),
                            status: LlmKeyStatus::Active,
                            max_concurrency: None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            cache.channels.insert(cached.id, Arc::new(cached));
        }
        cache.rebuild_routes();
        LlmRouter::from_cache_for_test(cache)
    }

    fn ctx<'a>() -> ResolveCtx<'a> {
        ResolveCtx {
            tenant: "default",
            group: None,
            pin_channel: None,
        }
    }

    #[test]
    fn polling_rotates_keys() {
        let r = router(vec![row(1, 0, &["a", "b", "c"], "m1")]);
        let ch = r.cache.read().unwrap().channels[&SnowflakeId(1)].clone();
        let seq: Vec<usize> = (0..6).map(|_| r.select_key(&ch).unwrap()).collect();
        assert_eq!(seq, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn disabled_key_never_selected() {
        let r = router(vec![row(1, 0, &["a", "b"], "m1")]);
        r.disable_key_for_test(SnowflakeId(1), 0);
        let ch = r.cache.read().unwrap().channels[&SnowflakeId(1)].clone();
        for _ in 0..10 {
            assert_eq!(r.select_key(&ch).unwrap(), 1);
        }
    }

    #[test]
    fn tier_escalation_and_collapse() {
        let r = router(vec![row(1, 10, &["a"], "m1"), row(2, 1, &["b"], "m1")]);
        let mut retry = RetryState::default();
        let (a, _) = r
            .select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut retry,
            )
            .unwrap();
        assert_eq!(a.id, SnowflakeId(1));
        let (b, _) = r
            .select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                1,
                &mut retry,
            )
            .unwrap();
        assert_eq!(b.id, SnowflakeId(2));

        // Tier 0 channel fully cooled → attempt 0 collapses onto tier 1.
        r.report_failure(
            SnowflakeId(1),
            0,
            &UpstreamFailure {
                status: Some(429),
                message: "rate limited".into(),
                retry_after: None,
            },
        );
        let mut retry = RetryState::default();
        let (c, _) = r
            .select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut retry,
            )
            .unwrap();
        assert_eq!(c.id, SnowflakeId(2));
    }

    #[test]
    fn last_resort_transient_once_per_request() {
        let r = router(vec![row(1, 0, &["a"], "m1")]);
        r.report_failure(
            SnowflakeId(1),
            0,
            &UpstreamFailure {
                status: Some(429),
                message: "rate limited".into(),
                retry_after: None,
            },
        );
        let mut retry = RetryState::default();
        let (ch, idx) = r
            .select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut retry,
            )
            .expect("last-resort picks the transient-cooled key");
        assert_eq!((ch.id, idx), (SnowflakeId(1), 0));
        assert!(
            r.select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                1,
                &mut retry
            )
            .is_err(),
            "same key must not be hard-tried twice per request"
        );
        let mut fresh = RetryState::default();
        assert!(
            r.select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut fresh
            )
            .is_ok(),
            "a new request may last-resort again"
        );
    }

    #[test]
    fn window_cooldown_is_not_last_resorted() {
        let r = router(vec![row(1, 0, &["a"], "m1")]);
        r.report_failure(
            SnowflakeId(1),
            0,
            &UpstreamFailure {
                status: Some(429),
                message: "usage limit reached, will reset at 5pm".into(),
                retry_after: None,
            },
        );
        let mut retry = RetryState::default();
        assert!(
            r.select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut retry
            )
            .is_err(),
            "window-cooled keys must fast-fail, not hard-try"
        );
    }

    #[test]
    fn arrears_disables_key_and_channel() {
        let r = router(vec![row(1, 0, &["a"], "m1")]);
        r.report_failure(
            SnowflakeId(1),
            0,
            &UpstreamFailure {
                status: Some(401),
                message: "invalid api key".into(),
                retry_after: None,
            },
        );
        let ch = r.cache.read().unwrap().channels[&SnowflakeId(1)].clone();
        assert_eq!(ch.keys[0].status, LlmKeyStatus::Disabled);
        assert_eq!(ch.status, LlmChannelStatus::AutoDisabled);
        let mut retry = RetryState::default();
        assert!(
            r.select_channel(
                &r.cache.read().unwrap().clone(),
                &ctx(),
                "m1",
                0,
                &mut retry
            )
            .is_err()
        );
    }

    #[test]
    fn reset_semantics_beats_403_status() {
        let f = UpstreamFailure {
            status: Some(403),
            message: "You have exhausted your premium request allowance; will reset at 2026-09-11"
                .into(),
            retry_after: None,
        };
        assert_eq!(classify_failure(&f), FailureKind::Window);
    }

    #[test]
    fn classification_table() {
        let arrears_kw = UpstreamFailure {
            status: Some(429),
            message: "You exceeded your current quota, check your plan".into(),
            retry_after: None,
        };
        assert_eq!(classify_failure(&arrears_kw), FailureKind::Arrears);
        let transient = UpstreamFailure {
            status: Some(500),
            message: "upstream boom".into(),
            retry_after: None,
        };
        assert_eq!(classify_failure(&transient), FailureKind::Transient);
        let small_retry_after = UpstreamFailure {
            status: Some(429),
            message: "slow down".into(),
            retry_after: Some(Duration::from_secs(3)),
        };
        assert_eq!(classify_failure(&small_retry_after), FailureKind::Transient);
        let big_retry_after = UpstreamFailure {
            status: Some(429),
            message: "slow down".into(),
            retry_after: Some(Duration::from_secs(3600)),
        };
        assert_eq!(classify_failure(&big_retry_after), FailureKind::Window);
    }

    #[test]
    fn builtin_model_seed_resolves() {
        let r = router(vec![]);
        assert!(r.model_info("default", "gpt-4o").is_some());
        assert!(r.model_info("default", "totally-unknown-model").is_none());
    }

    #[test]
    fn success_clears_transient_state() {
        let r = router(vec![row(1, 0, &["a"], "m1")]);
        r.report_failure(
            SnowflakeId(1),
            0,
            &UpstreamFailure {
                status: Some(429),
                message: "rl".into(),
                retry_after: None,
            },
        );
        r.report_success(SnowflakeId(1), 0);
        let ch = r.cache.read().unwrap().channels[&SnowflakeId(1)].clone();
        assert!(r.select_key(&ch).is_some());
    }
}
