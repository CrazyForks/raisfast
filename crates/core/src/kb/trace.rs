//! KB run recorder — orchestration-level side recording for the
//! observability plane (kb-observability-design §4).
//!
//! Discipline:
//! - **Zero-cost when off**: `RunRecorder::disabled()` / `TraceMode::Off`
//!   keeps `inner: None` — no allocation, no I/O.
//! - **Insert timing by kind (DR2)**: long-running kinds INSERT `running`
//!   at `begin()` and UPDATE at `finish()`; short kinds INSERT once at
//!   `finish()`. `errors` mode defers persistence to `finish()` and only
//!   writes failed/degraded runs.
//! - **Bounded summaries (DR3)**: lists capped at [`STAGE_LIST_TOP`],
//!   text snippets at [`SNIPPET_CHARS`], accumulated stages JSON at
//!   [`STAGES_JSON_CAP`] (oversized summaries are dropped and flagged).
//! - **Never fails the pipeline**: every persistence error is a
//!   `tracing::warn!`, never propagated.

use std::time::Instant;

use serde_json::{Value, json};

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::kb::models::kb_run::{self, NewKbRun};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::now_utc;

/// Hard cap for the accumulated stages JSON (DR3).
const STAGES_JSON_CAP: usize = 16 * 1024;
/// Max entries per scored list inside a stage summary (DR3).
pub const STAGE_LIST_TOP: usize = 10;
/// Max chars for text snippets inside stage summaries (DR3).
pub const SNIPPET_CHARS: usize = 120;

/// Trace persistence mode (`RAISFAST_KB_TRACE_MODE`, DR5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TraceMode {
    #[default]
    All,
    Errors,
    Off,
}

impl TraceMode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "errors" => Self::Errors,
            "off" => Self::Off,
            _ => Self::All,
        }
    }
}

/// Stage status values (kb-observability-design §3.2).
pub const STAGE_OK: &str = "ok";
pub const STAGE_DEGRADED: &str = "degraded";
pub const STAGE_FAILED: &str = "failed";
pub const STAGE_SKIPPED: &str = "skipped";

#[derive(Debug)]
struct StageState {
    name: String,
    started: Instant,
}

#[derive(Debug)]
struct RunState {
    mode: TraceMode,
    /// Long-running kinds INSERT `running` at `begin()` (DR2).
    long_running: bool,
    run_id: Option<SnowflakeId>,
    kind: &'static str,
    trigger_src: &'static str,
    tenant_id: String,
    kb_id: Option<SnowflakeId>,
    doc_id: Option<SnowflakeId>,
    agent_id: Option<SnowflakeId>,
    session_id: Option<SnowflakeId>,
    job_id: Option<SnowflakeId>,
    attempt: i64,
    config_snapshot: Option<Value>,
    stages: Vec<Value>,
    current: Option<StageState>,
    /// Any stage recorded `degraded` → run downgrades `ok` to `degraded`.
    degraded: bool,
    started: Instant,
    /// Serialized stages length already accounted for (DR3 cap).
    stages_len: usize,
}

/// Declarative spec for one run (kb-observability-design §4.2).
pub struct RunSpec {
    pub kind: &'static str,
    /// 'job' | 'public' | 'agent' | 'admin'.
    pub trigger_src: &'static str,
    pub tenant_id: String,
    pub kb_id: Option<SnowflakeId>,
    pub doc_id: Option<SnowflakeId>,
    pub agent_id: Option<SnowflakeId>,
    pub session_id: Option<SnowflakeId>,
    pub job_id: Option<SnowflakeId>,
    pub attempt: i64,
    /// Long-running kinds get a `running` row at `begin()` (DR2).
    pub long_running: bool,
}

/// Side-recorder threaded through the orchestration layer. Cheap `Option`
/// wrapper: `disabled()` is the zero-cost null object for tests/legacy
/// callers [自造+理由: DR6 零侵入旁录].
#[derive(Debug)]
pub struct RunRecorder {
    inner: Option<Box<RunState>>,
}

impl RunRecorder {
    /// Null object — no recording, no I/O (tests / `TraceMode::Off`).
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Create a recorder from a spec (no I/O yet). `Off` mode collapses to
    /// the disabled null object.
    pub fn create(mode: TraceMode, spec: RunSpec) -> Self {
        if mode == TraceMode::Off {
            return Self::disabled();
        }
        Self {
            inner: Some(Box::new(RunState {
                mode,
                long_running: spec.long_running,
                run_id: None,
                kind: spec.kind,
                trigger_src: spec.trigger_src,
                tenant_id: spec.tenant_id,
                kb_id: spec.kb_id,
                doc_id: spec.doc_id,
                agent_id: spec.agent_id,
                session_id: spec.session_id,
                job_id: spec.job_id,
                attempt: spec.attempt,
                config_snapshot: None,
                stages: Vec::new(),
                current: None,
                degraded: false,
                started: Instant::now(),
                stages_len: 0,
            })),
        }
    }

    /// Late kb binding (the orchestration fn resolves doc → kb after the
    /// caller created the recorder).
    pub fn set_kb(&mut self, kb_id: SnowflakeId) {
        if let Some(state) = self.inner.as_deref_mut() {
            state.kb_id = Some(kb_id);
        }
    }

    /// First persistence hop for long tasks (DR2): INSERT the `running`
    /// row. Errors are warned, never propagated.
    pub async fn begin(&mut self, pool: &Pool, config: &AppConfig) {
        let Some(state) = self.inner.as_deref_mut() else {
            return;
        };
        state.config_snapshot = Some(config_snapshot(config));
        if !state.long_running || state.mode == TraceMode::Errors {
            return; // short / errors mode: defer to finish()
        }
        let new_run = NewKbRun {
            kind: state.kind,
            trigger_src: state.trigger_src,
            tenant_id: state.tenant_id.clone(),
            kb_id: state.kb_id,
            doc_id: state.doc_id,
            agent_id: state.agent_id,
            session_id: state.session_id,
            job_id: state.job_id,
            attempt: state.attempt,
            status: kb_run::STATUS_RUNNING,
            config_snapshot: state.config_snapshot.clone(),
        };
        match kb_run::insert_run(pool, &new_run, None, None, None).await {
            Ok(id) => state.run_id = Some(id),
            Err(e) => tracing::warn!("[kb-trace] begin insert failed: {e}"),
        }
    }

    /// Open a stage (records start time).
    pub fn stage(&mut self, name: &str) {
        if let Some(state) = self.inner.as_deref_mut() {
            state.current = Some(StageState {
                name: name.to_string(),
                started: Instant::now(),
            });
        }
    }

    /// Close the open stage with a status, bounded summary and optional
    /// error message.
    pub fn end_stage(&mut self, status: &str, summary: Value, error: Option<&str>) {
        let Some(state) = self.inner.as_deref_mut() else {
            return;
        };
        let Some(current) = state.current.take() else {
            return;
        };
        if status == STAGE_DEGRADED {
            state.degraded = true;
        }
        let latency_ms = current.started.elapsed().as_millis() as i64;
        let mut summary = summary;
        cap_stage_summary(&mut summary, &mut state.stages_len);
        let mut record = json!({
            "stage": current.name,
            "status": status,
            "started_at": now_utc().to_rfc3339(),
            "ended_at": now_utc().to_rfc3339(),
            "latency_ms": latency_ms,
            "summary": summary,
        });
        if let Some(err) = error {
            record["error"] = json!({ "message": err });
        }
        state.stages.push(record);
    }

    /// Convenience: close the open stage as failed with the error text.
    pub fn fail_stage(&mut self, error: &str) {
        self.end_stage(STAGE_FAILED, json!({}), Some(error));
    }

    /// Whether recording (and thus response tracing) is active.
    pub fn enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Stage records as JSON (for the `?trace=true` response payload).
    /// Snapshot before [`Self::finish`] consumes the recorder state.
    pub fn stages_snapshot(&self) -> Value {
        let Some(state) = self.inner.as_deref() else {
            return Value::Null;
        };
        serde_json::to_value(&state.stages).unwrap_or(Value::Null)
    }

    /// Terminal hop: long tasks UPDATE their `running` row; short tasks
    /// INSERT once in their terminal state. `Errors` mode persists only
    /// failed/degraded runs (DR5). Takes `&mut self` so orchestration
    /// wrappers holding `&mut RunRecorder` can call it in both arms.
    /// Returns the run row id when a row was persisted.
    pub async fn finish(
        &mut self,
        pool: &Pool,
        status: &'static str,
        error: Option<&str>,
    ) -> Option<SnowflakeId> {
        let state = self.inner.as_deref_mut()?;
        let final_status: &'static str = if status == kb_run::STATUS_OK && state.degraded {
            kb_run::STATUS_DEGRADED
        } else {
            status
        };
        let failed =
            final_status == kb_run::STATUS_FAILED || final_status == kb_run::STATUS_DEGRADED;
        let latency_ms = state.started.elapsed().as_millis() as i64;
        let stages = serde_json::to_value(&state.stages).unwrap_or(Value::Null);
        match state.run_id.take() {
            Some(id) => {
                if let Err(e) = kb_run::finish_run(
                    pool,
                    id,
                    &state.tenant_id,
                    final_status,
                    Some(latency_ms),
                    error,
                    Some(&stages),
                )
                .await
                {
                    tracing::warn!("[kb-trace] finish update failed: {e}");
                }
                Some(id)
            }
            None => {
                if state.mode == TraceMode::Errors && !failed {
                    return None; // DR5: successful runs are not persisted
                }
                let new_run = NewKbRun {
                    kind: state.kind,
                    trigger_src: state.trigger_src,
                    tenant_id: state.tenant_id.clone(),
                    kb_id: state.kb_id,
                    doc_id: state.doc_id,
                    agent_id: state.agent_id,
                    session_id: state.session_id,
                    job_id: state.job_id,
                    attempt: state.attempt,
                    status: final_status,
                    config_snapshot: state.config_snapshot.clone(),
                };
                match kb_run::insert_run(pool, &new_run, Some(&stages), Some(latency_ms), error)
                    .await
                {
                    Ok(id) => Some(id),
                    Err(e) => {
                        tracing::warn!("[kb-trace] finish insert failed: {e}");
                        None
                    }
                }
            }
        }
    }
}

/// Config snapshot frozen at run start (kb-observability-design §4.3).
pub fn config_snapshot(config: &AppConfig) -> Value {
    json!({
        "top_k": config.kb.top_k,
        "wiki_boost": config.kb.wiki_boost,
        "fallback_threshold": config.kb.fallback_threshold,
        "context_budget_tokens": config.kb.context_budget_tokens,
        "vector_backend": config.kb.vector_backend,
        "trace_mode": config.kb.trace_mode,
    })
}

/// Enforce DR3: when the accumulated stages JSON approaches the cap, drop
/// the incoming summary body and flag it as truncated.
fn cap_stage_summary(summary: &mut Value, stages_len: &mut usize) {
    let serialized = serde_json::to_string(summary).unwrap_or_default();
    let incoming = serialized.len() + 2; // quotes + comma overhead
    if *stages_len + incoming > STAGES_JSON_CAP {
        *summary = json!({ "truncated": true });
    } else {
        *stages_len += incoming;
    }
}

/// Char-bounded snippet for stage summaries (DR3; mirrors the admin
/// `preview` helper — kept here so the trace module is self-contained).
pub fn snippet(text: &str) -> String {
    let truncated: String = text.chars().take(SNIPPET_CHARS).collect();
    if truncated.chars().count() < text.chars().count() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

/// Top-N scored id list for stage summaries (DR3).
pub fn top_scored(entries: &[(i64, f32)]) -> Value {
    json!(
        entries
            .iter()
            .take(STAGE_LIST_TOP)
            .map(|(id, score)| json!({ "id": id, "score": score }))
            .collect::<Vec<_>>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_recorder_is_silent() {
        let mut r = RunRecorder::disabled();
        r.stage("parse");
        r.end_stage(STAGE_OK, json!({"a": 1}), None);
        assert!(r.inner.is_none());
    }

    #[test]
    fn off_mode_collapses_to_disabled() {
        let r = RunRecorder::create(
            TraceMode::Off,
            RunSpec {
                kind: kb_run::KIND_INGEST_DOC,
                trigger_src: "admin",
                tenant_id: "default".into(),
                kb_id: None,
                doc_id: Some(SnowflakeId(1)),
                agent_id: None,
                session_id: None,
                job_id: None,
                attempt: 1,
                long_running: true,
            },
        );
        assert!(r.inner.is_none());
    }

    #[test]
    fn degraded_stage_marks_run() {
        let mut r = RunRecorder::create(
            TraceMode::All,
            RunSpec {
                kind: kb_run::KIND_ASK,
                trigger_src: "public",
                tenant_id: "default".into(),
                kb_id: None,
                doc_id: None,
                agent_id: None,
                session_id: None,
                job_id: None,
                attempt: 1,
                long_running: false,
            },
        );
        r.stage("s1");
        r.end_stage(STAGE_DEGRADED, json!({}), None);
        let state = r.inner.as_deref().expect("state");
        assert!(state.degraded);
        assert_eq!(state.stages.len(), 1);
    }

    #[test]
    fn snippet_bounds_chars() {
        let long = "x".repeat(500);
        assert_eq!(snippet(&long).chars().count(), SNIPPET_CHARS + 1); // + ellipsis
        assert_eq!(snippet("short"), "short");
    }

    #[test]
    fn top_scored_caps_at_limit() {
        let entries: Vec<(i64, f32)> = (0..30).map(|i| (i, i as f32)).collect();
        let v = top_scored(&entries);
        assert_eq!(v.as_array().map_or(0, std::vec::Vec::len), STAGE_LIST_TOP);
    }
}
