//! Telemetry batch pipeline (integration.md §6.3 吞吐分级 / §10.7 批 steps).
//!
//! `kind=telemetry` envelopes bypass the per-message transaction: they are
//! buffered per channel and flushed as ONE transaction (time window 200ms or
//! 100 items, whichever first). Overflow beyond `max_buffered` drops (telemetry
//! is lossy-tolerant by design) with a counter for the health API.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use serde_json::Value;

/// One buffered telemetry envelope, captured with everything the flush needs
/// (no channel lookup required later).
#[derive(Debug, Clone)]
pub struct BatchItem {
    pub receipt_id: i64,
    pub external_id: String,
    pub hash: String,
    pub envelope: Value,
    /// Normalized payload ready for `tx_insert` (external_id already merged).
    pub target_data: Value,
    pub target_type: String,
    pub tenant_id: String,
    pub received_at: crate::utils::tz::Timestamp,
}

/// Per-channel batch statistics (health API source).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BatchStats {
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub buffered: u64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub flushed_items: u64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub flush_batches: u64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub dropped_overflow: u64,
    /// Total items ever submitted (diagnostics).
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub submitted: u64,
    /// Last flush error (diagnostics; None when healthy).
    pub last_flush_error: Option<String>,
}

pub enum SubmitOutcome {
    Buffered,
    /// Buffer reached flush size — caller (flusher) should drain soon.
    Full,
    /// Over `max_buffered` — dropped (counted).
    Dropped,
}

pub(crate) struct ChannelBuffer {
    items: Vec<BatchItem>,
    first_at: Option<Instant>,
}

/// Per-channel telemetry buffers + stats. Flushing is owned by the pipeline
/// (`Pipeline::flush_batch_items`) via a Weak task (see `spawn_batch_flusher`).
pub struct TelemetryBatcher {
    buffers: DashMap<i64, ChannelBuffer>,
    stats: DashMap<i64, BatchStats>,
    max_n: usize,
    window: Duration,
    max_buffered: usize,
}

impl TelemetryBatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            buffers: DashMap::new(),
            stats: DashMap::new(),
            max_n: 100,
            window: Duration::from_millis(200),
            max_buffered: 400,
        }
    }

    /// Buffer one item.
    pub fn submit(&self, channel_id: i64, item: BatchItem) -> SubmitOutcome {
        self.stat(channel_id, |s| s.submitted += 1);
        let mut entry = self
            .buffers
            .entry(channel_id)
            .or_insert_with(|| ChannelBuffer {
                items: Vec::new(),
                first_at: None,
            });
        if entry.items.len() >= self.max_buffered {
            drop(entry);
            self.stat(channel_id, |s| s.dropped_overflow += 1);
            return SubmitOutcome::Dropped;
        }
        entry.first_at.get_or_insert(Instant::now());
        entry.items.push(item);
        let len = entry.items.len();
        drop(entry);
        self.stat(channel_id, |s| s.buffered = len as u64);
        if len >= self.max_n {
            SubmitOutcome::Full
        } else {
            SubmitOutcome::Buffered
        }
    }

    /// Drain buffers that hit the size or time-window trigger.
    /// Returns `(channel_id, items)` pairs ready to flush.
    pub fn drain_ready(&self) -> HashMap<i64, Vec<BatchItem>> {
        let mut ready = HashMap::new();
        let keys: Vec<i64> = self.buffers.iter().map(|e| *e.key()).collect();
        for id in keys {
            let take = {
                let mut entry = match self.buffers.get_mut(&id) {
                    Some(e) => e,
                    None => continue,
                };
                let time_up = entry.first_at.is_some_and(|t| t.elapsed() >= self.window);
                let size_up = entry.items.len() >= self.max_n;
                if (time_up || size_up) && !entry.items.is_empty() {
                    let items = std::mem::take(&mut entry.items);
                    entry.first_at = None;
                    Some(items)
                } else {
                    None
                }
            };
            if let Some(items) = take {
                self.stat(id, |s| s.buffered = 0);
                ready.insert(id, items);
            }
        }
        ready
    }

    /// Stats snapshot for the health API.
    #[must_use]
    pub fn stats_snapshot(&self) -> HashMap<i64, BatchStats> {
        self.stats
            .iter()
            .map(|s| (*s.key(), s.value().clone()))
            .collect()
    }

    fn stat(&self, channel_id: i64, f: impl FnOnce(&mut BatchStats)) {
        let mut entry = self.stats.entry(channel_id).or_default();
        f(&mut entry);
    }

    /// Record a successful flush.
    pub fn record_flush(&self, channel_id: i64, items: u64) {
        self.stat(channel_id, |s| {
            s.flush_batches += 1;
            s.flushed_items += items;
        });
    }

    /// Record a failed flush (diagnostics).
    pub fn record_flush_error(&self, channel_id: i64, error: &str) {
        self.stat(channel_id, |s| s.last_flush_error = Some(error.to_string()));
    }
}

impl Default for TelemetryBatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64) -> BatchItem {
        BatchItem {
            receipt_id: id,
            external_id: format!("t-{id}"),
            hash: format!("hash-{id}"),
            envelope: Value::Null,
            target_data: Value::Null,
            target_type: "ingress_note".into(),
            tenant_id: "default".into(),
            received_at: crate::utils::tz::now_utc(),
        }
    }

    #[test]
    fn size_trigger_and_drain() {
        let b = TelemetryBatcher::new();
        for i in 0..(b.max_n as i64) {
            assert!(matches!(
                b.submit(1, item(i)),
                SubmitOutcome::Buffered | SubmitOutcome::Full
            ));
        }
        let ready = b.drain_ready();
        assert_eq!(ready[&1].len(), b.max_n);
        // Drained: nothing further until window/size again.
        assert!(b.drain_ready().is_empty());
    }

    #[tokio::test]
    async fn time_window_triggers() {
        let b = TelemetryBatcher::new();
        b.submit(7, item(1));
        assert!(b.drain_ready().is_empty(), "window not elapsed yet");
        tokio::time::sleep(b.window + Duration::from_millis(20)).await;
        let ready = b.drain_ready();
        assert_eq!(ready[&7].len(), 1);
    }

    #[test]
    fn overflow_drops_and_counts() {
        let b = TelemetryBatcher::new();
        for i in 0..(b.max_buffered as i64) {
            b.submit(2, item(i));
        }
        assert!(matches!(b.submit(2, item(999)), SubmitOutcome::Dropped));
        let stats = b.stats_snapshot();
        assert_eq!(stats[&2].dropped_overflow, 1);
    }
}
