//! TRACE_CTX — task-local trace context + step-timeline recorder (§10.7).
//!
//! Set once at pipeline entry (and again at job-execution entry), read
//! automatically by `emit_event` / `plane.send` / plugin host APIs so plugin
//! authors never have to thread trace ids by hand.

use serde_json::{Value, json};

/// Ambient trace context (task-local).
#[derive(Debug, Clone)]
pub struct TraceCtx {
    /// `itg_receipts.id` — the whole-chain trace id.
    pub trace_id: i64,
    pub channel_key: String,
}

tokio::task_local! {
    static TRACE_CTX: TraceCtx;
}

/// Run `f` with a trace context installed (pipeline/job entry points).
pub async fn with_trace<R>(ctx: TraceCtx, f: impl std::future::Future<Output = R>) -> R {
    TRACE_CTX.scope(ctx, f).await
}

/// Read the ambient trace context, if any.
#[must_use]
pub fn current() -> Option<TraceCtx> {
    TRACE_CTX.try_with(|ctx| ctx.clone()).ok()
}

/// One step entry of the receipt timeline (`itg_receipts.steps`).
#[derive(Debug, Clone)]
pub struct StepEntry {
    /// Step name (`queue`/`verify`/`normalize`/`dedup`/`route`/`ack`,
    /// `job:<type>#<attempt>`, `pipeline#<n>`, `replay#<n>`, …).
    pub step: String,
    /// Terminal state: `ok` | `failed` | `pending` | `skipped`.
    pub status: &'static str,
    /// Duration in milliseconds (absent for pending placeholders).
    pub ms: Option<u64>,
    /// Human-readable detail (verifier kind, mapping engine, target, error).
    pub detail: Option<String>,
}

impl StepEntry {
    /// Pending placeholder (written at route time for planned async jobs).
    #[must_use]
    pub fn pending(step: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            status: "pending",
            ms: None,
            detail: None,
        }
    }

    /// Completed step.
    #[must_use]
    pub fn done(step: impl Into<String>, ms: u64, detail: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            status: "ok",
            ms: Some(ms),
            detail: Some(detail.into()),
        }
    }

    /// Failed step.
    #[must_use]
    pub fn failed(step: impl Into<String>, ms: u64, detail: impl Into<String>) -> Self {
        Self {
            step: step.into(),
            status: "failed",
            ms: Some(ms),
            detail: Some(detail.into()),
        }
    }

    /// JSON wire form (stored in `itg_receipts.steps`).
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut obj = json!({"step": self.step, "status": self.status});
        if let Some(ms) = self.ms {
            obj["ms"] = json!(ms);
        }
        if let Some(ref detail) = self.detail {
            obj["detail"] = json!(detail);
        }
        obj
    }
}

/// Collects step entries for one pipeline pass; serializes to the
/// `itg_receipts.steps` JSON array (first pass in-tx; async appends later
/// extend it via the M2 `append_step` API).
#[derive(Debug, Default)]
pub struct StepTimeline {
    entries: Vec<StepEntry>,
}

impl StepTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a step.
    pub fn push(&mut self, entry: StepEntry) {
        self.entries.push(entry);
    }

    /// Serialize to the stored JSON array.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Array(self.entries.iter().map(StepEntry::to_json).collect())
    }

    /// Number of recorded entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing was recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn trace_ctx_roundtrip() {
        with_trace(
            TraceCtx {
                trace_id: 42,
                channel_key: "test-ch".into(),
            },
            async {
                let ctx = current().expect("ctx inside scope");
                assert_eq!(ctx.trace_id, 42);
                assert_eq!(ctx.channel_key, "test-ch");
            },
        )
        .await;
        assert!(current().is_none());
    }

    #[test]
    fn timeline_serializes_all_fields() {
        let mut tl = StepTimeline::new();
        tl.push(StepEntry::pending("job:llm-reply"));
        tl.push(StepEntry::done("verify", 2, "hmac-sha256"));
        tl.push(StepEntry::failed("route", 8, "ct insert: duplicate"));
        let json = tl.to_json();
        let arr = json.as_array().expect("array");
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["status"], "pending");
        assert!(arr[0].get("ms").is_none());
        assert_eq!(arr[1]["ms"], 2);
        assert_eq!(arr[2]["status"], "failed");
    }
}
