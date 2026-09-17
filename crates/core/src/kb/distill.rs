//! Wiki distillation — LLM drafts knowledge pages from ready documents
//! (kb-technical-design §7).
//!
//! Two-pass pipeline adapted from WeKnora's prompts (v1 reduction of their
//! multi-pass wiki agent [抄WK:internal/agent/prompts_wiki.go——已读]):
//! ① knowledge extraction per document: entities + concepts with slug
//!   continuity rules (reuse existing slugs, don't re-mention removed ones)
//!   [抄WK:WikiKnowledgeExtractPrompt];
//! ② per-topic page drafting from the supporting chunks, with wiki-links
//!   `[[slug|display name]]` against the available-page list
//!   [抄WK:WikiSummaryPrompt 链接规则].
//! Declared v1 reduction: WK's chunk-citation pass (WikiChunkCitationPrompt)
//! is replaced by slug/title matching for chunk selection; provenance is
//! doc-level spans. Pages are DRAFTS until human approval (P1).

use std::sync::Arc;

use raisfast_agent::ChatRequest;
use raisfast_agent::messages::{ChatMessage, ChatRole};

use crate::errors::app_error::{AppError, AppResult};
use crate::kb::models;
use crate::kb::models::chunk::KbChunk;
use crate::kb::service::KbDeps;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::prompt_file::prompt_file;

/// One extracted topic.
#[derive(Debug, Clone, serde::Deserialize)]
struct Topic {
    name: String,
    slug: String,
    #[serde(default)]
    description: String,
}

async fn chat_json(
    deps: &KbDeps,
    tenant: &str,
    model: Option<&str>,
    system: &str,
    user: &str,
) -> AppResult<String> {
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: Some(system.to_string()),
            images: Vec::new(),
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: ChatRole::User,
            content: Some(user.to_string()),
            images: Vec::new(),
            tool_calls: None,
            tool_call_id: None,
        },
    ];
    let request = ChatRequest {
        messages: &messages,
        tools: None,
        temperature: Some(0.2),
        max_tokens: None,
        stop: None,
    };
    crate::kb::service::kb_chat_model(deps, tenant, model, &request)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("distill chat: {e}")))
}

/// Extract JSON from an LLM reply (tolerates fences / prose wrappers).
fn parse_json_object<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

/// The distill job body: draft wiki pages for the given ready documents.
pub async fn distill_documents(
    deps: &KbDeps,
    kb_id: SnowflakeId,
    doc_ids: &[SnowflakeId],
    tenant_id: &str,
) -> AppResult<Vec<SnowflakeId>> {
    // Per-KB synthesis model [抄WK:wiki_ingest_batch.go 的
    // WikiConfig.SynthesisModelID → SummaryModelID 级联]；我们的兜底更宽：
    // KB 行 distill_model → 全局 RAISFAST_KB_DISTILL_MODEL → 租户默认。
    let kb_row = models::knowledge_base::find_kb_by_id(&deps.pool, kb_id, tenant_id).await?;
    let distill_model = kb_row
        .as_ref()
        .and_then(|kb| kb.distill_model.clone())
        .filter(|m| !m.is_empty())
        .or_else(|| {
            deps.config
                .kb
                .distill_model
                .clone()
                .filter(|m| !m.is_empty())
        });

    // Gather chunks per doc.
    let mut doc_chunks: Vec<(SnowflakeId, Vec<KbChunk>)> = Vec::new();
    for doc_id in doc_ids {
        let chunks = models::chunk::find_chunks_by_doc(&deps.pool, *doc_id).await?;
        if chunks.is_empty() {
            continue;
        }
        doc_chunks.push((*doc_id, chunks));
    }
    if doc_chunks.is_empty() {
        return Ok(Vec::new());
    }

    // Pass ① per-doc extraction with slug continuity against existing pages.
    let existing = models::wiki_page::list_pages(&deps.pool, Some(kb_id), None, 1, 500, tenant_id)
        .await?
        .0;
    let previous_slugs: Vec<String> = existing
        .iter()
        .map(|p| format!("[[{}]] = {}", p.slug, p.title))
        .collect();
    let mut topics: Vec<Topic> = Vec::new();
    for (doc_id, chunks) in &doc_chunks {
        let corpus: String = chunks
            .iter()
            .map(|c| c.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let user = format!(
            "<previous_slugs>\n{}\n</previous_slugs>\n\n<document>\n{corpus}\n</document>",
            previous_slugs.join("\n")
        );
        let reply = chat_json(
            deps,
            tenant_id,
            distill_model.as_deref(),
            &prompt_file!("src/kb/prompts/distill_extract.md"),
            &user,
        )
        .await?;
        let parsed: Option<serde_json::Value> = parse_json_object(&reply);
        if let Some(v) = parsed
            && let Some(arr) = v.get("topics").and_then(|t| t.as_array())
        {
            for item in arr {
                if let Ok(t) = serde_json::from_value::<Topic>(item.clone())
                    && !t.name.is_empty()
                    && !t.slug.is_empty()
                    && !topics.iter().any(|x| x.slug == t.slug)
                {
                    topics.push(t);
                }
            }
        }
        let _ = doc_id;
    }
    if topics.is_empty() {
        return Ok(Vec::new());
    }

    // Pass ② per-topic drafting (chunks selected by slug/name matching —
    // declared reduction of WK's citation pass). Known slugs UPSERT into
    // their existing page instead of duplicating [抄WK:cite known-slug
    // union 语义]; slug continuity stops being a prompt-only hint.
    let existing_by_slug: std::collections::HashMap<String, SnowflakeId> =
        existing.iter().map(|p| (p.slug.clone(), p.id)).collect();
    let available: String = topics
        .iter()
        .map(|t| format!("[[{}]] = {}", t.slug, t.name))
        .collect::<Vec<_>>()
        .join("\n");
    let mut created = Vec::new();
    for topic in &topics {
        let mut material = String::new();
        let mut source_docs: Vec<SnowflakeId> = Vec::new();
        for (doc_id, chunks) in &doc_chunks {
            let matched: Vec<&KbChunk> = chunks
                .iter()
                .filter(|c| {
                    let hay = &c.content;
                    hay.contains(&topic.name)
                        || topic
                            .slug
                            .rsplit('/')
                            .next()
                            .is_some_and(|tail| hay.contains(tail.replace('-', "").as_str()))
                })
                .take(12)
                .collect();
            if matched.is_empty() {
                continue;
            }
            source_docs.push(*doc_id);
            material.push_str(
                &matched
                    .iter()
                    .map(|c| c.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            );
            material.push_str("\n\n");
        }
        if material.trim().is_empty() {
            continue;
        }
        let user = format!(
            "<available_wiki_pages>\n{available}\n</available_wiki_pages>\n\n<material>\n{material}\n</material>\n\n主题：{}（{}）",
            topic.name, topic.description
        );
        let content = chat_json(
            deps,
            tenant_id,
            distill_model.as_deref(),
            &prompt_file!("src/kb/prompts/distill_draft.md"),
            &user,
        )
        .await?
        .trim()
        .to_string();
        if content.is_empty() {
            continue;
        }
        // Linked pages: other topics referenced via [[slug|…]] in the draft.
        let linked: Vec<String> = topics
            .iter()
            .filter(|t| t.slug != topic.slug && content.contains(&format!("[[{}|", t.slug)))
            .map(|t| t.slug.clone())
            .collect();
        let linked_json = serde_json::json!(linked);
        let page_id = match existing_by_slug.get(&topic.slug) {
            Some(pid) => {
                models::wiki_page::update_page_draft(
                    &deps.pool,
                    *pid,
                    &topic.name,
                    &content,
                    Some(&topic.description),
                    Some(linked_json),
                    tenant_id,
                )
                .await?;
                *pid
            }
            None => {
                let page = models::wiki_page::create_page(
                    &deps.pool,
                    &models::wiki_page::CreateWikiPageCmd {
                        kb_id,
                        title: topic.name.clone(),
                        slug: topic.slug.clone(),
                        content,
                        summary: Some(topic.description.clone()),
                        linked_page_ids: Some(linked_json),
                        created_by: None,
                    },
                    tenant_id,
                )
                .await?;
                page.id
            }
        };
        for doc_id in source_docs {
            models::wiki_source::link_source(&deps.pool, page_id, 0, doc_id).await?;
        }
        created.push(page_id);
    }
    Ok(created)
}

/// Publish an approved page: snapshot to `content_revisions`, index its
/// chunks (kind='wiki_page') into vector + BM25 (§7 入池).
pub async fn publish_page(
    deps: &KbDeps,
    page_id: SnowflakeId,
    reviewer: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    let Some(page) = models::wiki_page::find_page_by_id(&deps.pool, page_id, tenant_id).await?
    else {
        return Err(AppError::NotFound("kb_wiki_page".into()));
    };
    // D2: snapshot the content being REPLACED — i.e. any already-published
    // version (first publish of a draft replaces nothing).
    if page.status == "published" {
        snapshot_page(&deps.pool, &page, reviewer, tenant_id).await?;
    }
    let revision =
        models::wiki_page::publish_page(&deps.pool, page_id, reviewer, tenant_id).await?;

    // Re-index: wipe this page's units, re-chunk, embed, upsert.
    index_page_units(deps, &page, revision).await?;
    Ok(())
}

/// Chunk a page (heading strategy), embed, write into both indexes.
/// Idempotent per page (delete-then-insert like document re-parse).
async fn index_page_units(
    deps: &KbDeps,
    page: &models::wiki_page::KbWikiPage,
    revision: i64,
) -> AppResult<()> {
    // wipe old units of this page (SQL + vector + fts)
    let old = models::chunk::find_chunks_by_page(&deps.pool, page.id).await?;
    let old_ids: Vec<i64> = old.iter().map(|c| i64::from(c.id)).collect();
    if !old_ids.is_empty() {
        deps.vector.delete(i64::from(page.kb_id), &old_ids).await?;
    }
    for c in &old {
        deps.kbsearch.delete_document(i64::from(c.id)).await?;
    }
    models::chunk::delete_chunks_by_page(&deps.pool, page.id).await?;

    let cfg = crate::kb::chunker::ChunkerConfig::default();
    let raw = crate::kb::chunker::chunk_markdown(&page.content, &cfg);
    let now = crate::utils::tz::now_utc();
    let mut inserts = Vec::with_capacity(raw.len());
    for (i, c) in raw.iter().enumerate() {
        inserts.push(models::chunk::KbChunkInsert {
            id: crate::utils::id::new_snowflake_id(),
            kb_id: page.kb_id,
            doc_id: None,
            faq_id: None,
            wiki_page_id: Some(page.id),
            kind: "wiki_page".into(),
            parent_id: None,
            seq: i as i64,
            content: c.content.clone(),
            breadcrumb: if c.breadcrumb.is_empty() {
                None
            } else {
                Some(c.breadcrumb.clone())
            },
            byte_start: c.byte_start as i64,
            byte_end: c.byte_end as i64,
            questions: None,
            embedding: None,
            embedding_model: None,
            created_at: now,
        });
    }
    for ins in &inserts {
        models::chunk::insert_chunk(&deps.pool, ins).await?;
    }

    let texts: Vec<&str> = inserts.iter().map(|c| c.content.as_str()).collect();
    let model = page_knowledge(deps, page.kb_id).await?;
    let vectors = deps
        .embedder
        .embed_for(&page.tenant_id, &model.0, model.1, &texts)
        .await?;
    let mut items = Vec::with_capacity(inserts.len());
    let mut fts = Vec::with_capacity(inserts.len());
    for (idx, ins) in inserts.iter().enumerate() {
        models::chunk::update_embedding(&deps.pool, ins.id, &vectors[idx]).await?;
        items.push(crate::kb::vectors::VectorItem {
            unit_id: i64::from(ins.id),
            kb_id: i64::from(page.kb_id),
            kind: "wiki_page".into(),
            embedding: vectors[idx].clone(),
        });
        fts.push(crate::kb::kbsearch::KbIndexUnit {
            unit_id: i64::from(ins.id),
            kb_id: i64::from(page.kb_id),
            doc_id: i64::from(ins.id), // page units key on themselves
            kind: "wiki_page".into(),
            text: format!("{}\n{}", page.title, ins.content),
        });
    }
    let dim = vectors.first().map(|v| v.len() as u32).unwrap_or(0);
    deps.vector
        .upsert(i64::from(page.kb_id), dim, &items)
        .await?;
    deps.kbsearch.reindex_document(&fts).await?;
    let _ = revision;
    Ok(())
}

/// (model, dim) pinned on the KB row.
async fn page_knowledge(
    deps: &crate::kb::service::KbDeps,
    kb_id: SnowflakeId,
) -> AppResult<(String, u32)> {
    let kb = models::knowledge_base::find_kb_by_id(&deps.pool, kb_id, "default")
        .await?
        .ok_or_else(|| AppError::NotFound("kb_knowledge_base".into()))?;
    Ok((
        kb.embedding_model.unwrap_or_default(),
        u32::try_from(kb.embedding_dim.unwrap_or(0)).unwrap_or(0),
    ))
}

async fn snapshot_page(
    pool: &crate::db::Pool,
    page: &models::wiki_page::KbWikiPage,
    actor: SnowflakeId,
    _tenant_id: &str,
) -> AppResult<()> {
    let _ = actor;
    let snapshot = serde_json::json!({
        "title": page.title,
        "slug": page.slug,
        "content": page.content,
        "summary": page.summary,
        "status": page.status,
    });
    raisfast_derive::crud_insert!(
        pool,
        "content_revisions",
        [
            "content_type" => "kb_wiki_page",
            "record_id" => page.id,
            "revision_number" => page.current_revision,
            "snapshot" => snapshot,
            "created_by" => page.reviewed_by.map(i64::from).unwrap_or(0)
        ]
    )?;
    Ok(())
}

/// Line-level diff of two page contents via `similar`
/// [抄EXT:mitsuhiko/similar——§7 行级 diff].
pub fn line_diff(old: &str, new: &str) -> Vec<DiffLine> {
    use similar::ChangeTag;
    use similar::TextDiff;
    let diff = TextDiff::from_lines(old, new);
    let mut out = Vec::new();
    for change in diff.iter_all_changes() {
        let tag = match change.tag() {
            ChangeTag::Equal => "equal",
            ChangeTag::Delete => "delete",
            ChangeTag::Insert => "insert",
        };
        out.push(DiffLine {
            tag: tag.to_string(),
            text: change.value().trim_end_matches('\n').to_string(),
        });
    }
    out
}

/// One line of a diff (serde-friendly for the admin review UI).
#[derive(Debug, Clone, serde::Serialize)]
pub struct DiffLine {
    pub tag: String,
    pub text: String,
}

/// Arc re-export for handler wiring.
pub type SharedDeps = Arc<KbDeps>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::service::{KbDeps, KbEmbedder};
    use crate::kb::vectors::BruteForceIndex;
    use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};
    use std::sync::Mutex;

    /// Sequential scripted provider: replies popped in order.
    struct ScriptedProvider {
        replies: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl ModelProvider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn chat(
            &self,
            _request: &ChatRequest<'_>,
            _model: &str,
        ) -> Result<ChatResponse, ProviderError> {
            let mut queue = self.replies.lock().unwrap_or_else(|e| e.into_inner());
            let reply = queue.pop().unwrap_or_default();
            Ok(ChatResponse::text_only(reply))
        }
    }

    struct MockEmbedder;

    #[async_trait::async_trait]
    impl KbEmbedder for MockEmbedder {
        async fn embed(&self, _tenant: &str, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0_f32; 4];
                    v[t.len() % 4] = 1.0;
                    v
                })
                .collect())
        }
    }

    async fn deps_with(replies: Vec<String>) -> KbDeps {
        let pool = crate::test_pool!();
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        let bus = crate::eventbus::EventBus::new(16);
        let router = crate::llm::service::LlmRouter::with_provider_for_test(
            Some(pool.clone()),
            Arc::new(ScriptedProvider {
                replies: Mutex::new(replies),
            }),
            &["kb-test-model"],
            Some("kb-test-model"),
        )
        .await;
        KbDeps {
            pool,
            config: Arc::new(config),
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-distill-test", "/uploads")
                    .unwrap(),
            ),
            vector: Arc::new(BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(MockEmbedder),
            reranker: None,
            router,
            emitter: crate::event::EventEmitter::eventbus_only(bus),
        }
    }

    async fn seed_kb_and_doc(deps: &KbDeps) -> (SnowflakeId, SnowflakeId) {
        let kb = models::knowledge_base::create_kb(
            &deps.pool,
            &models::knowledge_base::CreateKbCmd {
                name: "kb".into(),
                description: None,
                slug: "kb".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
                rerank_model: None,
                rerank_window: None,
                rerank_threshold: None,
                chat_model: None,
                distill_model: None,
                image_config: None,
            },
            "default",
        )
        .await
        .unwrap();
        let mut markdown =
            "# 数据库\n\nraisfast 支持 SQLite PostgreSQL MySQL。向量检索由 Qdrant 提供。"
                .to_string();
        markdown.push_str(&"配置详见环境变量章节。".repeat(40));
        let doc = crate::kb::service::create_online_document(
            deps,
            kb.id,
            "数据库文档",
            &markdown,
            None,
            "default",
        )
        .await
        .unwrap();
        crate::kb::service::process_document(deps, doc.id, "default")
            .await
            .unwrap();
        (kb.id, doc.id)
    }

    #[tokio::test]
    async fn distill_creates_drafts_with_sources() {
        let topics = r#"{"topics":[
            {"name":"向量检索","slug":"topic/xiang-liang-jian-suo","description":"向量检索说明"},
            {"name":"数据库","slug":"topic/shu-ju-ku","description":"数据库支持说明"}
        ]}"#;
        let page1 = "## 概述\n\n向量检索基于 [[topic/shu-ju-ku|数据库]] 索引。\n\n## 要点\n\n- 快"
            .to_string();
        let page2 = "## 支持列表\n\nSQLite、PostgreSQL、MySQL。\n\n## 要点\n\n- 三种".to_string();
        // replies pop in reverse (pop_front? we used pop() → LIFO): push in reverse order
        let deps = deps_with(vec![page2, page1, topics.to_string()]).await;
        let (kb_id, doc_id) = seed_kb_and_doc(&deps).await;

        let created = distill_documents(&deps, kb_id, &[doc_id], "default")
            .await
            .unwrap();
        assert_eq!(created.len(), 2, "two topics → two draft pages");
        let pages =
            models::wiki_page::list_pages(&deps.pool, Some(kb_id), Some("draft"), 1, 10, "default")
                .await
                .unwrap()
                .0;
        assert_eq!(pages.len(), 2);
        // provenance: each page links the source doc
        for p in &pages {
            let srcs = models::wiki_source::find_page_ids_by_doc(&deps.pool, doc_id)
                .await
                .unwrap();
            assert!(srcs.contains(&i64::from(p.id)));
        }
        // linked ids recorded for the cross-link
        let linked = pages
            .iter()
            .find(|p| p.slug == "topic/xiang-liang-jian-suo")
            .unwrap()
            .linked_page_ids
            .clone()
            .unwrap();
        assert!(linked.to_string().contains("topic/shu-ju-ku"));
    }

    #[tokio::test]
    async fn publish_indexes_wiki_units_and_boost_path() {
        let topics = r#"{"topics":[{"name":"数据库","slug":"topic/db","description":"db"}]}"#;
        let page = "## 概述\n\n支持三种数据库后端。\n\n## 要点\n\n- 全支持".to_string();
        let deps = deps_with(vec![page, topics.to_string()]).await;
        let (kb_id, doc_id) = seed_kb_and_doc(&deps).await;
        let created = distill_documents(&deps, kb_id, &[doc_id], "default")
            .await
            .unwrap();
        let page_id = created[0];

        publish_page(&deps, page_id, SnowflakeId(1), "default")
            .await
            .unwrap();
        let p = models::wiki_page::find_page_by_id(&deps.pool, page_id, "default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.status, "published");
        assert_eq!(p.current_revision, 1);

        // wiki units indexed (vector + fts)
        let units = models::chunk::find_chunks_by_page(&deps.pool, page_id)
            .await
            .unwrap();
        assert!(!units.is_empty());
        assert!(
            units
                .iter()
                .all(|u| u.kind == "wiki_page" && u.embedding.is_some())
        );
        let hits = deps
            .kbsearch
            .search(i64::from(kb_id), "数据库", 10)
            .await
            .unwrap();
        assert!(!hits.is_empty());
        let probe = {
            let mut v = vec![0.0_f32; 4];
            v[0] = 1.0;
            v
        };
        let vhits = deps
            .vector
            .search(i64::from(kb_id), &probe, 10, Some("wiki_page"))
            .await
            .unwrap();
        assert!(!vhits.is_empty(), "vector must serve wiki_page kind filter");
    }

    #[tokio::test]
    async fn reprocess_marks_published_page_stale() {
        let topics = r#"{"topics":[{"name":"数据库","slug":"topic/db","description":"db"}]}"#;
        let page = "## 概述\n\n内容。\n\n## 要点\n\n- x".to_string();
        let deps = deps_with(vec![page, topics.to_string()]).await;
        let (kb_id, doc_id) = seed_kb_and_doc(&deps).await;
        let created = distill_documents(&deps, kb_id, &[doc_id], "default")
            .await
            .unwrap();
        publish_page(&deps, created[0], SnowflakeId(1), "default")
            .await
            .unwrap();

        // Source doc re-parsed → page must go stale (§7 incremental rule).
        crate::kb::service::process_document(&deps, doc_id, "default")
            .await
            .unwrap();
        let p = models::wiki_page::find_page_by_id(&deps.pool, created[0], "default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(p.status, "stale");
    }

    #[test]
    fn line_diff_marks_changes() {
        let lines = line_diff("a\nb\n", "a\nc\n");
        assert!(lines.iter().any(|l| l.tag == "equal" && l.text == "a"));
        assert!(lines.iter().any(|l| l.tag == "delete" && l.text == "b"));
        assert!(lines.iter().any(|l| l.tag == "insert" && l.text == "c"));
    }
}
