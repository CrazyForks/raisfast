//! KB 子系统场景级集成测试（独立于 src 内单元测试）。
//!
//! 覆盖矩阵（kb-technical-design §5–§9）：摄入与校验、状态机与幂等、
//! 多 KB 隔离、检索管线（双路召回/融合/FAQ 注入/父子扩展/未覆盖兜底/
//! 引用对齐）、wiki 蒸馏-发布-编辑-失效、FAQ 入池撤池、反馈闭环、
//! 向量后端语义。运行：`just test`（sqlite 并行路径）。

use std::sync::Arc;

use raisfast::kb::models;
use raisfast::kb::models::knowledge_base::CreateKbCmd;
use raisfast::kb::models::knowledge_base::UpdateKbCmd;
use raisfast::kb::pipeline::{self, AskRequest};
use raisfast::kb::service::{self, KbDeps, KbEmbedder};
use raisfast::kb::vectors::{BruteForceIndex, VectorIndex};
use raisfast::types::snowflake_id::SnowflakeId;
use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

// ── 测试基建 ───────────────────────────────────────────────────────

/// 确定性 embedder：文本字节和 → one-hot。
struct SumEmbedder(usize);

#[async_trait::async_trait]
impl KbEmbedder for SumEmbedder {
    async fn embed(&self, texts: &[&str]) -> raisfast::errors::app_error::AppResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| {
                let mut v = vec![0.0_f32; self.0];
                v[t.bytes().map(|b| b as usize).sum::<usize>() % self.0] = 1.0;
                v
            })
            .collect())
    }
}

/// 脚本化 chat provider：按序弹出固定回复。
struct ScriptedProvider(std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl ModelProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn chat(&self, _r: &ChatRequest<'_>, _m: &str) -> Result<ChatResponse, ProviderError> {
        let reply = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop()
            .unwrap_or_default();
        Ok(ChatResponse::text_only(reply))
    }
}

/// 声明式回答 provider：回复嵌入知识的固定模板。
struct CiteProvider;

#[async_trait::async_trait]
impl ModelProvider for CiteProvider {
    fn name(&self) -> &str {
        "cite"
    }

    async fn chat(&self, r: &ChatRequest<'_>, _m: &str) -> Result<ChatResponse, ProviderError> {
        // 回显知识块里出现的第一个标题，证明模型确实看到了装配上下文。
        let user = r
            .messages
            .last()
            .map(|m| m.content.clone().unwrap_or_default())
            .unwrap_or_default();
        let title = user
            .lines()
            .find(|l| l.starts_with("[1]"))
            .map(|l| l.to_string())
            .unwrap_or_else(|| "[1] 未知".into());
        Ok(ChatResponse::text_only(format!("{title} —— 已作答。")))
    }
}

async fn deps() -> KbDeps {
    deps_with_provider(Arc::new(CiteProvider)).await
}

async fn deps_with_provider(provider: Arc<dyn ModelProvider>) -> KbDeps {
    let pool = raisfast::test_pool!();
    let mut config = raisfast::config::app::AppConfig::test_defaults();
    config.kb.enabled = true;
    config.kb.fallback_threshold = 0.05;
    KbDeps {
        pool,
        config: Arc::new(config),
        storage: Arc::new(
            raisfast::storage::local::LocalStorage::new("/tmp/kb-it-uploads", "/uploads").unwrap(),
        ),
        vector: Arc::new(BruteForceIndex::new()),
        kbsearch: Arc::new(raisfast::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
        embedder: Arc::new(SumEmbedder(4)),
        provider: Some(provider),
        emitter: raisfast::event::EventEmitter::eventbus_only(raisfast::eventbus::EventBus::new(
            16,
        )),
    }
}

async fn seed_kb(deps: &KbDeps, name: &str) -> SnowflakeId {
    models::knowledge_base::create_kb(
        &deps.pool,
        &CreateKbCmd {
            name: name.into(),
            description: None,
            slug: format!("{}-{}", name, raisfast::utils::id::new_id()),
            kind: "document".into(),
            indexing_strategy: None,
            embedding_model: Some("test-model".into()),
            embedding_dim: Some(4),
        },
        "default",
    )
    .await
    .unwrap()
    .id
}

async fn ingest_md(deps: &KbDeps, kb: SnowflakeId, title: &str, md: &str) -> SnowflakeId {
    let doc = service::create_online_document(deps, kb, title, md, None, "default")
        .await
        .unwrap();
    service::process_document(deps, doc.id, "default")
        .await
        .unwrap();
    doc.id
}

fn long_body(seed: &str, n: usize) -> String {
    seed.repeat(n)
}

// ── 场景：摄入与校验 ──────────────────────────────────────────────

#[tokio::test]
async fn s01_upload_rejects_bad_inputs() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    // 未知扩展名 + octet-stream → 拒绝
    assert!(
        service::upload_document(
            &deps,
            kb,
            "evil.exe",
            "application/x-msdownload",
            b"MZ",
            None,
            "default"
        )
        .await
        .is_err()
    );
    // 空文件 → 拒绝
    assert!(
        service::upload_document(&deps, kb, "a.md", "text/markdown", b"", None, "default")
            .await
            .is_err()
    );
    // 合法 markdown → 通过
    assert!(
        service::upload_document(
            &deps,
            kb,
            "a.md",
            "text/markdown",
            b"# T\n\nbody",
            None,
            "default"
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn s02_status_machine_and_idempotent_reprocess() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = service::create_online_document(
        &deps,
        kb,
        "状态机",
        &format!("# 状态机\n\n{}", long_body("安装配置部署三步走。", 80)),
        None,
        "default",
    )
    .await
    .unwrap();
    assert_eq!(doc.status, "pending");

    service::process_document(&deps, doc.id, "default")
        .await
        .unwrap();
    let doc = models::document::find_document_by_id(&deps.pool, doc.id, "default")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(doc.status, "ready");
    assert!(doc.chunk_count > 0);

    // steps 计时：每个流水线步骤都有 start+end（indexing 含在内）
    let steps = doc.steps.as_ref().expect("steps map persisted");
    for step in models::document::PIPELINE_STEPS {
        let s = steps
            .get(step)
            .unwrap_or_else(|| panic!("step '{step}' recorded"));
        assert!(s.get("start").is_some(), "{step} has start");
        assert!(s.get("end").is_some(), "{step} has end (closed on ready)");
    }

    // 幂等重跑：chunk 数稳定、无重复
    let before = models::chunk::find_chunks_by_doc(&deps.pool, doc.id)
        .await
        .unwrap()
        .len();
    service::process_document(&deps, doc.id, "default")
        .await
        .unwrap();
    let after = models::chunk::find_chunks_by_doc(&deps.pool, doc.id)
        .await
        .unwrap()
        .len();
    assert_eq!(before, after);
}

#[tokio::test]
async fn s03_delete_cleans_all_three_stores() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = ingest_md(
        &deps,
        kb,
        "删除",
        &format!("# 删除\n\n{}", long_body("唯一内容 deletable。", 60)),
    )
    .await;

    assert!(
        !deps
            .kbsearch
            .search(i64::from(kb), "deletable", 10)
            .await
            .unwrap()
            .is_empty()
    );
    // 原始字节必须真实存在过（online 文档也落 storage），删除后才谈得上清理。
    let row = models::document::find_document_by_id(&deps.pool, doc, "default")
        .await
        .unwrap()
        .unwrap();
    let key = row.storage_key.clone().expect("online doc has storage key");
    assert!(
        tokio::fs::try_exists(format!("/tmp/kb-it-uploads/{key}")).await.unwrap(),
        "storage file must exist before delete"
    );
    service::delete_document_everywhere(&deps, doc, "default")
        .await
        .unwrap();
    assert!(
        !tokio::fs::try_exists(format!("/tmp/kb-it-uploads/{key}")).await.unwrap(),
        "storage file must be removed with the document"
    );
    assert!(
        models::chunk::find_chunks_by_doc(&deps.pool, doc)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        deps.kbsearch
            .search(i64::from(kb), "deletable", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        models::document::find_document_by_id(&deps.pool, doc, "default")
            .await
            .unwrap()
            .is_none()
    );
}

// ── 场景：检索管线 ────────────────────────────────────────────────

#[tokio::test]
async fn s04_ask_answered_with_aligned_citations() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    ingest_md(
        &deps,
        kb,
        "数据库",
        &format!(
            "# 数据库\n\n{}",
            long_body("支持 PostgreSQL 后端连接。", 60)
        ),
    )
    .await;

    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "支持什么数据库".into(),
    };
    let mut out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    pipeline::finish_answer(&deps, &mut out).await.unwrap();
    assert_eq!(out.status, "answered");
    assert!(!out.answer.is_empty());
    // 引用编号连续且对应注入了模型的单元
    assert!(!out.references.is_empty());
    for (i, r) in out.references.iter().enumerate() {
        assert_eq!(r.n, i + 1);
    }
}

#[tokio::test]
async fn s05_uncovered_kb_never_generates() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "空库").await;
    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "任意问题".into(),
    };
    let mut out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    pipeline::finish_answer(&deps, &mut out).await.unwrap();
    assert_eq!(out.status, "uncovered");
    assert_eq!(out.answer, "知识库未覆盖该问题。");
    assert!(out.references.is_empty());
}

#[tokio::test]
async fn s06_multi_kb_isolation() {
    let deps = deps().await;
    let kb_a = seed_kb(&deps, "a").await;
    let kb_b = seed_kb(&deps, "b").await;
    ingest_md(
        &deps,
        kb_a,
        "甲",
        &format!("# 甲\n\n{}", long_body("甲库专属关键词 pineapple。", 60)),
    )
    .await;
    ingest_md(
        &deps,
        kb_b,
        "乙",
        &format!("# 乙\n\n{}", long_body("乙库专属关键词 durian。", 60)),
    )
    .await;

    let ask_a = AskRequest {
        kb_ids: vec![i64::from(kb_a)],
        doc_ids: Vec::new(),
        question: "pineapple 是什么".into(),
    };
    let out_a = pipeline::prepare_answer(&deps, &ask_a).await.unwrap();
    assert!(
        out_a
            .context_units
            .iter()
            .all(|u| !u.content.contains("durian")),
        "scoped ask must not leak the other KB"
    );

    let ask_all = AskRequest {
        kb_ids: vec![],
        doc_ids: Vec::new(),
        question: "durian 是什么".into(),
    };
    let out_all = pipeline::prepare_answer(&deps, &ask_all).await.unwrap();
    assert!(
        out_all
            .context_units
            .iter()
            .any(|u| u.content.contains("durian")),
        "empty kb_ids = all enabled KBs"
    );
}

#[tokio::test]
async fn s07_understanding_degrades_to_raw_question() {
    // provider 返回垃圾 → S1 优雅降级，检索仍用原问题命中
    let deps = deps_with_provider(Arc::new(ScriptedProvider(std::sync::Mutex::new(vec![
        "这不是 JSON".to_string(),
        "[1]（document）# 数据库\n支持 PostgreSQL。\n\n —— 已作答。".to_string(),
    ]))))
    .await;
    let kb = seed_kb(&deps, "kb").await;
    ingest_md(
        &deps,
        kb,
        "数据库",
        &format!("# 数据库\n\n{}", long_body("支持 PostgreSQL 后端。", 60)),
    )
    .await;

    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "支持什么数据库".into(),
    };
    let mut out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    pipeline::finish_answer(&deps, &mut out).await.unwrap();
    assert_eq!(
        out.status, "answered",
        "degraded S1 must not break the pipeline"
    );
}

// ── 场景：FAQ ─────────────────────────────────────────────────────

#[tokio::test]
async fn s08_faq_lifecycle_and_pinned_context() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    ingest_md(
        &deps,
        kb,
        "文档",
        &format!("# 文档\n\n{}", long_body("背景资料若干。", 60)),
    )
    .await;

    let faq = models::faq::create_faq(
        &deps.pool,
        &models::faq::CreateFaqCmd {
            kb_id: kb,
            standard_question: "如何重置密码".into(),
            similar_questions: vec!["忘记密码".into()],
            answers: vec!["点击登录页的忘记密码。".into()],
            enabled: true,
            created_by: None,
        },
        "default",
    )
    .await
    .unwrap();
    service::index_faq(&deps, &faq).await.unwrap();

    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "如何重置密码".into(),
    };
    let mut out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    pipeline::finish_answer(&deps, &mut out).await.unwrap();
    let first = out.context_units.first().expect("context units");
    assert!(first.is_faq, "FAQ must be pinned at the top of the context");
    assert!(first.content.contains("Q: 如何重置密码"));

    // 撤池：disable 后不再进入检索
    service::deindex_faq(&deps, faq.id, kb).await.unwrap();
    let ask2 = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "如何重置密码".into(),
    };
    let out2 = pipeline::prepare_answer(&deps, &ask2).await.unwrap();
    assert!(
        out2.context_units.iter().all(|u| !u.is_faq),
        "withdrawn FAQ must leave retrieval"
    );
}

// ── 场景：wiki 蒸馏与治理 ─────────────────────────────────────────

fn wiki_scripts() -> Vec<String> {
    // ScriptedProvider 按栈序弹出：先弹①抽取 JSON，再弹②各主题草稿。
    vec![
        // pass ② 第二个主题（数据库）的页面草稿
        "## 支持列表\n\nSQLite、Qdrant。\n\n## 要点\n\n- 稳".to_string(),
        // pass ② 第一个主题（向量检索）的页面草稿（含互链）
        "## 概述\n\n检索基于 [[topic/shu-ju-ku|数据库]]。\n\n## 要点\n\n- 快".to_string(),
        // pass ① 主题抽取 JSON
        r#"{"topics":[{"name":"向量检索","slug":"topic/xiang-liang","description":"检索说明"},{"name":"数据库","slug":"topic/shu-ju-ku","description":"库说明"}]}"#
            .to_string(),
    ]
}

#[tokio::test]
async fn s09_wiki_distill_publish_boost() {
    let deps = deps_with_provider(Arc::new(ScriptedProvider(std::sync::Mutex::new(
        wiki_scripts(),
    ))))
    .await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = ingest_md(
        &deps,
        kb,
        "数据库文档",
        &format!(
            "# 数据库\n\n{}",
            long_body("支持 SQLite 与 Qdrant 向量检索。", 60)
        ),
    )
    .await;

    let created = raisfast::kb::distill::distill_documents(&deps, kb, &[doc], "default")
        .await
        .unwrap();
    assert_eq!(created.len(), 2);
    let pages = models::wiki_page::list_pages(&deps.pool, kb, Some("draft"), 1, 10, "default")
        .await
        .unwrap()
        .0;
    assert_eq!(pages.len(), 2, "pages stay draft until human approval");

    // 发布 → wiki_page 单元入池 + 管线以 wiki 加权路径可命中
    raisfast::kb::distill::publish_page(&deps, created[0], SnowflakeId(1), "default")
        .await
        .unwrap();
    let units = models::chunk::find_chunks_by_page(&deps.pool, created[0])
        .await
        .unwrap();
    assert!(!units.is_empty());
    assert!(
        units
            .iter()
            .all(|u| u.kind == "wiki_page" && u.embedding.is_some())
    );
}

#[tokio::test]
async fn s10_source_change_marks_page_stale() {
    let deps = deps_with_provider(Arc::new(ScriptedProvider(std::sync::Mutex::new(
        wiki_scripts(),
    ))))
    .await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = ingest_md(
        &deps,
        kb,
        "文档",
        &format!("# 数据库\n\n{}", long_body("内容主体。", 60)),
    )
    .await;
    let created = raisfast::kb::distill::distill_documents(&deps, kb, &[doc], "default")
        .await
        .unwrap();
    raisfast::kb::distill::publish_page(&deps, created[0], SnowflakeId(1), "default")
        .await
        .unwrap();

    // 源文档重解析 → 信任链可见：页面 stale
    service::process_document(&deps, doc, "default")
        .await
        .unwrap();
    let page = models::wiki_page::find_page_by_id(&deps.pool, created[0], "default")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.status, "stale");

    // 源删除 → 同样失效
    service::delete_document_everywhere(&deps, doc, "default")
        .await
        .unwrap();
    let page = models::wiki_page::find_page_by_id(&deps.pool, created[0], "default")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.status, "stale");
}

#[tokio::test]
async fn s11_chunk_edit_reembeds_and_reindexes() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = ingest_md(
        &deps,
        kb,
        "编辑",
        &format!("# 编辑\n\n{}", long_body("旧内容 old-content。", 60)),
    )
    .await;

    let chunk = models::chunk::find_chunks_by_doc(&deps.pool, doc)
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    models::chunk::update_content(&deps.pool, chunk.id, "全新内容 brand-new-token")
        .await
        .unwrap();
    let vectors = deps
        .embedder
        .embed(&["全新内容 brand-new-token"])
        .await
        .unwrap();
    models::chunk::update_embedding(&deps.pool, chunk.id, &vectors[0])
        .await
        .unwrap();
    deps.vector
        .upsert(
            i64::from(kb),
            4,
            &[raisfast::kb::vectors::VectorItem {
                unit_id: i64::from(chunk.id),
                kb_id: i64::from(kb),
                kind: chunk.kind.clone(),
                embedding: vectors[0].clone(),
            }],
        )
        .await
        .unwrap();

    let probe = {
        let mut v = vec![0.0_f32; 4];
        v["全新内容 brand-new-token"
            .bytes()
            .map(|b| b as usize)
            .sum::<usize>()
            % 4] = 1.0;
        v
    };
    let hits = deps
        .vector
        .search(i64::from(kb), &probe, 10, None)
        .await
        .unwrap();
    assert!(
        hits.iter().any(|h| h.unit_id == i64::from(chunk.id)),
        "edited chunk must be re-embedded"
    );
}

// ── 场景：反馈闭环 ────────────────────────────────────────────────

#[tokio::test]
async fn s12_query_logging_and_gap_aggregation() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    for _ in 0..3 {
        models::query_log::insert_log(
            &deps.pool,
            Some(kb),
            "怎么开通多租户",
            Some("知识库未覆盖该问题。"),
            None,
            "uncovered",
            Some(0.0),
            None,
        )
        .await
        .unwrap();
    }
    models::query_log::insert_log(
        &deps.pool,
        Some(kb),
        "已答问题",
        Some("答案"),
        None,
        "answered",
        Some(0.9),
        None,
    )
    .await
    .unwrap();

    let gaps = models::query_log::list_gaps(&deps.pool, 10).await.unwrap();
    let hit = gaps
        .iter()
        .find(|(q, c, _)| q == "怎么开通多租户" && *c >= 3);
    assert!(
        hit.is_some(),
        "gaps must surface repeated uncovered queries: {gaps:?}"
    );
    assert!(
        !gaps.iter().any(|(q, _, _)| q == "已答问题"),
        "answered queries are not gaps"
    );
}

// ── 场景：向量后端语义 ────────────────────────────────────────────

#[tokio::test]
async fn s13_bruteforce_backend_semantics() {
    let idx = BruteForceIndex::new();
    let mk = |id: i64, kind: &str| raisfast::kb::vectors::VectorItem {
        unit_id: id,
        kb_id: 1,
        kind: kind.into(),
        embedding: vec![1.0, 0.0],
    };
    idx.upsert(1, 2, &[mk(1, "document"), mk(2, "wiki_page"), mk(3, "faq")])
        .await
        .unwrap();
    let wiki = idx
        .search(1, &[1.0, 0.0], 10, Some("wiki_page"))
        .await
        .unwrap();
    assert_eq!(wiki.len(), 1);
    assert_eq!(wiki[0].unit_id, 2, "kind filter must scope results");
    idx.delete_all(1).await.unwrap();
    assert!(
        idx.search(1, &[1.0, 0.0], 10, None)
            .await
            .unwrap()
            .is_empty()
    );
}

// ── 反馈补测：管线语义与治理不变量 ────────────────────────────────

#[tokio::test]
async fn s14_child_hit_expands_to_parent_content() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    // 长文档 → 必然产生父块(4096)+子块(384)；子块被命中时上下文必须是父块全文
    let md = format!(
        "# 长文\n\n{}",
        long_body("PARENT_SENTINEL_甲乙丙丁。", 1200)
    );
    ingest_md(&deps, kb, "长文", &md).await;
    // 用子块里的原文提问 → BM25 命中子块 → 装配上下文必须是父块（远大于 384 子块上限）
    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "PARENT_SENTINEL_甲乙丙丁".into(),
    };
    let out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    let doc_unit = out
        .context_units
        .iter()
        .find(|u| u.kind == "document")
        .expect("document unit");
    assert!(
        doc_unit.content.len() > 1000,
        "child hit must expand to the PARENT block, got {} chars",
        doc_unit.content.len()
    );
}

#[tokio::test]
async fn s15_wiki_boost_reorders_over_document() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    ingest_md(
        &deps,
        kb,
        "文档",
        &format!(
            "# 文档\n\n{}",
            long_body("共现关键词 zephyr 出现在文档里。", 60)
        ),
    )
    .await;
    // 已发布 wiki 页与文档同关键词
    let page = models::wiki_page::create_page(
        &deps.pool,
        &models::wiki_page::CreateWikiPageCmd {
            kb_id: kb,
            title: "知识页".into(),
            slug: "topic/zephyr".into(),
            content: format!(
                "## 概述\n\n{}",
                long_body("共现关键词 zephyr 的权威总结。", 60)
            ),
            summary: None,
            linked_page_ids: None,
            created_by: None,
        },
        "default",
    )
    .await
    .unwrap();
    raisfast::kb::distill::publish_page(&deps, page.id, SnowflakeId(1), "default")
        .await
        .unwrap();

    // 双路召回后同分并列时，wiki ×1.3 必须把 wiki 单元顶到最前
    let ask = AskRequest {
        kb_ids: vec![i64::from(kb)],
        doc_ids: Vec::new(),
        question: "zephyr".into(),
    };
    let out = pipeline::prepare_answer(&deps, &ask).await.unwrap();
    assert!(
        out.context_units
            .first()
            .is_some_and(|u| u.kind == "wiki_page"),
        "wiki boost must rank wiki_page first, got: {:?}",
        out.context_units
            .iter()
            .map(|u| u.kind.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn s16_republish_idempotent_and_snapshots_recorded() {
    let scripts = vec![
        "## 第二版\n\n更新内容。".to_string(),
        "## 概述\n\n第一版内容。".to_string(),
        r#"{"topics":[{"name":"主题","slug":"topic/t","description":"d"}]}"#.to_string(),
    ];
    let deps = deps_with_provider(Arc::new(ScriptedProvider(std::sync::Mutex::new(scripts)))).await;
    let kb = seed_kb(&deps, "kb").await;
    let doc = ingest_md(
        &deps,
        kb,
        "文档",
        &format!("# 主题\n\n{}", long_body("素材。", 60)),
    )
    .await;

    let created = raisfast::kb::distill::distill_documents(&deps, kb, &[doc], "default")
        .await
        .unwrap();
    let page_id = created[0];
    raisfast::kb::distill::publish_page(&deps, page_id, SnowflakeId(1), "default")
        .await
        .unwrap();
    let units_v1 = models::chunk::find_chunks_by_page(&deps.pool, page_id)
        .await
        .unwrap()
        .len();

    // 编辑 + 再发布：单元数不翻倍（幂等 wipe-then-insert）
    models::wiki_page::update_page_content(
        &deps.pool,
        page_id,
        "## 第二版\n\n更新内容。",
        None,
        "published",
        "default",
    )
    .await
    .unwrap();
    raisfast::kb::distill::publish_page(&deps, page_id, SnowflakeId(1), "default")
        .await
        .unwrap();
    let units_v2 = models::chunk::find_chunks_by_page(&deps.pool, page_id)
        .await
        .unwrap()
        .len();
    assert_eq!(units_v1, units_v2, "republish must not duplicate units");

    // 版本快照行存在且内容正确（恢复链可用）
    let page = models::wiki_page::find_page_by_id(&deps.pool, page_id, "default")
        .await
        .unwrap()
        .unwrap();
    assert!(page.current_revision >= 2, "republish must bump revision");
    let (_, total) = raisfast::services::content_revision::list_revisions(
        &deps.pool,
        "kb_wiki_page",
        page_id,
        1,
        10,
    )
    .await
    .unwrap();
    assert!(total >= 1, "snapshot rows must exist after republish");
}

#[tokio::test]
async fn s17_tenant_isolation() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    // 另一 tenant 查同 id → None
    let got = models::knowledge_base::find_kb_by_id(&deps.pool, kb, "other-tenant")
        .await
        .unwrap();
    assert!(got.is_none(), "cross-tenant kb read must be invisible");
    let doc = ingest_md(&deps, kb, "文档", "# 文\n\n内容").await;
    let got = models::document::find_document_by_id(&deps.pool, doc, "other-tenant")
        .await
        .unwrap();
    assert!(got.is_none(), "cross-tenant doc read must be invisible");
}

#[tokio::test]
async fn s18_update_kb_metadata_and_tenant_guard() {
    let deps = deps().await;
    let kb = seed_kb(&deps, "kb").await;
    // 可变字段更新：name/slug/description/status
    models::knowledge_base::update_kb(
        &deps.pool,
        kb,
        &UpdateKbCmd {
            name: "改名".into(),
            description: Some("新描述".into()),
            slug: format!("slug-{}", raisfast::utils::id::new_id()),
            status: "archived".into(),
        },
        "default",
    )
    .await
    .unwrap();
    let got = models::knowledge_base::find_kb_by_id(&deps.pool, kb, "default")
        .await
        .unwrap()
        .expect("kb must exist");
    assert_eq!(got.name, "改名");
    assert_eq!(got.description.as_deref(), Some("新描述"));
    assert_eq!(got.status, "archived");
    // 不可变字段未被触碰
    assert_eq!(got.kind, "document");
    assert_eq!(got.embedding_model.as_deref(), Some("test-model"));
    // 跨 tenant 更新无效果（0 行受影响）
    models::knowledge_base::update_kb(
        &deps.pool,
        kb,
        &UpdateKbCmd {
            name: "越权".into(),
            description: None,
            slug: "hijack".into(),
            status: "active".into(),
        },
        "other-tenant",
    )
    .await
    .unwrap();
    let got = models::knowledge_base::find_kb_by_id(&deps.pool, kb, "default")
        .await
        .unwrap()
        .expect("kb must exist");
    assert_eq!(got.name, "改名", "cross-tenant update must be a no-op");
}
