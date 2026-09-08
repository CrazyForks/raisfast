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

const EXTRACT_PROMPT: &str = "你是知识抽取系统。分析文档内容，抽取重要实体与关键概念。\
输出 JSON：{\"topics\":[{\"name\":\"名称\",\"slug\":\"topic/小写连字符标识\",\
\"description\":\"一句话说明（15-40字）\"}]}。\
规则：slug 稳定——若之前列表中存在相同主题，必须复用其原 slug；文档中已不存在的主题不要输出；\
不要发明文档中没有的主题；全部用中文书写。只输出 JSON。";

const DRAFT_PROMPT: &str = "你是维基百科编辑。根据给定的资料分块，为主题撰写一篇结构化 Markdown 知识页。\
规则：\
1. 使用正确的标题层级（## 二级、### 三级）；\
2. 可用页面清单列出了本库其他页面，凡提到清单中的主题必须写成 [[slug|显示名]] 形式的互链，\
不得使用粗体或裸文本；只能使用清单中给出的 slug，不得杜撰；\
3. 只依据给定资料撰写，不得编造；资料不足以成文时输出空字符串；\
4. 末尾加 \"## 要点\" 小节；全文 300-800 字。只输出 Markdown 正文。";

/// One extracted topic.
#[derive(Debug, Clone, serde::Deserialize)]
struct Topic {
    name: String,
    slug: String,
    #[serde(default)]
    description: String,
}

async fn chat_json(deps: &KbDeps, system: &str, user: &str) -> AppResult<String> {
    let provider = deps
        .provider
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("kb chat provider unavailable".into()))?;
    let model = deps.config.ai.model.as_deref().unwrap_or_default();
    let messages = vec![
        ChatMessage {
            role: ChatRole::System,
            content: Some(system.to_string()),
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: ChatRole::User,
            content: Some(user.to_string()),
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
    let response = provider
        .chat(&request, model)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("distill chat: {e}")))?;
    Ok(response.text.unwrap_or_default())
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
    let existing = models::wiki_page::list_pages(&deps.pool, kb_id, None, 1, 500, tenant_id)
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
        let reply = chat_json(deps, EXTRACT_PROMPT, &user).await?;
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
    // declared reduction of WK's citation pass).
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
        let content = chat_json(deps, DRAFT_PROMPT, &user)
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
        let page = models::wiki_page::create_page(
            &deps.pool,
            &models::wiki_page::CreateWikiPageCmd {
                kb_id,
                title: topic.name.clone(),
                slug: topic.slug.clone(),
                content,
                summary: Some(topic.description.clone()),
                linked_page_ids: Some(serde_json::json!(linked)),
                created_by: None,
            },
            tenant_id,
        )
        .await?;
        for doc_id in source_docs {
            models::wiki_source::link_source(&deps.pool, page.id, 0, doc_id).await?;
        }
        created.push(page.id);
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
    // D2: snapshot BEFORE the publish bump (old published content, if any).
    if page.current_revision > 1 {
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
    let vectors = deps.embedder.embed(&texts).await?;
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
        async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
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
            provider: Some(Arc::new(ScriptedProvider {
                replies: Mutex::new(replies),
            })),
            emitter: crate::event::EventEmitter::eventbus_only(bus),
        }
    }

    async fn seed_kb_and_doc(deps: &KbDeps) -> (SnowflakeId, SnowflakeId) {
        let kb = models::knowledge_base::create_kb(
            &deps.pool,
            &models::knowledge_base::CreateKbCmd {
                name: "kb".into(),
                slug: "kb".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();
        let markdown = "# 数据库\n\nraisfast 支持 SQLite PostgreSQL MySQL。向量检索由 Qdrant 提供。"
            .to_string() + &"配置详见环境变量章节。".repeat(40);
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
            models::wiki_page::list_pages(&deps.pool, kb_id, Some("draft"), 1, 10, "default")
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
