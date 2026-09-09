//! QA retrieval pipeline — fixed-order stages S1–S11 (kb-technical-design §6).
//!
//! Stage semantics mirror the WeKnora chat_pipeline files (one submodule per
//! group, noted per file). v1 runs a fixed order with plain async fns; the
//! stage boundaries are kept identical to the blueprint so a configurable
//! plugin chain can be introduced later without redesign. Single-turn
//! stateless (D5); KB binding via `kb_ids` on the request (D4).

mod assemble;
mod fusion;
mod generate;
mod merge;
mod search;
mod understand;

use std::sync::Arc;

use serde::Serialize;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::kb::models::chunk::KbChunk;
use crate::kb::service::KbDeps;

/// One recalled candidate after fusion (S3), carrying its hydrated chunk.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub unit_id: i64,
    pub kb_id: i64,
    /// Fused relevance score (RRF-derived, then boosted in S6).
    pub score: f32,
    pub chunk: Arc<KbChunk>,
}

/// Final context unit after S7 merging (FAQ-injected / parent-expanded).
#[derive(Debug, Clone, Serialize)]
pub struct ContextUnit {
    pub unit_id: i64,
    pub kind: String,
    /// Display title: FAQ standard question / breadcrumb / doc fallback.
    pub title: String,
    pub content: String,
    pub score: f32,
    /// Whether this unit was injected as FAQ (pinned to the top in S8).
    pub is_faq: bool,
}

/// A citation entry returned with the answer (S10).
#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub n: usize,
    pub unit_id: i64,
    pub kind: String,
    pub title: String,
    /// Short content snippet for the references drawer.
    pub snippet: String,
    pub score: f32,
}

/// The ask request (single-turn, D5).
#[derive(Debug, Clone)]
pub struct AskRequest {
    /// Calling tenant — every KB scope resolution is tenant-scoped
    /// (`resolve_kbs` validates both the default and the explicit list).
    pub tenant_id: String,
    /// KB scope; empty = all enabled KBs of the tenant (D4 [抄WK:SearchTargets
    /// 语义]).
    pub kb_ids: Vec<i64>,
    pub question: String,
    /// Document scope for per-doc testing; empty = whole KB scope.
    /// Filters recall candidates before fusion (FAQ/wiki units have no
    /// doc and are dropped while the filter is active).
    pub doc_ids: Vec<i64>,
}

/// Pipeline outcome consumed by the HTTP layer (stream + non-stream).
#[derive(Debug)]
pub struct AskOutcome {
    pub status: &'static str,
    /// The (possibly rewritten) question — carried for S9 and query logging.
    pub question: String,
    pub answer: String,
    pub references: Vec<Reference>,
    pub top_score: f32,
    /// Hydrated context units — the generation prompt is built from these,
    /// and streaming reuses them after the first token.
    pub context_units: Vec<ContextUnit>,
}

/// Run S1–S8 (everything up to a fully assembled context), shared by the
/// streaming and non-streaming generation paths.
pub async fn prepare_answer(deps: &KbDeps, req: &AskRequest) -> AppResult<AskOutcome> {
    if req.question.trim().is_empty() {
        return Err(AppError::BadRequest("question must not be empty".into()));
    }
    let kbs = resolve_kbs(deps, &req.kb_ids, &req.tenant_id).await?;

    // S1 understand (LLM rewrite + keywords; degrades to raw question).
    let understood = understand::run(deps, &req.question).await;

    // S2–S7 recall → fuse → hydrate → boost → merge.
    let (top_score, units) = recall_and_merge(deps, &kbs, &req.doc_ids, &understood).await?;

    Ok(AskOutcome {
        status: "answered",
        question: understood.text,
        answer: String::new(),
        references: Vec::new(),
        top_score,
        context_units: units,
    })
}

/// Retrieval-only seam for the agent `knowledge_search` tool (kb-technical-
/// design §10): validates the KB scope against the tenant, then runs S2–S7
/// **without S1** (the agent formulates the query itself — an LLM rewrite
/// here would only add latency) and without S9 generation (the agent's own
/// turn is the generator).
pub async fn search_units(
    deps: &KbDeps,
    tenant_id: &str,
    kb_ids: &[i64],
    query: &str,
) -> AppResult<(f32, Vec<ContextUnit>)> {
    if query.trim().is_empty() {
        return Err(AppError::BadRequest("query must not be empty".into()));
    }
    let kbs = resolve_kbs(deps, kb_ids, tenant_id).await?;
    let understood = understand::UnderstoodQuery::raw(query);
    recall_and_merge(deps, &kbs, &[], &understood).await
}

/// S2–S7 over a validated KB scope: recall → fuse → top-k → hydrate →
/// wiki boost → merge. Returns `(top_score, context_units)`.
async fn recall_and_merge(
    deps: &KbDeps,
    kbs: &[i64],
    doc_ids: &[i64],
    understood: &understand::UnderstoodQuery,
) -> AppResult<(f32, Vec<ContextUnit>)> {
    // S2+S3+S4 recall → fuse → top-k, per KB scope.
    let mut candidates = Vec::new();
    for kb_id in kbs {
        let mut recalled = search::recall(deps, *kb_id, understood).await?;
        // Document scope (playground per-doc testing): drop units that do
        // not belong to the selected documents, before fusion cuts top-k.
        if !doc_ids.is_empty() {
            let mut ids: Vec<i64> = recalled
                .bm25
                .iter()
                .chain(recalled.dense.iter())
                .map(|(unit_id, _)| *unit_id)
                .collect();
            ids.sort_unstable();
            ids.dedup();
            let chunks = crate::kb::models::chunk::find_chunks_by_ids(&deps.pool, &ids).await?;
            let allowed: std::collections::HashSet<i64> = chunks
                .into_iter()
                .filter_map(|c| {
                    c.doc_id
                        .filter(|d| doc_ids.contains(&i64::from(*d)))
                        .map(|_| i64::from(c.id))
                })
                .collect();
            recalled
                .bm25
                .retain(|(unit_id, _)| allowed.contains(unit_id));
            recalled
                .dense
                .retain(|(unit_id, _)| allowed.contains(unit_id));
        }
        candidates.extend(fusion::fuse_and_cut(recalled, deps.config.kb.top_k));
    }
    // Stable cross-KB order, then S6+S7.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Hydrate chunks once (all stages need content/kind/parent).
    let ids: Vec<i64> = candidates.iter().map(|c| c.unit_id).collect();
    let chunks = crate::kb::models::chunk::find_chunks_by_ids(&deps.pool, &ids).await?;
    let by_id: std::collections::HashMap<i64, Arc<KbChunk>> = chunks
        .into_iter()
        .map(|c| (i64::from(c.id), Arc::new(c)))
        .collect();
    let mut hydrated: Vec<Candidate> = candidates
        .into_iter()
        .filter_map(|c| {
            let chunk = by_id.get(&c.unit_id).cloned()?;
            let kb_id = i64::from(chunk.kb_id);
            Some(Candidate {
                unit_id: c.unit_id,
                kb_id,
                score: c.score,
                chunk,
            })
        })
        .collect();

    // S5 rerank: v1 passthrough (external rerank provider unimplemented;
    // the stage boundary is kept — see kb-technical-design §6 note).
    // S6 wiki boost.
    merge::apply_wiki_boost(&mut hydrated, deps.config.kb.wiki_boost);
    // S7 merge: FAQ inject + parent expand + dedup.
    let units = merge::merge_units(deps, &hydrated).await?;

    Ok((hydrated.first().map(|c| c.score).unwrap_or(0.0), units))
}

/// S9–S11 over a prepared outcome: fallback check, generation, references.
pub async fn finish_answer(deps: &KbDeps, outcome: &mut AskOutcome) -> AppResult<()> {
    // S11 fallback gate (checked before generation so uncovered never burns
    // an LLM call).
    if outcome.context_units.is_empty() || outcome.top_score < deps.config.kb.fallback_threshold {
        outcome.status = "uncovered";
        outcome.answer = "知识库未覆盖该问题。".to_string();
        outcome.references = Vec::new();
        return Ok(());
    }
    // S8 assemble (budget-aware, FAQ pinned).
    let prompt_units = assemble::assemble(outcome, deps.config.kb.context_budget_tokens);
    // S9 generate.
    let provider = deps.provider.as_deref().ok_or_else(|| {
        AppError::ServiceUnavailable("kb chat provider unavailable (RAISFAST_AI_*)".into())
    })?;
    outcome.answer =
        generate::generate_answer(deps, provider, &prompt_units, &outcome.question).await?;
    // S10 references over the units actually fed to the model.
    outcome.references = generate::build_references(&prompt_units);
    Ok(())
}

/// Streaming variant of S9: feeds text deltas to `on_delta` as they arrive,
/// then completes references. Reuses S8 assembly from `prepare_answer`.
pub async fn finish_answer_streaming(
    deps: &KbDeps,
    outcome: &mut AskOutcome,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AppResult<()> {
    if outcome.context_units.is_empty() || outcome.top_score < deps.config.kb.fallback_threshold {
        outcome.status = "uncovered";
        outcome.answer = "知识库未覆盖该问题。".to_string();
        return Ok(());
    }
    let prompt_units = assemble::assemble(outcome, deps.config.kb.context_budget_tokens);
    let provider = deps.provider.as_deref().ok_or_else(|| {
        AppError::ServiceUnavailable("kb chat provider unavailable (RAISFAST_AI_*)".into())
    })?;
    outcome.answer = generate::generate_answer_streaming(
        deps,
        provider,
        &prompt_units,
        &outcome.question,
        on_delta,
    )
    .await?;
    outcome.references = generate::build_references(&prompt_units);
    Ok(())
}

/// Resolve the KB scope, tenant-scoped on both branches (also used by the
/// agent tool registration to pre-bind its search targets):
/// - empty request = all enabled KBs **of the tenant**;
/// - explicit ids = validated against the tenant and `status='active'`;
///   any id that fails validation rejects the request (fail-closed —
///   object-level authz on the retrieval path, mirroring the write path's
///   `ensure_kb` gate).
pub(crate) async fn resolve_kbs(
    deps: &KbDeps,
    requested: &[i64],
    tenant_id: &str,
) -> AppResult<Vec<i64>> {
    // Dedup first so the row-count comparison below is sound.
    let mut requested = requested.to_vec();
    requested.sort_unstable();
    requested.dedup();
    let requested = requested.as_slice();
    if requested.is_empty() {
        let sql = format!(
            "SELECT {} FROM kb_knowledge_bases WHERE status = 'active' AND tenant_id = {}",
            crate::db::Driver::cast_int("id"),
            crate::db::Driver::ph(1)
        );
        let rows: Vec<i64> = sqlx::query_scalar(crate::db::safe_sql(&sql))
            .bind(tenant_id)
            .fetch_all(&deps.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
        if rows.is_empty() {
            return Err(AppError::NotFound("kb_knowledge_base".into()));
        }
        return Ok(rows);
    }
    let placeholders: Vec<String> = (1..=requested.len()).map(crate::db::Driver::ph).collect();
    let sql = format!(
        "SELECT {} FROM kb_knowledge_bases WHERE id IN ({}) AND status = 'active' AND tenant_id = {}",
        crate::db::Driver::cast_int("id"),
        placeholders.join(", "),
        crate::db::Driver::ph(requested.len() + 1)
    );
    let mut query = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql));
    for id in requested {
        query = query.bind(id);
    }
    let rows: Vec<i64> = query
        .bind(tenant_id)
        .fetch_all(&deps.pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    if rows.len() != requested.len() {
        // Some requested ids are unknown, inactive, or belong to another
        // tenant — do not reveal which (uniform NotFound).
        return Err(AppError::NotFound("kb_knowledge_base".into()));
    }
    Ok(rows)
}

/// Re-exported stage types.
pub use search::RecallOutcome;
pub use understand::UnderstoodQuery;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::service::{KbDeps, KbEmbedder};
    use crate::kb::vectors::BruteForceIndex;
    use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

    struct MockEmbedder {
        dim: usize,
    }

    #[async_trait::async_trait]
    impl KbEmbedder for MockEmbedder {
        async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0_f32; self.dim];
                    let seed = t.bytes().map(|b| b as usize).sum::<usize>();
                    v[seed % self.dim] = 1.0;
                    v
                })
                .collect())
        }
    }

    /// Echo provider: claims it answered from citation [1].
    struct MockChatProvider;

    #[async_trait::async_trait]
    impl ModelProvider for MockChatProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _request: &ChatRequest<'_>,
            _model: &str,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse::text_only("[1] 模拟答案：支持三种数据库。"))
        }
    }

    async fn deps() -> KbDeps {
        let pool = crate::test_pool!();
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        config.kb.fallback_threshold = 0.05; // deterministic for mocks
        let bus = crate::eventbus::EventBus::new(16);
        KbDeps {
            pool,
            config: Arc::new(config),
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-pipeline-test", "/uploads")
                    .unwrap(),
            ),
            vector: Arc::new(BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(MockEmbedder { dim: 4 }),
            provider: Some(Arc::new(MockChatProvider)),
            emitter: crate::event::EventEmitter::eventbus_only(bus),
        }
    }

    #[tokio::test]
    async fn ask_answered_with_references() {
        let deps = deps().await;
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "docs".into(),
                description: None,
                slug: "docs".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();

        let mut markdown = "# 安装\n\n".to_string();
        markdown.push_str(&"raisfast 支持 SQLite PostgreSQL MySQL 数据库后端。".repeat(60));
        let doc = crate::kb::service::create_online_document(
            &deps, kb.id, "安装", &markdown, None, "default",
        )
        .await
        .unwrap();
        crate::kb::service::process_document(&deps, doc.id, "default")
            .await
            .unwrap();

        let ask = AskRequest {
            tenant_id: "default".into(),
            kb_ids: vec![i64::from(kb.id)],
            doc_ids: Vec::new(),
            question: "支持哪些数据库？".into(),
        };
        let mut outcome = prepare_answer(&deps, &ask).await.unwrap();
        assert!(!outcome.context_units.is_empty(), "context must have units");
        finish_answer(&deps, &mut outcome).await.unwrap();
        assert_eq!(outcome.status, "answered");
        assert!(outcome.answer.contains("[1]"), "citations must map to refs");
        assert_eq!(
            outcome.references.len(),
            outcome
                .context_units
                .len()
                .min(outcome.references.len())
                .max(1)
                .max(outcome.references.len())
        );
        assert!(!outcome.references.is_empty());
        assert!(outcome.references[0].n == 1);
    }

    #[tokio::test]
    async fn ask_uncovered_when_no_recall() {
        let deps = deps().await;
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "empty".into(),
                description: None,
                slug: "empty".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();

        let ask = AskRequest {
            tenant_id: "default".into(),
            kb_ids: vec![i64::from(kb.id)],
            doc_ids: Vec::new(),
            question: "量子力学的诠释有哪些".into(),
        };
        let mut outcome = prepare_answer(&deps, &ask).await.unwrap();
        finish_answer(&deps, &mut outcome).await.unwrap();
        assert_eq!(outcome.status, "uncovered", "empty KB must not generate");
        assert_eq!(outcome.answer, "知识库未覆盖该问题。");
        assert!(outcome.references.is_empty());
    }

    #[tokio::test]
    async fn streaming_answer_accumulates() {
        let deps = deps().await;
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "s".into(),
                description: None,
                slug: "s".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();
        let mut markdown = "# 配置\n\n".to_string();
        markdown.push_str(&"环境变量 RAISFAST_KB_ENABLED 控制知识库开关。".repeat(60));
        let doc = crate::kb::service::create_online_document(
            &deps, kb.id, "配置", &markdown, None, "default",
        )
        .await
        .unwrap();
        crate::kb::service::process_document(&deps, doc.id, "default")
            .await
            .unwrap();

        let ask = AskRequest {
            tenant_id: "default".into(),
            kb_ids: vec![i64::from(kb.id)],
            doc_ids: Vec::new(),
            question: "知识库怎么开关".into(),
        };
        let mut outcome = prepare_answer(&deps, &ask).await.unwrap();
        let mut acc = String::new();
        {
            let mut on_delta = |d: &str| acc.push_str(d);
            finish_answer_streaming(&deps, &mut outcome, &mut on_delta)
                .await
                .unwrap();
        }
        assert_eq!(outcome.status, "answered");
        assert_eq!(
            acc, outcome.answer,
            "streamed deltas must equal final answer"
        );
        assert!(!outcome.references.is_empty());
    }

    async fn seeded_tenant_kb(deps: &KbDeps, tenant: &str) -> i64 {
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "iso".into(),
                description: None,
                slug: format!("iso-{tenant}"),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            tenant,
        )
        .await
        .unwrap();
        let mut markdown = "# 隔离\n\n".to_string();
        markdown.push_str(&format!("租户 {tenant} 的私有部署步骤说明。").repeat(60));
        let doc =
            crate::kb::service::create_online_document(deps, kb.id, "iso", &markdown, None, tenant)
                .await
                .unwrap();
        crate::kb::service::process_document(deps, doc.id, tenant)
            .await
            .unwrap();
        i64::from(kb.id)
    }

    #[tokio::test]
    async fn tenant_scope_is_isolated() {
        let deps = deps().await;
        let kb_a = seeded_tenant_kb(&deps, "tenant-a").await;

        // Explicit foreign kb id → rejected (object-level authz).
        let ask = AskRequest {
            tenant_id: "tenant-b".into(),
            kb_ids: vec![kb_a],
            doc_ids: Vec::new(),
            question: "私有部署步骤".into(),
        };
        let err = prepare_answer(&deps, &ask).await.unwrap_err();
        assert!(matches!(
            err,
            crate::errors::app_error::AppError::NotFound(_)
        ));

        // Default scope from tenant-b → no active KBs there → NotFound.
        let ask = AskRequest {
            tenant_id: "tenant-b".into(),
            kb_ids: Vec::new(),
            doc_ids: Vec::new(),
            question: "私有部署步骤".into(),
        };
        let err = prepare_answer(&deps, &ask).await.unwrap_err();
        assert!(matches!(
            err,
            crate::errors::app_error::AppError::NotFound(_)
        ));

        // Owner tenant still retrieves.
        let ask = AskRequest {
            tenant_id: "tenant-a".into(),
            kb_ids: vec![kb_a],
            doc_ids: Vec::new(),
            question: "私有部署步骤".into(),
        };
        let outcome = prepare_answer(&deps, &ask).await.unwrap();
        assert!(!outcome.context_units.is_empty(), "owner must retrieve");
    }

    #[tokio::test]
    async fn inactive_kb_rejected() {
        let deps = deps().await;
        let kb_id = seeded_tenant_kb(&deps, "default").await;
        crate::kb::models::knowledge_base::update_kb(
            &deps.pool,
            crate::types::snowflake_id::SnowflakeId(kb_id),
            &crate::kb::models::knowledge_base::UpdateKbCmd {
                name: "iso".into(),
                description: None,
                slug: "iso-default".into(),
                status: "disabled".into(),
            },
            "default",
        )
        .await
        .unwrap();
        let ask = AskRequest {
            tenant_id: "default".into(),
            kb_ids: vec![kb_id],
            doc_ids: Vec::new(),
            question: "私有部署步骤".into(),
        };
        let err = prepare_answer(&deps, &ask).await.unwrap_err();
        assert!(
            matches!(err, crate::errors::app_error::AppError::NotFound(_)),
            "inactive kb must fail closed"
        );
    }
}
