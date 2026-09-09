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
        jobs.into_iter().find(|j| {
            matches!(j, Job::KbProcessDocument { .. })
        })
    }

    async fn handle(&self, job: &Job) -> AppResult<()> {
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
        match crate::kb::service::process_document(&deps, *doc_id, tenant_id).await {
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
}

impl KbRebuildVectorIndexHandler {
    pub fn new(pool: crate::db::Pool, runtime: Arc<KbRuntime>) -> Self {
        Self { pool, runtime }
    }
}

#[async_trait::async_trait]
impl JobHandler for KbRebuildVectorIndexHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::KbRebuildVectorIndex { kb_id, tenant_id } = job else {
            return Ok(());
        };
        // Full rebuild from SQL (the source of truth, D3): embedded chunks
        // of this KB, dim from the KB row.
        let kb = crate::kb::models::knowledge_base::find_kb_by_id(&self.pool, *kb_id, tenant_id)
            .await?
            .ok_or_else(|| {
                crate::errors::app_error::AppError::NotFound("kb_knowledge_base".into())
            })?;
        let dim = u32::try_from(kb.embedding_dim.unwrap_or(0)).unwrap_or(0);
        if dim == 0 {
            return Err(crate::errors::app_error::AppError::BadRequest(
                "kb has no embedding_dim configured".into(),
            ));
        }
        let chunks = raisfast_derive::crud_find_all!(
            &self.pool,
            "kb_chunks",
            crate::kb::models::chunk::KbChunk,
            where: ("kb_id", kb_id)
        )?;
        let mut items = Vec::new();
        for c in chunks.iter().filter(|c| c.embedding.is_some()) {
            items.push(crate::kb::vectors::VectorItem {
                unit_id: i64::from(c.id),
                kb_id: i64::from(c.kb_id),
                kind: c.kind.clone(),
                embedding: crate::kb::models::chunk::unpack_embedding(
                    c.embedding.as_deref().unwrap_or_default(),
                ),
            });
        }
        self.runtime
            .vector
            .rebuild(i64::from(*kb_id), dim, &items)
            .await?;
        tracing::info!(
            "[kb] vector index rebuilt for kb {kb_id}: {} units",
            items.len()
        );
        Ok(())
    }
}
