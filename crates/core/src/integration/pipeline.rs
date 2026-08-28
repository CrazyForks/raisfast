//! Inbound pipeline — Verify → Normalize → Dedup → Route → Ack
//! (integration.md §6). One transaction per message (Message-class frequency;
//! Telemetry batching arrives in P2).
//!
//! Trace: `trace_id = receipt_id` is allocated before the transaction opens;
//! the whole pass runs inside `with_trace` so downstream helpers can pick the
//! context up automatically (§10.7).

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::constants::COL_ID;
use crate::db::driver::DbDriver;
use crate::content_type::repository::{ContentRepository, SaveContext};
use crate::content_type::ContentTypeRegistry;
use crate::db::pool::DbConnection;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::{Event, EventEmitter};
use crate::integration::channel::ItgChannel;
use crate::integration::envelope::InboundEnvelope;
use crate::integration::framing;
use crate::integration::mapping::{self, MappingPlan, Normalized};
use crate::integration::receipt;
use crate::integration::trace::{StepEntry, StepTimeline, TraceCtx};
use crate::integration::verify::{InboundHttpRequest, VerifyOutcome};
use crate::integration::vault::Vault;
use crate::types::snowflake_id::SnowflakeId;
use crate::worker::JobQueue;
use crate::utils::tz::Timestamp;

/// How to answer the connector after the pipeline finishes.
#[derive(Debug, Clone)]
pub enum AckAction {
    /// Push mode: HTTP status + optional body.
    Http { status: u16, body: Option<String> },
}

/// Terminal outcome of one pipeline pass.
#[derive(Debug, Clone)]
pub struct PipelineOutcome {
    pub ack: AckAction,
    /// Receipt/trace id (allocated even for duplicates — points at the
    /// original row on the duplicate branch).
    pub receipt_id: i64,
    pub duplicate: bool,
    pub delivered: bool,
    /// `Some(attempt)` when an internal retry was scheduled — the caller
    /// enqueues the delayed `ingress.retry` job post-commit (§6.4).
    pub retry_scheduled: Option<i64>,
}

/// Result of one replay execution (§6.4 replay semantics).
#[derive(Debug, Clone)]
pub enum ReplayOutcome {
    /// Route re-executed; `dry_run=false` path. Carries the target row id.
    Upserted { target_id: Option<SnowflakeId> },
    /// Comparison report only — nothing written.
    DryRun { report: Value },
}

/// Result of one retry execution (for the `ingress.retry` job handler).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryResult {
    Delivered,
    Rescheduled,
    Dead,
    /// Receipt missing / wrong state / channel disabled — nothing done.
    Skipped,
}

/// The push pipeline. Built once, shared across requests. Rate limiting lives
/// in `routes.rs` before the pipeline is entered.
pub struct Pipeline {
    pool: crate::db::Pool,
    storage_root: String,
    repo: ContentRepository,
    registry: Arc<ContentTypeRegistry>,
    emitter: EventEmitter,
    vault: Option<Vault>,
    /// Compiled mapping plans, keyed by (channel id, updated_at) — the
    /// updated_at component invalidates on channel config changes.
    plan_cache:
        dashmap::DashMap<(i64, String), Arc<MappingPlan>>,
}

impl Pipeline {
    /// Assemble the pipeline.
    #[must_use]
    pub fn new(
        pool: crate::db::Pool,
        storage_root: String,
        registry: Arc<ContentTypeRegistry>,
        emitter: EventEmitter,
        vault: Option<Vault>,
    ) -> Self {
        Self {
            repo: ContentRepository::new(pool.clone()),
            pool,
            storage_root,
            registry,
            emitter,
            vault,
            plan_cache: dashmap::DashMap::new(),
        }
    }

    /// Archive the raw body for replay/audit (§10.2). Returns the stored path.
    /// `archive-strict` channels fail the pipeline when archiving fails;
    /// default channels degrade to `None` + warn (availability first).
    async fn archive_raw(
        &self,
        channel: &ItgChannel,
        receipt_id: i64,
        body: &[u8],
    ) -> Result<Option<String>, AppError> {
        let dir = std::path::Path::new(&self.storage_root)
            .join("integration")
            .join("raw")
            .join(channel.id.to_string());
        let path = dir.join(format!("{receipt_id}.bin"));
        let write = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(&path, body).await?;
            std::io::Result::Ok(())
        }
        .await;
        match write {
            Ok(()) => Ok(Some(path.to_string_lossy().into_owned())),
            Err(e) => {
                let msg = format!("raw archive failed: {e}");
                if channel.archive_strict() {
                    Err(AppError::Internal(anyhow::anyhow!(msg)))
                } else {
                    tracing::warn!(trace_id = receipt_id, channel = %channel.channel_key, msg);
                    Ok(None)
                }
            }
        }
    }

    fn plan_for(&self, channel: &ItgChannel) -> Result<Option<Arc<MappingPlan>>, AppError> {
        if channel.normalizer_plugin.is_some() {
            return Err(AppError::BadRequest(
                "normalizer plugins arrive in a later phase — \
                 use declarative mapping for now"
                    .into(),
            ));
        }
        let Some(mapping_def) = channel.mapping.as_ref() else {
            return Ok(None);
        };
        let key = (channel.id.0, channel.updated_at.to_rfc3339());
        if let Some(hit) = self.plan_cache.get(&key) {
            return Ok(Some(hit.clone()));
        }
        let plan = Arc::new(mapping::compile(mapping_def)?);
        self.plan_cache.insert(key, plan.clone());
        Ok(Some(plan))
    }

    /// Run the full pipeline for one push request.
    ///
    /// Never panics; every failure maps to an [`AckAction`].
    pub async fn run_push(
        &self,
        channel: &Arc<ItgChannel>,
        req: &InboundHttpRequest,
    ) -> PipelineOutcome {
        let receipt_id = crate::utils::id::new_id();
        let channel_key = channel.channel_key.clone();
        let ctx = TraceCtx {
            trace_id: receipt_id,
            channel_key: channel_key.clone(),
        };

        crate::integration::trace::with_trace(ctx, async {
            self.run_push_traced(channel, req, receipt_id).await
        })
        .await
    }

    async fn run_push_traced(
        &self,
        channel: &Arc<ItgChannel>,
        req: &InboundHttpRequest,
        receipt_id: i64,
    ) -> PipelineOutcome {
        let mut timeline = StepTimeline::new();
        let now = crate::utils::tz::now_utc();

        // ── Verify (L0) ─────────────────────────────────────────────
        let t = Instant::now();
        let verify = crate::integration::verify::verify(channel, self.vault.as_ref(), req);
        match &verify {
            VerifyOutcome::Ok => timeline.push(StepEntry::done(
                "verify",
                t.elapsed().as_millis() as u64,
                channel.verify_kind.clone(),
            )),
            VerifyOutcome::ChallengeEcho(echo) => {
                // GET challenge handshake — no envelope at all.
                return PipelineOutcome {
                    ack: AckAction::Http {
                        status: 200,
                        body: Some(echo.clone()),
                    },
                    receipt_id,
                    duplicate: false,
                    delivered: false,
                    retry_scheduled: None,
                };
            }
            VerifyOutcome::Reject { status, reason } => {
                timeline.push(StepEntry::failed(
                    "verify",
                    t.elapsed().as_millis() as u64,
                    reason.clone(),
                ));
                tracing::warn!(trace_id = receipt_id, channel = %channel.channel_key, %reason, "ingress verify rejected");
                return PipelineOutcome {
                    ack: AckAction::Http {
                        status: *status,
                        body: None,
                    },
                    receipt_id,
                    duplicate: false,
                    delivered: false,
                    retry_scheduled: None,
                };
            }
        }

        // ── Normalize (L2: framing + mapping; self-timed) ──────────
        let normalized = self.normalize(channel, req, &mut timeline);
        let Some(normalized) = normalized else {
            // Timeline already carries the failing step (or skip note).
            return PipelineOutcome {
                ack: AckAction::Http { status: 400, body: None },
                receipt_id,
                duplicate: false,
                delivered: false,
                retry_scheduled: None,
            };
        };

        // ── Raw archive (pre-tx; file IO stays outside the write lock) ──
        let raw_ref = match self.archive_raw(channel, receipt_id, &req.body).await {
            Ok(path) => path,
            Err(archive_err) => {
                timeline.push(StepEntry::failed("archive", 0, archive_err.to_string()));
                return PipelineOutcome {
                    ack: AckAction::Http { status: 500, body: None },
                    receipt_id,
                    duplicate: false,
                    delivered: false,
                    retry_scheduled: None,
                };
            }
        };

        // ── Envelope snapshot (deterministic retry/replay source, §6.4) ─
        let envelope = InboundEnvelope {
            receipt_id: SnowflakeId::new(receipt_id),
            channel_id: channel.id,
            provider: channel.provider.clone(),
            external_id: normalized.external_id.clone(),
            sender: normalized.sender.clone(),
            recipient: None,
            kind: normalized.kind,
            payload: normalized.payload.clone(),
            raw_ref,
            connection: None,
            ingested_at: now,
            received_at: now,
        };
        let envelope_json = serde_json::to_value(&envelope).unwrap_or(Value::Null);

        // ── Dedup + Route + Ack in ONE transaction (§6.3) ───────────
        let hash = receipt::payload_hash(&req.body);
        let tx_result = self
            .dedup_route_in_tx(
                channel,
                &envelope,
                &envelope_json,
                &normalized.external_id,
                envelope.kind.as_str(),
                &hash,
                &mut timeline,
                receipt_id,
                now,
            )
            .await;

        match tx_result {
            Ok(mut outcome) => {
                if !outcome.duplicate && outcome.delivered {
                    self.post_route(channel, &envelope).await;
                }
                if let Some(attempt) = outcome.retry_scheduled.take() {
                    self.schedule_retry_job(receipt_id, attempt).await;
                }
                outcome
            }
            Err(err) => {
                // Transaction-level failure (begin/commit/SQL): fail closed.
                tracing::error!(trace_id = receipt_id, error = %err, "ingress pipeline tx failed");
                PipelineOutcome {
                    ack: AckAction::Http { status: 500, body: None },
                    receipt_id,
                    duplicate: false,
                    delivered: false,
                    retry_scheduled: None,
                }
            }
        }
    }

    fn normalize(
        &self,
        channel: &ItgChannel,
        req: &InboundHttpRequest,
        timeline: &mut StepTimeline,
    ) -> Option<Normalized> {
        let t = Instant::now();
        let detail = match self.normalize_inner(channel, req) {
            Ok(Some(n)) => {
                timeline.push(StepEntry::done(
                    "normalize",
                    t.elapsed().as_millis() as u64,
                    "framing+mapping".to_string(),
                ));
                return Some(n);
            }
            Ok(None) => {
                // `when` did not match — envelope skipped by design, ack 200.
                timeline.push(StepEntry::done(
                    "normalize",
                    t.elapsed().as_millis() as u64,
                    "when-not-matched:skipped".to_string(),
                ));
                return None;
            }
            Err(err) => err.to_string(),
        };
        timeline.push(StepEntry::failed(
            "normalize",
            t.elapsed().as_millis() as u64,
            detail,
        ));
        None
    }

    fn normalize_inner(
        &self,
        channel: &ItgChannel,
        req: &InboundHttpRequest,
    ) -> Result<Option<Normalized>, AppError> {
        let input = framing::decode(&channel.framing, &channel.codec, &req.body)?;
        let Some(plan) = self.plan_for(channel)? else {
            return Err(AppError::BadRequest(
                "channel has no mapping and no plugin — configure `mapping`".into(),
            ));
        };
        plan.apply(&input)
    }

    /// Route inside the caller transaction: CT write via `tx_insert`.
    async fn route_tx(
        &self,
        tx: &mut DbConnection,
        channel: &ItgChannel,
        envelope: &InboundEnvelope,
        timeline: &mut StepTimeline,
    ) -> Result<Option<SnowflakeId>, AppError> {
        let ct = self
            .registry
            .get(&channel.target_type)
            .or_else(|| self.registry.get(&channel.target_type.replace('_', "-")))
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "target content type '{}' not found (registry key)",
                    channel.target_type
                ))
            })?;

        let mut data = envelope.payload.clone();
        if let Value::Object(obj) = &mut data {
            // Replay-upsert association (§6.4): default the CT's external link.
            if ct.get_field("external_id").is_some() && !obj.contains_key("external_id") {
                obj.insert("external_id".into(), Value::String(envelope.external_id.clone()));
            }
        }

        let save_ctx = SaveContext {
            user_id: None,
            user_int_id: None,
            user_role: None,
            tenant_id: Some(channel.tenant_id.clone()),
        };

        let created = self
            .repo
            .tx_insert(tx, &ct, data, Some(&channel.tenant_id), &save_ctx)
            .await?;

        // Pending placeholders for planned async jobs (completeness rule §10.7).
        if let Some(jobs) = channel
            .route_extra
            .as_ref()
            .and_then(|r| r.get("jobs"))
            .and_then(Value::as_array)
        {
            for job in jobs {
                if let Some(job_type) = job.get("job_type").and_then(Value::as_str) {
                    timeline.push(StepEntry::pending(format!("job:{job_type}")));
                }
            }
        }

        Ok(created
            .get("id")
            .and_then(Value::as_i64)
            .map(SnowflakeId::new))
    }



    /// Transaction wrapper for the dedup+route pass (the macro's early
    /// `return Err` requires an `AppResult`-returning function).
    #[allow(clippy::too_many_arguments)]
    async fn dedup_route_in_tx(
        &self,
        channel: &ItgChannel,
        envelope: &InboundEnvelope,
        envelope_json: &Value,
        external_id: &str,
        kind: &str,
        hash: &str,
        timeline: &mut crate::integration::trace::StepTimeline,
        receipt_id: i64,
        now: Timestamp,
    ) -> Result<PipelineOutcome, AppError> {
        crate::in_transaction!(&self.pool, tx, {
            self.dedup_route_tx(
                &mut tx, channel, envelope, envelope_json, external_id, kind, hash,
                timeline, receipt_id, now,
            )
            .await
        })
    }

    /// Dedup + Route inside one transaction (§6.3). Returns the terminal
    /// outcome; the macro commits on Ok and rolls back on Err.
    #[allow(clippy::too_many_arguments)]
    async fn dedup_route_tx(
        &self,
        tx: &mut DbConnection,
        channel: &ItgChannel,
        envelope: &InboundEnvelope,
        envelope_json: &Value,
        external_id: &str,
        kind: &str,
        hash: &str,
        timeline: &mut crate::integration::trace::StepTimeline,
        receipt_id: i64,
        now: Timestamp,
    ) -> Result<PipelineOutcome, AppError> {
        let inserted = receipt::insert_ignore_tx(
            tx,
            SnowflakeId::new(receipt_id),
            channel.id,
            external_id,
            kind,
            hash,
            now,
        )
        .await?;

        timeline.push(StepEntry::done(
            "dedup",
            0,
            if inserted.is_some() { "first" } else { "duplicate" },
        ));

        if inserted.is_none() {
            // Duplicate branch (§6.3/§6.4). Fingerprint mismatch → warn + ack.
            // Same fingerprint + previously failed + `external` retry-mode →
            // this re-delivery IS the provider's retry: re-route from snapshot.
            let Some(state) = receipt::find_state_tx(tx, channel.id, external_id).await? else {
                return Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 200, body: None },
                    receipt_id,
                    duplicate: true,
                    delivered: true,
                    retry_scheduled: None,
                });
            };
            let same_hash = state.payload_hash == hash;
            if !same_hash {
                tracing::warn!(
                    trace_id = receipt_id,
                    channel = %channel.channel_key,
                    external_id,
                    "duplicate external_id with DIFFERENT payload hash — provider bug?"
                );
                return Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 200, body: None },
                    receipt_id: state.id.0,
                    duplicate: true,
                    delivered: true,
                    retry_scheduled: None,
                });
            }
            let undelivered = matches!(
                state.status.as_str(),
                receipt::STATUS_RECEIVED | receipt::STATUS_RETRYING
            );
            if channel.is_external_retry() && undelivered {
                return self
                    .redeliver_route_tx(tx, channel, state, timeline, receipt_id, now)
                    .await;
            }
            return Ok(PipelineOutcome {
                ack: AckAction::Http { status: 200, body: None },
                receipt_id: state.id.0,
                duplicate: true,
                delivered: state.status == receipt::STATUS_DELIVERED,
                retry_scheduled: None,
            });
        }

        let t = Instant::now();
        match self.route_tx(tx, channel, envelope, timeline).await {
            Ok(target_id) => {
                timeline.push(StepEntry::done(
                    "route",
                    t.elapsed().as_millis() as u64,
                    format!(
                        "ct={},target={}",
                        channel.target_type,
                        target_id.map(|i| i.to_string()).unwrap_or_else(|| "-".into())
                    ),
                ));
                timeline.push(StepEntry::done("ack", 0, channel.ack_kind.clone()));
                receipt::mark_delivered_tx(
                    tx,
                    SnowflakeId::new(receipt_id),
                    envelope_json,
                    &timeline.to_json(),
                    target_id,
                    now,
                )
                .await?;
                Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 200, body: None },
                    receipt_id,
                    duplicate: false,
                    delivered: true,
                    retry_scheduled: None,
                })
            }
            Err(route_err) => {
                timeline.push(StepEntry::failed(
                    "route",
                    t.elapsed().as_millis() as u64,
                    route_err.to_string(),
                ));
                if channel.is_external_retry() {
                    // External mode: provider does the retrying — fail the ack
                    // so it re-delivers; status stays `received` with snapshot.
                    timeline.push(StepEntry::done("ack", 0, "http-500(external-redelivery)"));
                    receipt::mark_failed_tx(
                        tx,
                        SnowflakeId::new(receipt_id),
                        envelope_json,
                        &timeline.to_json(),
                    )
                    .await?;
                    return Ok(PipelineOutcome {
                        ack: AckAction::Http { status: 500, body: None },
                        receipt_id,
                        duplicate: false,
                        delivered: false,
                        retry_scheduled: None,
                    });
                }
                // Internal mode: ack 200, self-scheduled backoff retry (§6.4).
                let attempts = 1;
                let next_at = now
                    + chrono::TimeDelta::try_seconds(receipt::backoff_secs(attempts))
                        .unwrap_or_default();
                timeline.push(StepEntry::done(
                    "ack",
                    0,
                    format!("http-200(internal-retry#{attempts})"),
                ));
                receipt::mark_retrying_tx(
                    tx,
                    SnowflakeId::new(receipt_id),
                    attempts,
                    next_at,
                    envelope_json,
                    &timeline.to_json(),
                )
                .await?;
                Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 200, body: None },
                    receipt_id,
                    duplicate: false,
                    delivered: false,
                    retry_scheduled: Some(attempts),
                })
            }
        }
    }

    /// Provider re-delivery (external retry-mode) re-routes from the stored
    /// envelope snapshot inside the SAME transaction as the dedup check.
    async fn redeliver_route_tx(
        &self,
        tx: &mut DbConnection,
        channel: &ItgChannel,
        state: receipt::ReceiptState,
        timeline: &mut crate::integration::trace::StepTimeline,
        _receipt_id: i64,
        now: Timestamp,
    ) -> Result<PipelineOutcome, AppError> {
        let envelope: InboundEnvelope = state
            .envelope
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "external redelivery without envelope snapshot (trace {})",
                    state.id.0
                ))
            })?;

        let attempt = state.attempts + 1;
        let t = Instant::now();
        match self.route_tx(tx, channel, &envelope, timeline).await {
            Ok(target_id) => {
                timeline.push(StepEntry::done(
                    "route",
                    t.elapsed().as_millis() as u64,
                    format!(
                        "external-redelivery#{attempt},target={}",
                        target_id.map(|i| i.to_string()).unwrap_or_else(|| "-".into())
                    ),
                ));
                receipt::mark_delivered_tx(
                    tx,
                    state.id,
                    &Value::Null,
                    &timeline.to_json(),
                    target_id,
                    now,
                )
                .await?;
                Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 200, body: None },
                    receipt_id: state.id.0,
                    duplicate: true,
                    delivered: true,
                    retry_scheduled: None,
                })
            }
            Err(route_err) => {
                timeline.push(StepEntry::failed(
                    "route",
                    t.elapsed().as_millis() as u64,
                    format!("external-redelivery#{attempt}: {route_err}"),
                ));
                if attempt > channel.redelivery_max {
                    timeline.push(StepEntry::failed(
                        "ack",
                        0,
                        format!("exhausted after {attempt} attempts"),
                    ));
                    receipt::mark_dead_tx(tx, state.id, &timeline.to_json()).await?;
                    self.emit_dead_letter(channel, state.id.0, &route_err.to_string());
                    return Ok(PipelineOutcome {
                        ack: AckAction::Http { status: 500, body: None },
                        receipt_id: state.id.0,
                        duplicate: true,
                        delivered: false,
                        retry_scheduled: None,
                    });
                }
                receipt::mark_retrying_tx(
                    tx,
                    state.id,
                    attempt,
                    now,
                    &state.envelope.unwrap_or(Value::Null),
                    &timeline.to_json(),
                )
                .await?;
                Ok(PipelineOutcome {
                    ack: AckAction::Http { status: 500, body: None },
                    receipt_id: state.id.0,
                    duplicate: true,
                    delivered: false,
                    retry_scheduled: None,
                })
            }
        }
    }

    /// One internal retry execution (`ingress.retry` job body, §6.4).
    ///
    /// Loads receipt + channel, guards the state machine, re-runs ONLY the
    /// route from the envelope snapshot (normalize is never repeated).
    pub async fn run_retry(&self, trace_id: i64) -> AppResult<RetryResult> {
        let Some(row) = receipt::find_by_id(&self.pool, SnowflakeId::new(trace_id)).await? else {
            tracing::warn!(trace_id, "ingress.retry: receipt missing");
            return Ok(RetryResult::Skipped);
        };
        if row.status != receipt::STATUS_RETRYING {
            // Already delivered by another path / dead — nothing to do.
            return Ok(RetryResult::Skipped);
        }
        let channel = crate::integration::channel::model::find_by_id(&self.pool, row.channel_id).await?;
        if !channel.enabled || channel.shadow {
            // Channel disabled mid-retry: record + defer one more cycle (§6.4).
            let attempt = row.attempts;
            let _ = receipt::append_step(
                &self.pool,
                SnowflakeId::new(trace_id),
                &serde_json::json!({"step": format!("retry#{attempt}"), "status": "skipped", "detail": "channel-disabled"}),
            )
            .await;
            self.schedule_retry_job(trace_id, attempt).await;
            return Ok(RetryResult::Skipped);
        }
        let attempt = row.attempts;
        let envelope: InboundEnvelope = row
            .envelope
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "retry without envelope snapshot (trace {trace_id})"
                ))
            })?;

        let now = crate::utils::tz::now_utc();
        let ctx = crate::integration::trace::TraceCtx {
            trace_id,
            channel_key: channel.channel_key.clone(),
        };
        crate::integration::trace::with_trace(ctx, async {
            let result = self.retry_attempt(&channel, &row, &envelope, attempt, now).await?;
            if result == RetryResult::Delivered {
                self.post_route(&channel, &envelope).await;
            }
            Ok(result)
        })
        .await
    }

    async fn retry_attempt(
        &self,
        channel: &ItgChannel,
        row: &receipt::ReceiptRow,
        envelope: &InboundEnvelope,
        attempt: i64,
        now: Timestamp,
    ) -> AppResult<RetryResult> {
        let next_attempt = attempt + 1;
        crate::in_transaction!(&self.pool, tx, {
            let mut timeline = crate::integration::trace::StepTimeline::new();
            let t = Instant::now();
            match self.route_tx(&mut tx, channel, envelope, &mut timeline).await {
                Ok(target_id) => {
                    timeline.push(StepEntry::done(
                        "route",
                        t.elapsed().as_millis() as u64,
                        format!(
                            "internal-retry#{attempt},target={}",
                            target_id.map(|i| i.to_string()).unwrap_or_else(|| "-".into())
                        ),
                    ));
                    let merged = merge_steps(row.steps.as_ref(), &timeline, true);
                    receipt::mark_delivered_tx(
                        &mut tx,
                        row.id,
                        &row.envelope.clone().unwrap_or(Value::Null),
                        &merged,
                        target_id,
                        now,
                    )
                    .await?;
                    Ok(RetryResult::Delivered)
                }
                Err(route_err) => {
                    timeline.push(StepEntry::failed(
                        "route",
                        t.elapsed().as_millis() as u64,
                        format!("internal-retry#{attempt}: {route_err}"),
                    ));
                    let merged = merge_steps(row.steps.as_ref(), &timeline, false);
                    if next_attempt > channel.redelivery_max {
                        receipt::mark_dead_tx(&mut tx, row.id, &merged).await?;
                        self.emit_dead_letter(channel, row.id.0, &route_err.to_string());
                        Ok(RetryResult::Dead)
                    } else {
                        let next_at = now
                            + chrono::TimeDelta::try_seconds(receipt::backoff_secs(next_attempt))
                                .unwrap_or_default();
                        receipt::mark_retrying_tx(
                            &mut tx,
                            row.id,
                            next_attempt,
                            next_at,
                            &row.envelope.clone().unwrap_or(Value::Null),
                            &merged,
                        )
                        .await?;
                        Ok(RetryResult::Rescheduled)
                    }
                }
            }
        })
    }

    /// Enqueue the delayed `ingress.retry` job (post-commit).
    async fn schedule_retry_job(&self, trace_id: i64, attempt: i64) {
        let queue = crate::worker::DefaultJobQueue::new(self.pool.clone());
        let run_after = crate::utils::tz::now_utc()
            + chrono::TimeDelta::try_seconds(receipt::backoff_secs(attempt))
                .unwrap_or_default();
        let new_job = crate::worker::NewJob {
            job: crate::worker::Job::Custom {
                job_type: "ingress.retry".into(),
                payload: serde_json::json!({
                    "trace_id": trace_id,
                    "attempt": attempt,
                }),
            },
            max_attempts: None,
            run_after: Some(run_after),
            cron_schedule_id: None,
            cron_log_id: None,
            priority: 5,
            timeout_secs: Some(60),
            dedup_key: Some(format!("ingress.retry:{trace_id}")),
        };
        if let Err(err) = queue.enqueue(new_job).await {
            tracing::error!(trace_id, attempt, error = %err, "enqueue ingress.retry failed");
        }
    }

    /// Public wrapper so the `ingress.retry` job handler can reschedule.
    pub async fn schedule_retry_public(&self, trace_id: i64, attempt: i64) {
        self.schedule_retry_job(trace_id, attempt).await;
    }

    /// Dead-letter alert event (§6.4 / §10.2).
    fn emit_dead_letter(&self, channel: &ItgChannel, trace_id: i64, reason: &str) {
        self.emitter.emit(Event::Custom {
            source: "integration".into(),
            event_type: "integration.dead_letter".into(),
            data: json!({
                "trace_id": trace_id,
                "channel": channel.channel_key,
                "reason": reason,
            }),
        });
    }

    /// Post-commit side effects: EventBus broadcast + job enqueue (§6.4).
    async fn post_route(&self, channel: &ItgChannel, envelope: &InboundEnvelope) {
        let broadcast = channel
            .route_extra
            .as_ref()
            .and_then(|r| r.get("broadcast"))
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if broadcast {
            self.emitter.emit(Event::Custom {
                source: "integration".into(),
                event_type: "ingress.received".into(),
                data: json!({
                    "trace_id": envelope.receipt_id.0,
                    "channel": channel.channel_key,
                    "kind": envelope.kind.as_str(),
                    "external_id": envelope.external_id,
                    "payload": envelope.payload,
                }),
            });
        }

        if let Some(jobs) = channel
            .route_extra
            .as_ref()
            .and_then(|r| r.get("jobs"))
            .and_then(Value::as_array)
        {
            let queue = crate::worker::DefaultJobQueue::new(self.pool.clone());
            for job in jobs {
                let Some(job_type) = job.get("job_type").and_then(Value::as_str) else {
                    continue;
                };
                let mut payload = job
                    .get("payload")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if let Value::Object(obj) = &mut payload {
                    obj.insert("trace_id".into(), json!(envelope.receipt_id.0));
                    obj.insert(
                        "channel_key".into(),
                        Value::String(channel.channel_key.clone()),
                    );
                }
                let new_job = crate::worker::NewJob {
                    job: crate::worker::Job::Custom {
                        job_type: job_type.to_string(),
                        payload,
                    },
                    max_attempts: None,
                    run_after: None,
                    cron_schedule_id: None,
                    cron_log_id: None,
                    priority: 0,
                    timeout_secs: None,
                    dedup_key: None,
                };
                if let Err(err) = queue.enqueue(new_job).await {
                    tracing::warn!(
                        trace_id = envelope.receipt_id.0,
                        job_type,
                        error = %err,
                        "route_extra job enqueue failed"
                    );
                }
            }
        }
    }
}

impl Pipeline {
    /// Replay one receipt from its envelope snapshot (§6.4): re-runs ONLY the
    /// route — verification and normalization are never repeated. `upsert`
    /// writes through the target CT's `external_id` association; `dry_run`
    /// returns a comparison report without touching data. The original
    /// first-pass timeline is never overwritten (appended `replay#n`).
    ///
    /// # Errors
    ///
    /// Returns `AppError` when the receipt/channel/CT is missing or the route
    /// fails (`upsert` mode — the receipt keeps its current status then).
    pub async fn run_replay(&self, receipt_id: SnowflakeId, dry_run: bool) -> AppResult<ReplayOutcome> {
        let row = receipt::find_by_id(&self.pool, receipt_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("receipt {receipt_id} not found")))?;
        let channel = crate::integration::channel::model::find_by_id(&self.pool, row.channel_id)
            .await
            .map_err(|_| AppError::NotFound(format!("channel {} gone", row.channel_id)))?;
        let envelope: InboundEnvelope = row
            .envelope
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "replay without envelope snapshot (trace {})",
                    row.id.0
                ))
            })?;

        // Replay numbering: count existing replay passes in the timeline.
        let replay_n = row
            .steps
            .as_ref()
            .and_then(|v| v.as_array().map(|a| a.len()))
            .unwrap_or(0)
            as i64
            + 1;

        if dry_run {
            let existing = self.find_target_by_external(&channel, &envelope.external_id).await?;
            let report = json!({
                "trace_id": row.id.0,
                "channel": channel.channel_key,
                "target_type": channel.target_type,
                "external_id": envelope.external_id,
                "existing_target_id": existing,
                "would_write": envelope.payload,
            });
            return Ok(ReplayOutcome::DryRun { report });
        }

        let now = crate::utils::tz::now_utc();
        let outcome = self.replay_upsert(&channel, &row, &envelope, now).await?;
        receipt::append_step(
            &self.pool,
            row.id,
            &json!({
                "step": format!("replay#{replay_n}"),
                "status": "ok",
                "detail": format!(
                    "upsert,target={}",
                    match &outcome {
                        ReplayOutcome::Upserted { target_id } =>
                            target_id.map(|i| i.to_string()).unwrap_or_else(|| "-".into()),
                        _ => "-".into(),
                    }
                ),
            }),
        )
        .await
        .ok();
        Ok(outcome)
    }

    /// Find an existing target row by the replay association (`external_id`).
    async fn find_target_by_external(
        &self,
        channel: &ItgChannel,
        external_id: &str,
    ) -> AppResult<Option<i64>> {
        let Some(ct) = self.resolve_target_ct(channel)? else {
            return Ok(None);
        };
        if ct.get_field("external_id").is_none() {
            return Err(AppError::BadRequest(format!(
                "target CT '{}' has no external_id field — replay-mode requires it",
                channel.target_type
            )));
        }
        let sql = format!(
            "SELECT {COL_ID} FROM {} WHERE external_id = {}",
            ct.table,
            crate::db::Driver::ph(1)
        );
        let id: Option<i64> =
            sqlx::query_scalar(crate::db::safe_sql(&sql))
                .bind(external_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(id)
    }

    fn resolve_target_ct(
        &self,
        channel: &ItgChannel,
    ) -> AppResult<Option<Arc<crate::content_type::schema::ContentTypeSchema>>> {
        Ok(self
            .registry
            .get(&channel.target_type)
            .or_else(|| self.registry.get(&channel.target_type.replace('_', "-"))))
    }

    /// Upsert replay: update the existing target row, or insert when none.
    async fn replay_upsert(
        &self,
        channel: &ItgChannel,
        row: &receipt::ReceiptRow,
        envelope: &InboundEnvelope,
        now: Timestamp,
    ) -> AppResult<ReplayOutcome> {
        let existing = self.find_target_by_external(channel, &envelope.external_id).await?;
        let ct = self
            .resolve_target_ct(channel)?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "target content type '{}' not found",
                    channel.target_type
                ))
            })?;

        let mut data = envelope.payload.clone();
        if let Value::Object(obj) = &mut data
            && ct.get_field("external_id").is_some()
            && !obj.contains_key("external_id")
        {
            obj.insert(
                "external_id".into(),
                Value::String(envelope.external_id.clone()),
            );
        }

        let save_ctx = SaveContext {
            user_id: None,
            user_int_id: None,
            user_role: None,
            tenant_id: Some(channel.tenant_id.clone()),
        };

        match existing {
            Some(target_id) => {
                let id = SnowflakeId::new(target_id);
                self.repo
                    .update(&ct, id, data, Some(&channel.tenant_id), &save_ctx)
                    .await?;
                if let Err(err) = self.finalize_replay_delivered(row, Some(id), now).await {
                    tracing::warn!(trace_id = row.id.0, error = %err, "replay status flip failed");
                }
                Ok(ReplayOutcome::Upserted { target_id: Some(id) })
            }
            None => {
                let target_id = crate::in_transaction!(&self.pool, tx, {
                    let created = self
                        .repo
                        .tx_insert(&mut tx, &ct, data, Some(&channel.tenant_id), &save_ctx)
                        .await?;
                    let target_id = created
                        .get("id")
                        .and_then(Value::as_i64)
                        .map(SnowflakeId::new);
                    receipt::mark_delivered_tx(
                        &mut tx,
                        row.id,
                        &row.envelope.clone().unwrap_or(Value::Null),
                        &row.steps.clone().unwrap_or(Value::Array(Vec::new())),
                        target_id,
                        now,
                    )
                    .await?;
                    Ok::<Option<SnowflakeId>, AppError>(target_id)
                })?;
                Ok(ReplayOutcome::Upserted { target_id })
            }
        }
    }

    /// Status flip for replays that hit an existing row (separate small tx).
    async fn finalize_replay_delivered(
        &self,
        row: &receipt::ReceiptRow,
        target_id: Option<SnowflakeId>,
        now: Timestamp,
    ) -> AppResult<()> {
        crate::in_transaction!(&self.pool, tx, {
            receipt::mark_delivered_tx(
                &mut tx,
                row.id,
                &row.envelope.clone().unwrap_or(Value::Null),
                &row.steps.clone().unwrap_or(Value::Array(Vec::new())),
                target_id,
                now,
            )
            .await?;
            Ok(())
        })
    }
}

/// Append a retry/redelivery pass summary onto the stored first-pass timeline.
fn merge_steps(stored: Option<&Value>, pass: &StepTimeline, ok: bool) -> Value {
    let mut arr = stored
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    arr.push(serde_json::json!({
        "step": "pipeline-pass",
        "status": if ok { "ok" } else { "failed" },
        "entries": pass.to_json(),
    }));
    Value::Array(arr)
}
