//! Presence store — process-local "who is available right now" primitive.
//!
//! Business-agnostic kernel primitive (architecture §5.3): answers
//! "is `(tenant, subject)` present, what's their wish, how many live
//! connections, who can take work". Consumers (chat assign, CRM lead
//! routing, forum moderator lists, …) only depend on the [`PresenceStore`]
//! trait — swapping `InMemoryPresenceStore` for a Redis backend is a
//! single implementation change.
//!
//! Design constraints (prevent rework):
//! - keyed by `(tenant_id, subject_id)` — multi-tenant safe from day one
//! - presence (machine fact, memory) and availability (human wish, written
//!   via `set_manual`) are separate; consumers read only the merged
//!   [`effective`] status
//! - TTL expiry via a background reaper; an SSE disconnect is not an
//!   immediate offline (heartbeat stays fresh within the grace window)
//! - events are emitted only on actual state change (`Transition` is
//!   `Some` only when the effective status flips); the map is the source
//!   of truth, never reconstruct state from an event stream

use std::sync::Arc;
use std::time::{Duration, Instant};

/// Human-set availability (persistent wish). Written by upper layers via
/// `set_manual` (e.g. a CRM agent profile syncing "in a meeting").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Availability {
    Online,
    Busy,
    Away,
    Offline,
}

impl Availability {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Away => "away",
            Self::Offline => "offline",
        }
    }
}

/// Effective status — the only thing consumers read. Machine facts
/// (connections + heartbeat) merged with the human wish (manual).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceStatus {
    Online,
    Busy,
    Away,
    Offline,
}

impl PresenceStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Away => "away",
            Self::Offline => "offline",
        }
    }
}

impl From<Availability> for PresenceStatus {
    fn from(value: Availability) -> Self {
        match value {
            Availability::Online => Self::Online,
            Availability::Busy => Self::Busy,
            Availability::Away => Self::Away,
            Availability::Offline => Self::Offline,
        }
    }
}

/// A single subject's presence record inside the store.
#[derive(Debug, Clone)]
pub struct PresenceEntry {
    pub tenant_id: String,
    pub subject_id: i64,
    /// Active SSE connection count (multi-tab counts multiple).
    pub conns: u32,
    /// Most recent liveness signal (heartbeat or SSE connect).
    pub last_seen: Instant,
    /// Human-set wish; overrides machine facts when present.
    pub manual: Option<Availability>,
}

impl PresenceEntry {
    /// Merge rule (architecture §5.3): manual wins, then any live
    /// connection or fresh heartbeat => online, else offline.
    #[must_use]
    pub fn effective(&self, ttl: Duration) -> PresenceStatus {
        if let Some(m) = self.manual {
            return m.into();
        }
        if self.conns > 0 || self.last_seen.elapsed() < ttl {
            PresenceStatus::Online
        } else {
            PresenceStatus::Offline
        }
    }
}

/// A state change produced by a store mutation. `Some` only when the
/// effective status actually flipped — callers emit `presence.*` events
/// only for these (30s heartbeats with no change produce no event).
#[derive(Debug, Clone)]
pub struct Transition {
    pub tenant_id: String,
    pub subject_id: i64,
    pub from: Option<PresenceStatus>,
    pub to: PresenceStatus,
}

/// Aggregate counts for a tenant (live dashboard).
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct PresenceCounts {
    pub online: u64,
    pub busy: u64,
    pub away: u64,
    pub offline: u64,
}

/// Presence store interface. Consumers depend only on this trait so a
/// Redis backend can replace `InMemoryPresenceStore` without touching them.
pub trait PresenceStore: Send + Sync {
    /// A live SSE connection was established (`conns += 1`, refresh
    /// `last_seen`). Returns a `Transition` when the effective status
    /// changed (e.g. offline -> online on first connect).
    fn connect(&self, tenant_id: &str, subject_id: i64) -> Option<Transition>;

    /// A live SSE connection dropped (`conns -= 1`, no `last_seen`
    /// refresh). Not immediately offline while the heartbeat is fresh.
    fn disconnect(&self, tenant_id: &str, subject_id: i64) -> Option<Transition>;

    /// Liveness heartbeat: refresh `last_seen`. Never forces offline by
    /// itself; the reaper converts staleness into offline.
    fn touch(&self, tenant_id: &str, subject_id: i64) -> Option<Transition>;

    /// Set/clear the human-set availability wish. `None` clears it.
    fn set_manual(
        &self,
        tenant_id: &str,
        subject_id: i64,
        status: Option<Availability>,
    ) -> Option<Transition>;

    /// Current effective status (no side effects).
    fn status(&self, tenant_id: &str, subject_id: i64) -> PresenceStatus;

    /// All entries for a tenant (agent/operator list rendering).
    fn snapshot(&self, tenant_id: &str) -> Vec<PresenceEntry>;

    /// Subjects that can take work right now (effective in {Online, Busy}
    /// and not manually away/offline). chat.assign / CRM routing consume this.
    fn available(&self, tenant_id: &str) -> Vec<i64>;

    /// Aggregate counts by effective status (live dashboard).
    fn counts(&self, tenant_id: &str) -> PresenceCounts;

    /// Sweep stale entries (called by the reaper task). Returns transitions
    /// for entries that expired into offline so the caller can emit events.
    fn sweep_expired(&self, ttl: Duration) -> Vec<Transition>;

    /// Number of subjects tracked (diagnostics/tests).
    fn len(&self) -> usize;

    /// Whether the store is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory presence store (single-instance discipline). Backed by a
/// `dashmap::DashMap` (already a platform dependency — lock-free reads,
/// no poisoning to handle). Reads (status/snapshot/available/counts)
/// never block a writer for long.
pub struct InMemoryPresenceStore {
    inner: dashmap::DashMap<(String, i64), PresenceEntry>,
}

impl Default for InMemoryPresenceStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryPresenceStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: dashmap::DashMap::new(),
        }
    }

    /// Heartbeat TTL used by mutation/read methods for the merge rule.
    /// Staleness into offline is always resolved by the reaper via
    /// `sweep_expired(ttl)` — these reads only need a "fresh" notion.
    fn heartbeat_ttl() -> Duration {
        Duration::from_secs(75)
    }
}
impl PresenceStore for InMemoryPresenceStore {
    fn connect(&self, tenant_id: &str, subject_id: i64) -> Option<Transition> {
        let ttl = Self::heartbeat_ttl();
        let key = (tenant_id.to_string(), subject_id);
        let prev = self.inner.get(&key).map(|e| e.effective(ttl));
        let mut entry = self.inner.entry(key).or_insert(PresenceEntry {
            tenant_id: tenant_id.to_string(),
            subject_id,
            conns: 0,
            last_seen: Instant::now(),
            manual: None,
        });
        entry.conns = entry.conns.saturating_add(1);
        // A connection is the strongest liveness signal — refresh so a
        // reconnect after a network blip resurrects immediately (no waiting
        // for the next heartbeat).
        entry.last_seen = Instant::now();
        let to = entry.effective(ttl);
        transition(tenant_id, subject_id, prev, to)
    }

    fn disconnect(&self, tenant_id: &str, subject_id: i64) -> Option<Transition> {
        let ttl = Self::heartbeat_ttl();
        let key = (tenant_id.to_string(), subject_id);
        let mut entry = self.inner.get_mut(&key)?;
        let prev = entry.effective(ttl);
        entry.conns = entry.conns.saturating_sub(1);
        // Do NOT refresh last_seen — disconnecting is not a liveness signal.
        let to = entry.effective(ttl);
        let should_remove =
            to == PresenceStatus::Offline && entry.manual.is_none() && entry.conns == 0;
        let t = transition(tenant_id, subject_id, Some(prev), to);
        if should_remove {
            drop(entry);
            // Nothing left to track: no connection, stale heartbeat, no wish.
            self.inner.remove(&key);
        }
        t
    }

    fn touch(&self, tenant_id: &str, subject_id: i64) -> Option<Transition> {
        let ttl = Self::heartbeat_ttl();
        let key = (tenant_id.to_string(), subject_id);
        let prev = self.inner.get(&key).map(|e| e.effective(ttl));
        let mut entry = self.inner.entry(key).or_insert(PresenceEntry {
            tenant_id: tenant_id.to_string(),
            subject_id,
            conns: 0,
            last_seen: Instant::now(),
            manual: None,
        });
        entry.last_seen = Instant::now();
        let to = entry.effective(ttl);
        transition(tenant_id, subject_id, prev, to)
    }

    fn set_manual(
        &self,
        tenant_id: &str,
        subject_id: i64,
        status: Option<Availability>,
    ) -> Option<Transition> {
        let ttl = Self::heartbeat_ttl();
        let key = (tenant_id.to_string(), subject_id);
        if status.is_none() && !self.inner.contains_key(&key) {
            return None;
        }
        let prev = self.inner.get(&key).map(|e| e.effective(ttl));
        let mut entry = self.inner.entry(key).or_insert(PresenceEntry {
            tenant_id: tenant_id.to_string(),
            subject_id,
            conns: 0,
            last_seen: Instant::now(),
            manual: None,
        });
        entry.manual = status;
        let to = entry.effective(ttl);
        transition(tenant_id, subject_id, prev, to)
    }

    fn status(&self, tenant_id: &str, subject_id: i64) -> PresenceStatus {
        let ttl = Self::heartbeat_ttl();
        self.inner
            .get(&(tenant_id.to_string(), subject_id))
            .map(|e| e.effective(ttl))
            .unwrap_or(PresenceStatus::Offline)
    }

    fn snapshot(&self, tenant_id: &str) -> Vec<PresenceEntry> {
        self.inner
            .iter()
            .filter(|r| r.value().tenant_id == tenant_id)
            .map(|r| r.value().clone())
            .collect()
    }

    fn available(&self, tenant_id: &str) -> Vec<i64> {
        let ttl = Self::heartbeat_ttl();
        let mut ids: Vec<i64> = self
            .inner
            .iter()
            .filter(|r| r.value().tenant_id == tenant_id)
            .filter(|r| {
                matches!(
                    r.value().effective(ttl),
                    PresenceStatus::Online | PresenceStatus::Busy
                )
            })
            .map(|r| r.value().subject_id)
            .collect();
        ids.sort_unstable();
        ids
    }

    fn counts(&self, tenant_id: &str) -> PresenceCounts {
        let ttl = Self::heartbeat_ttl();
        let mut counts = PresenceCounts::default();
        for r in self.inner.iter() {
            let e = r.value();
            if e.tenant_id != tenant_id {
                continue;
            }
            match e.effective(ttl) {
                PresenceStatus::Online => counts.online += 1,
                PresenceStatus::Busy => counts.busy += 1,
                PresenceStatus::Away => counts.away += 1,
                PresenceStatus::Offline => counts.offline += 1,
            }
        }
        counts
    }

    fn sweep_expired(&self, ttl: Duration) -> Vec<Transition> {
        let mut transitions = Vec::new();
        let now = Instant::now();
        self.inner.retain(|(tenant, subject), entry| {
            let expired = entry.manual.is_none()
                && entry.conns == 0
                && now.duration_since(entry.last_seen) >= ttl;
            if expired {
                // The effective status flips to offline the moment the
                // heartbeat goes stale, but that flip is lazy (computed on
                // read). The reaper is what surfaces it to subscribers:
                // a tracked non-manual entry being removed was alive up to
                // its last heartbeat, so emit online -> offline once here.
                transitions.push(Transition {
                    tenant_id: tenant.clone(),
                    subject_id: *subject,
                    from: Some(PresenceStatus::Online),
                    to: PresenceStatus::Offline,
                });
                false // remove
            } else {
                true
            }
        });
        transitions
    }

    fn len(&self) -> usize {
        self.inner.len()
    }
}

fn transition(
    tenant_id: &str,
    subject_id: i64,
    from: Option<PresenceStatus>,
    to: PresenceStatus,
) -> Option<Transition> {
    if from == Some(to) {
        return None;
    }
    Some(Transition {
        tenant_id: tenant_id.to_string(),
        subject_id,
        from,
        to,
    })
}

// The in-memory store is TTL-agnostic except `sweep_expired`, but the
// mutation methods use `heartbeat_ttl()` to compute effective transitions.
// Default to the config default (75s) so the primitive works standalone
// (tests, non-configured embedding); the real TTL is supplied to reaper
// via `sweep_expired`. To avoid threading config into the store, mutations
// here use a fixed generous TTL for the merge rule only — staleness is
// always resolved by the reaper, never by these reads.

/// Spawn the presence reaper: periodically sweeps stale entries to offline.
/// Emits `presence.offline` events for each transition. Uses `predefined`
/// event names so any frontend subscribing to `presence.*` sees them.
pub fn spawn_reaper(
    store: Arc<dyn PresenceStore>,
    eventbus: crate::eventbus::EventBus,
    ttl: Duration,
    sweep_interval: Duration,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(sweep_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    for t in store.sweep_expired(ttl) {
                        emit_transition(&eventbus, &t);
                    }
                }
                _ = shutdown_rx.changed() => {
                    tracing::info!("presence reaper shutting down");
                    break;
                }
            }
        }
    })
}

/// Emit a `presence.*` event for a transition (event type chosen by `to`).
pub fn emit_transition(eventbus: &crate::eventbus::EventBus, t: &Transition) {
    let event_type = match t.to {
        PresenceStatus::Online => "presence.online",
        PresenceStatus::Offline => "presence.offline",
        _ => "presence.status",
    };
    eventbus.emit(crate::event::Event::Custom {
        source: "presence".to_string(),
        event_type: event_type.to_string(),
        data: serde_json::json!({
            "tenant_id": t.tenant_id,
            "subject_id": t.subject_id.to_string(),
            "status": t.to.as_str(),
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> InMemoryPresenceStore {
        InMemoryPresenceStore::new()
    }

    #[test]
    fn connect_transitions_offline_to_online() {
        let s = store();
        let t = s.connect("t1", 1).unwrap();
        assert_eq!(t.from, None);
        assert_eq!(t.to, PresenceStatus::Online);
        assert_eq!(s.status("t1", 1), PresenceStatus::Online);
        // second connect (new tab) does not re-emit
        assert!(s.connect("t1", 1).is_none());
    }

    #[test]
    fn disconnect_keeps_online_during_grace() {
        let s = store();
        s.connect("t1", 1);
        // heartbeat still fresh → disconnect not offline
        let t = s.disconnect("t1", 1);
        assert!(t.is_none());
        assert_eq!(s.status("t1", 1), PresenceStatus::Online);
    }

    #[test]
    fn manual_away_overrides_heartbeat() {
        let s = store();
        s.connect("t1", 1);
        let t = s.set_manual("t1", 1, Some(Availability::Away)).unwrap();
        assert_eq!(t.to, PresenceStatus::Away);
        assert_eq!(s.status("t1", 1), PresenceStatus::Away);
        assert!(s.available("t1").is_empty());
        // clearing manual returns to online (conns still live)
        let t = s.set_manual("t1", 1, None).unwrap();
        assert_eq!(t.to, PresenceStatus::Online);
        assert_eq!(s.available("t1"), vec![1]);
    }

    #[test]
    fn tenant_isolation() {
        let s = store();
        s.connect("t1", 1);
        s.connect("t2", 2);
        assert_eq!(s.snapshot("t1").len(), 1);
        assert_eq!(s.available("t1"), vec![1]);
        assert_eq!(s.available("t2"), vec![2]);
        assert_eq!(s.counts("t1").online, 1);
    }

    #[test]
    fn reaper_expires_stale_to_offline() {
        let s = store();
        s.connect("t1", 1);
        s.disconnect("t1", 1);
        // force staleness: insert with old last_seen
        {
            let mut entry = s.inner.get_mut(&("t1".to_string(), 1)).unwrap();
            entry.last_seen = Instant::now() - Duration::from_secs(200);
        }
        let transitions = s.sweep_expired(Duration::from_secs(75));
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].to, PresenceStatus::Offline);
        assert!(s.status("t1", 1) == PresenceStatus::Offline);
        assert!(s.len() == 0);
    }

    #[test]
    fn manual_offline_survives_reaper() {
        let s = store();
        s.set_manual("t1", 1, Some(Availability::Offline));
        let transitions = s.sweep_expired(Duration::from_secs(75));
        assert!(transitions.is_empty());
        assert_eq!(s.status("t1", 1), PresenceStatus::Offline);
        assert!(s.len() == 1);
    }

    #[test]
    fn counts_by_effective() {
        let s = store();
        s.connect("t1", 1); // online
        s.connect("t1", 2); // online
        s.set_manual("t1", 2, Some(Availability::Busy)); // busy
        s.set_manual("t1", 3, Some(Availability::Away)); // away (no conns needed)
        let c = s.counts("t1");
        assert_eq!(c.online, 1);
        assert_eq!(c.busy, 1);
        assert_eq!(c.away, 1);
    }
}
