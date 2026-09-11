//! FIFO wait queue between callers and upstream slots (design §7.6).
//!
//! Strict first-come-first-served across keys via ticket numbering:
//! `tokio::sync::Semaphore` fairness only holds within a single key, so the
//! queue adds a `serving` counter — only the waiter whose ticket equals
//! `serving` may grab a freed slot, then `serving` advances.
//!
//! Correctness rules (design §7.6 queue pins):
//! - CV discipline: **every** state change notifies waiters — slot release
//!   (`notify_all` fan-out), `serving` advance, waiter removal. tokio's
//!   `notify_waiters` stores no permits, so a missed notify means a stall
//!   until the next event.
//! - Head cancel: a dropped `OwnedWaitTicket` whose ticket equals `serving`
//!   advances `serving` and notifies — without this, one disconnect while
//!   queued deadlocks the whole queue.
//! - Dual bounds: depth (`max_waiting`) and bytes (`max_waiting_bytes`);
//!   either exceeded rejects the newcomer immediately (caller maps to 503).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Notify;

use crate::llm::cache::RouteKey;

struct QueueState {
    waiters: VecDeque<Arc<WaiterSlot>>,
    bytes: usize,
}

struct WaiterSlot {
    ticket: u64,
    bytes: usize,
    /// Set when the waiter took its turn (left the queue) so a stale drop
    /// does not double-advance `serving`.
    done: AtomicBool,
}

/// One FIFO queue (per `RouteKey`), always shared as `Arc`.
pub struct WaitQueue {
    next_ticket: AtomicU64,
    serving: AtomicU64,
    state: Mutex<QueueState>,
    notify: Notify,
}

/// Queue admission rejected: depth or byte bound exceeded → caller returns
/// 503 overloaded ("处理不过来，排队也白等").
#[derive(Debug)]
pub struct QueueFull {
    pub waiting: usize,
}

/// Timed out waiting for a slot → caller returns 429 + Retry-After.
#[derive(Debug)]
pub struct WaitTimeout {
    /// Queue position at timeout, for the Retry-After estimate.
    pub position: usize,
}

impl WaitQueue {
    /// New empty queue.
    pub fn new() -> Self {
        Self {
            next_ticket: AtomicU64::new(0),
            serving: AtomicU64::new(0),
            state: Mutex::new(QueueState {
                waiters: VecDeque::new(),
                bytes: 0,
            }),
            notify: Notify::new(),
        }
    }

    /// Current queue depth.
    pub fn depth(&self) -> usize {
        self.state.lock().expect("waitqueue lock").waiters.len()
    }

    /// Enqueue a waiter under the dual bounds. On success the caller owns an
    /// `OwnedWaitTicket`; dropping it before the turn arrives cancels the
    /// waiter and (head only) advances `serving`.
    pub fn enqueue(
        self: &Arc<Self>,
        bytes: usize,
        max_waiting: usize,
        max_waiting_bytes: usize,
    ) -> Result<OwnedWaitTicket, QueueFull> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        let slot = Arc::new(WaiterSlot {
            ticket,
            bytes,
            done: AtomicBool::new(false),
        });
        {
            let mut state = self.state.lock().expect("waitqueue lock");
            if state.waiters.len() >= max_waiting || state.bytes + bytes > max_waiting_bytes {
                return Err(QueueFull {
                    waiting: state.waiters.len(),
                });
            }
            state.waiters.push_back(slot.clone());
            state.bytes += bytes;
        }
        Ok(OwnedWaitTicket {
            queue: self.clone(),
            slot,
        })
    }

    async fn wait_turn(&self, slot: &WaiterSlot, deadline: Instant) -> Result<(), Duration> {
        loop {
            if slot.ticket == self.serving.load(Ordering::Acquire) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(remaining);
            }
            let notified = tokio::time::timeout(remaining, self.notify.notified());
            if notified.await.is_err() {
                return Err(Duration::ZERO);
            }
        }
    }

    fn take_turn(&self, slot: &WaiterSlot) {
        if slot.done.swap(true, Ordering::AcqRel) {
            return;
        }
        self.remove_slot(slot);
        self.serving.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    /// Notify all current waiters (slot-release fan-out / state changes).
    pub fn notify_all(&self) {
        self.notify.notify_waiters();
    }

    fn remove_slot(&self, slot: &WaiterSlot) {
        let mut state = self.state.lock().expect("waitqueue lock");
        if let Some(pos) = state.waiters.iter().position(|w| w.ticket == slot.ticket) {
            state.waiters.remove(pos);
            state.bytes = state.bytes.saturating_sub(slot.bytes);
        }
    }

    fn position_of(&self, ticket: u64) -> usize {
        let state = self.state.lock().expect("waitqueue lock");
        state
            .waiters
            .iter()
            .position(|w| w.ticket == ticket)
            .unwrap_or(0)
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Owned queue admission handle. Drop before `wait` completes = cancel.
pub struct OwnedWaitTicket {
    queue: Arc<WaitQueue>,
    slot: Arc<WaiterSlot>,
}

impl OwnedWaitTicket {
    /// Wait for this ticket's turn within the deadline. On success the turn
    /// is consumed (waiter leaves the queue, `serving` advances, next waiter
    /// notified) — the caller then retries slot acquisition.
    pub async fn wait(self, deadline: Instant) -> Result<(), WaitTimeout> {
        match self.queue.wait_turn(&self.slot, deadline).await {
            Ok(()) => {
                self.queue.take_turn(&self.slot);
                Ok(())
            }
            Err(_) => {
                let position = self.queue.position_of(self.slot.ticket);
                Err(WaitTimeout { position })
            }
        }
    }
}

impl Drop for OwnedWaitTicket {
    fn drop(&mut self) {
        // Cancellation path (timeout handled by caller, disconnect = drop):
        // remove the waiter; if it was the head, advance `serving` and wake
        // the next — the head-cancel deadlock guard.
        if !self.slot.done.swap(true, Ordering::AcqRel) {
            self.queue.remove_slot(&self.slot);
            if self.slot.ticket == self.queue.serving.load(Ordering::Acquire) {
                self.queue.serving.fetch_add(1, Ordering::AcqRel);
                self.queue.notify.notify_waiters();
            }
        }
    }
}

/// Registry of per-route queues.
pub struct QueueRegistry {
    queues: DashMap<RouteKey, Arc<WaitQueue>>,
}

impl QueueRegistry {
    /// New empty registry.
    pub fn new() -> Self {
        Self {
            queues: DashMap::new(),
        }
    }

    /// Get or create the queue for a route key.
    pub fn get_or_create(&self, key: &RouteKey) -> Arc<WaitQueue> {
        self.queues
            .entry(key.clone())
            .or_insert_with(|| Arc::new(WaitQueue::new()))
            .clone()
    }

    /// Notify every waiter of one route (slot-release fan-out).
    pub fn notify_route(&self, key: &RouteKey) {
        if let Some(q) = self.queues.get(key) {
            q.notify_all();
        }
    }

    /// Notify all routes a channel serves (a freed key slot is shared across
    /// every model queue the channel serves, design §7.6).
    pub fn notify_channel(&self, tenant: &str, groups: &[String], models: &[String]) {
        for group in groups {
            for model in models {
                let key = RouteKey {
                    tenant: tenant.to_owned(),
                    group: group.clone(),
                    model: model.clone(),
                };
                self.notify_route(&key);
            }
        }
    }

    /// Total waiting count across routes (metrics).
    pub fn total_waiting(&self) -> usize {
        self.queues.iter().map(|q| q.value().depth()).sum()
    }
}

impl Default for QueueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deadline(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    #[tokio::test]
    async fn strict_fifo_order() {
        let q = Arc::new(WaitQueue::new());
        let t0 = q.enqueue(0, 10, 1_000_000).expect("enqueue");
        let t1 = q.enqueue(0, 10, 1_000_000).expect("enqueue");
        let t2 = q.enqueue(0, 10, 1_000_000).expect("enqueue");

        // All three are pending; only the head may take a turn.
        let h0 = tokio::spawn(async move { t0.wait(deadline(3)).await.is_ok() });
        let h1 = tokio::spawn(async move { t1.wait(deadline(3)).await.is_ok() });
        let h2 = tokio::spawn(async move { t2.wait(deadline(3)).await.is_ok() });

        q.notify_all();
        assert!(h0.await.expect("join"), "head takes the first turn");
        q.notify_all();
        assert!(h1.await.expect("join"), "second waits for the first");
        q.notify_all();
        assert!(h2.await.expect("join"), "third waits for the second");
    }

    #[tokio::test]
    async fn head_cancel_advances_serving() {
        let q = Arc::new(WaitQueue::new());
        let t0 = q.enqueue(0, 10, 1_000).expect("enqueue");
        let t1 = q.enqueue(0, 10, 1_000).expect("enqueue");
        // Head cancels before its turn (drop without waiting).
        drop(t0);
        // t1 must now be eligible without any further notify.
        assert!(
            t1.wait(deadline(1)).await.is_ok(),
            "head cancel must advance serving"
        );
    }

    #[tokio::test]
    async fn depth_and_byte_bounds_reject() {
        let q = Arc::new(WaitQueue::new());
        let t1 = q.enqueue(10, 1, 1_000).expect("first fits");
        assert!(q.enqueue(10, 1, 1_000).is_err(), "depth bound");
        drop(t1);
        let q2 = Arc::new(WaitQueue::new());
        let t2 = q2.enqueue(700, 10, 1_000).expect("first fits");
        assert!(q2.enqueue(700, 10, 1_000).is_err(), "byte bound");
        drop(t2);
    }

    #[tokio::test]
    async fn timeout_reports_position() {
        let q = Arc::new(WaitQueue::new());
        let _t0 = q.enqueue(0, 10, 1_000).expect("enqueue");
        let t1 = q.enqueue(0, 10, 1_000).expect("enqueue");
        // t1 is not the head; nothing notifies; must time out.
        let err = t1.wait(deadline(0)).await.expect_err("must time out");
        assert!(err.position >= 1);
    }
}
