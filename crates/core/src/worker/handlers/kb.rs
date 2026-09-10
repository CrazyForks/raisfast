//! KB worker handlers — thin bridges delegating to `kb::service`
//! [抄RF:worker/handlers/agent_run.rs 桥接先例].

use std::sync::Arc;

use crate::errors::app_error::AppResult;
use crate::kb::KbRuntime;
use crate::storage::Storage;
use crate::worker::{Job, JobHandler};

pub struct KbProcessDocumentHandler {
    pool: crate::db::Pool,
    runtime: Arc<KbRuntime>,
    storage: Arc<dyn Storage>,
    config: Arc<crate::config::app::AppConfig>,
    emitter: crate::event::EventEmitter,
}

impl KbProcessDocumentHandler {
    pub fn new(
        pool: crate::db::Pool,
        runtime: Arc<KbRuntime>,
        storage: Arc<dyn Storage>,
        config: Arc<crate::config::app::AppConfig>,
        emitter: crate::event::EventEmitter,
    ) -> Self {
        Self {
            pool,
            runtime,
            storage,
            config,
            emitter,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for KbProcessDocumentHandler {
    fn coalesce_key(&self, job: &Job) -> Option<String> {
        match job {
            Job::KbProcessDocument { doc_id, .. } => Some(format!("kb_process_doc_{doc_id}")),
            _ => None,
        }
    }

    /// Coalesce key is per-doc: a group is duplicate re-process requests for
    /// the SAME doc — one idempotent run suffices
    /// [抄RF:worker/handlers/search_index.rs coalesce 配对实现].
    fn coalesce(&self, jobs: Vec<Job>) -> Option<Job> {
        jobs.into_iter()
            .find(|j| matches!(j, Job::KbProcessDocument { .. }))
    }

    async fn handle(&self, job: &Job) -> AppResult<()> {
        // Legacy entry (tests / direct dispatch): no job provenance.
        self.run(job, None, 1).await
    }

    /// Full-context dispatch [kb-observability-design DR6]: the runner goes
    /// through here so the run row carries the exact `jobs.attempts` and
    /// `jobs.id` (precise retry provenance + run↔job linkage).
    async fn handle_queued(&self, queued: &crate::worker::QueuedJob) -> AppResult<()> {
        let job_id = queued
            .id
            .parse::<i64>()
            .ok()
            .map(crate::types::snowflake_id::SnowflakeId);
        self.run(&queued.job, job_id, i64::from(queued.attempts))
            .await
    }
}

impl KbProcessDocumentHandler {
    /// Shared body: build the traced deps + recorder, delegate to the
    /// pipeline, keep the legacy failure bookkeeping (doc status + event).
    async fn run(
        &self,
        job: &Job,
        job_id: Option<crate::types::snowflake_id::SnowflakeId>,
        attempt: i64,
    ) -> AppResult<()> {
        let Job::KbProcessDocument { doc_id, tenant_id } = job else {
            return Ok(());
        };
        let deps = crate::kb::service::KbDeps {
            pool: self.pool.clone(),
            config: self.config.clone(),
            storage: self.storage.clone(),
            vector: self.runtime.vector.clone(),
            kbsearch: self.runtime.kbsearch.clone(),
            embedder: self.runtime.embedder.clone(),
            provider: Some(self.runtime.provider.clone()),
            emitter: self.emitter.clone(),
        };
        let mode = crate::kb::trace::TraceMode::parse(&self.config.kb.trace_mode);
        let mut trace = crate::kb::trace::RunRecorder::create(
            mode,
            crate::kb::trace::RunSpec {
                kind: crate::kb::models::kb_run::KIND_INGEST_DOC,
                trigger_src: "job",
                tenant_id: tenant_id.clone(),
                kb_id: None, // bound inside the pipeline (doc → kb lookup)
                doc_id: Some(*doc_id),
                agent_id: None,
                session_id: None,
                job_id,
                attempt,
                long_running: true,
            },
        );
        match crate::kb::service::process_document_traced(&deps, *doc_id, tenant_id, &mut trace)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) => {
                // Record failure on the row and emit; then surface for retry.
                let _ = crate::kb::models::document::set_document_status(
                    &self.pool,
                    *doc_id,
                    "failed",
                    Some(&e.to_string()),
                    None,
                    tenant_id,
                )
                .await;
                if let Some(doc) =
                    crate::kb::models::document::find_document_by_id(&self.pool, *doc_id, tenant_id)
                        .await?
                {
                    self.emitter
                        .emit(crate::event::Event::KbDocumentFailed(doc));
                }
                Err(e)
            }
        }
    }
}

pub struct KbDistillWikiHandler {
    pool: crate::db::Pool,
    runtime: Arc<KbRuntime>,
    storage: Arc<dyn Storage>,
    config: Arc<crate::config::app::AppConfig>,
    emitter: crate::event::EventEmitter,
}

impl KbDistillWikiHandler {
    pub fn new(
        pool: crate::db::Pool,
        runtime: Arc<KbRuntime>,
        storage: Arc<dyn Storage>,
        config: Arc<crate::config::app::AppConfig>,
        emitter: crate::event::EventEmitter,
    ) -> Self {
        Self {
            pool,
            runtime,
            storage,
            config,
            emitter,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for KbDistillWikiHandler {
    fn coalesce_key(&self, job: &Job) -> Option<String> {
        match job {
            Job::KbDistillWiki { kb_id, .. } => Some(format!("kb_distill_{kb_id}")),
            _ => None,
        }
    }

    /// Merge same-KB distill requests: union + dedup doc_ids
    /// [抄RF:worker/handlers/search_index.rs coalesce 合并模式].
    fn coalesce(&self, jobs: Vec<Job>) -> Option<Job> {
        let mut doc_ids: Vec<i64> = Vec::new();
        let mut first: Option<Job> = None;
        for job in jobs {
            if let Job::KbDistillWiki {
                kb_id,
                doc_ids: ids,
                tenant_id,
            } = job
            {
                if first.is_none() {
                    first = Some(Job::KbDistillWiki {
                        kb_id,
                        doc_ids: Vec::new(),
                        tenant_id,
                    });
                }
                doc_ids.extend(ids);
            }
        }
        let mut job = first?;
        if let Job::KbDistillWiki { doc_ids: ids, .. } = &mut job {
            doc_ids.sort_unstable();
            doc_ids.dedup();
            *ids = doc_ids;
        }
        Some(job)
    }

    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::KbDistillWiki {
            kb_id,
            doc_ids,
            tenant_id,
        } = job
        else {
            return Ok(());
        };
        let deps = crate::kb::service::KbDeps {
            pool: self.pool.clone(),
            config: self.config.clone(),
            storage: self.storage.clone(),
            vector: self.runtime.vector.clone(),
            kbsearch: self.runtime.kbsearch.clone(),
            embedder: self.runtime.embedder.clone(),
            provider: Some(self.runtime.provider.clone()),
            emitter: self.emitter.clone(),
        };
        let doc_ids: Vec<crate::types::snowflake_id::SnowflakeId> =
            doc_ids.iter().map(|i| (*i).into()).collect();
        let created =
            crate::kb::distill::distill_documents(&deps, *kb_id, &doc_ids, tenant_id).await?;
        tracing::info!(
            "[kb] distilled kb {kb_id}: {} draft page(s) from {} doc(s)",
            created.len(),
            doc_ids.len()
        );
        Ok(())
    }
}

pub struct KbRebuildVectorIndexHandler {
    pool: crate::db::Pool,
    runtime: Arc<KbRuntime>,
    config: Arc<crate::config::app::AppConfig>,
}

impl KbRebuildVectorIndexHandler {
    pub fn new(
        pool: crate::db::Pool,
        runtime: Arc<KbRuntime>,
        config: Arc<crate::config::app::AppConfig>,
    ) -> Self {
        Self {
            pool,
            runtime,
            config,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for KbRebuildVectorIndexHandler {
    /// Per-KB coalesce [抄RF:KbProcessDocument 同键模式 — DR10 注：启动
    /// 批量 + 手动 rebuild 并发时去重].
    fn coalesce_key(&self, job: &Job) -> Option<String> {
        match job {
            Job::KbRebuildVectorIndex { kb_id, .. } => Some(format!("kb_rebuild_{kb_id}")),
            _ => None,
        }
    }

    fn coalesce(&self, jobs: Vec<Job>) -> Option<Job> {
        jobs.into_iter()
            .find(|j| matches!(j, Job::KbRebuildVectorIndex { .. }))
    }

    async fn handle(&self, job: &Job) -> AppResult<()> {
        self.run(job, None, 1).await
    }

    async fn handle_queued(&self, queued: &crate::worker::QueuedJob) -> AppResult<()> {
        let job_id = queued
            .id
            .parse::<i64>()
            .ok()
            .map(crate::types::snowflake_id::SnowflakeId);
        self.run(&queued.job, job_id, i64::from(queued.attempts))
            .await
    }
}

impl KbRebuildVectorIndexHandler {
    async fn run(
        &self,
        job: &Job,
        job_id: Option<crate::types::snowflake_id::SnowflakeId>,
        attempt: i64,
    ) -> AppResult<()> {
        let Job::KbRebuildVectorIndex { kb_id, tenant_id } = job else {
            return Ok(());
        };
        // Traced long task (DR2: `running` row visible while rebuilding).
        let mut trace = crate::kb::trace::RunRecorder::create(
            crate::kb::trace::TraceMode::parse(&self.config.kb.trace_mode),
            crate::kb::trace::RunSpec {
                kind: crate::kb::models::kb_run::KIND_REBUILD_VECTOR,
                trigger_src: "job",
                tenant_id: tenant_id.clone(),
                kb_id: Some(*kb_id),
                doc_id: None,
                agent_id: None,
                session_id: None,
                job_id,
                attempt,
                long_running: true,
            },
        );
        let vector = self.runtime.vector.clone();
        let pool = self.pool.clone();
        trace.begin(&pool, &self.config).await;
        trace.stage("rebuild");
        let result =
            crate::kb::service::rebuild_vector_index_from_sql(&pool, &vector, *kb_id).await;
        match &result {
            Ok(n) => {
                trace.end_stage(
                    crate::kb::trace::STAGE_OK,
                    serde_json::json!({ "units": n, "backend": vector.backend_name() }),
                    None,
                );
                trace
                    .finish(&pool, crate::kb::models::kb_run::STATUS_OK, None)
                    .await;
            }
            Err(e) => {
                trace.fail_stage(&e.to_string());
                trace
                    .finish(
                        &pool,
                        crate::kb::models::kb_run::STATUS_FAILED,
                        Some(&e.to_string()),
                    )
                    .await;
            }
        }
        result.map(|_| ())
    }
}

/// kb_runs retention sweeper (kb-observability-design §10) — thin bridge
/// to `kb::diagnostics::sweep_runs` [抄RF:worker/handlers/itg_egress_cleanup.rs 清理模式].
pub struct KbRunsCleanupHandler {
    pool: crate::db::Pool,
    config: Arc<crate::config::app::AppConfig>,
}

impl KbRunsCleanupHandler {
    pub fn new(pool: crate::db::Pool, config: Arc<crate::config::app::AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for KbRunsCleanupHandler {
    async fn handle(&self, _job: &Job) -> AppResult<()> {
        let days = self.config.kb.trace_retention_days;
        if days <= 0 {
            return Ok(()); // keep forever
        }
        let removed = crate::kb::diagnostics::sweep_runs(&self.pool, days).await?;
        if removed > 0 {
            tracing::info!(
                "[kb] runs retention sweep: removed {removed} row(s) older than {days}d"
            );
        }
        Ok(())
    }
}
