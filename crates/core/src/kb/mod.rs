//! Knowledge base (KB) subsystem — self-contained vertical module.
//!
//! Layout mirrors `src/agent/` (handler/service/models inside the module, a
//! thin worker bridge under `worker/handlers/`); layering discipline
//! (Handler → Service → Model) is preserved within the module.
//!
//! Blueprints and decisions: `dev-docs/rag/knowledge-base-design.md`
//! (macro) and `dev-docs/rag/kb-technical-design.md` (technical). Every
//! mechanism carries a `[抄WK:…]` / `[抄RF:…]` / `[抄EXT:…]` reference there.
//!
//! Milestones (technical design §13): M1 vector foundation → M2 document
//! ingestion → M3 QA pipeline → M4 wiki distillation → M5 FAQ & feedback
//! loop → M6 admin frontend.

pub mod chunker;
pub mod diagnostics;
pub mod distill;
pub mod eval;
pub mod rerank;

/// Runtime singletons shared by handlers and workers (built once at boot,
/// `None` when the KB is disabled).
pub struct KbRuntime {
    pub vector: std::sync::Arc<dyn vectors::VectorIndex>,
    pub kbsearch: std::sync::Arc<kbsearch::KbSearchEngine>,
    pub embedder: std::sync::Arc<dyn service::KbEmbedder>,
    /// S5 reranker; `None` when no rerank model is configured (§6.1).
    pub reranker: Option<std::sync::Arc<dyn rerank::KbReranker>>,
    /// Parser engine registry.
    pub parsers: std::sync::Arc<crate::docparse::ParserRegistry>,
}

/// Build the KB runtime from config; `Ok(None)` when the KB is disabled,
/// loud error when enabled-but-misconfigured (D6).
pub fn build_kb_runtime(
    config: &crate::config::app::AppConfig,
    router: std::sync::Arc<crate::llm::service::LlmRouter>,
) -> crate::errors::app_error::AppResult<Option<std::sync::Arc<KbRuntime>>> {
    if !config.kb.enabled {
        return Ok(None);
    }
    vectors::validate_kb_config(config)?;
    let vector = vectors::build_vector_index(config)?;
    let kbsearch_dir = std::path::Path::new(&config.storage_root_dir).join("kb_search_index");
    let kbsearch = kbsearch::KbSearchEngine::open(&kbsearch_dir)?;
    let embedder: std::sync::Arc<dyn service::KbEmbedder> = std::sync::Arc::new(
        service::ProviderEmbedder::new(router.clone(), config.kb.embed_batch_size),
    );
    // Rerank support is always wired; enablement is resolved per query from
    // the KB rows (per-KB override → global RAISFAST_KB_RERANK_MODEL
    // default → passthrough, §6.1.2).
    let reranker: std::sync::Arc<dyn rerank::KbReranker> = std::sync::Arc::new(
        rerank::ProviderReranker::new(router.clone(), config.kb.rerank_batch_size),
    );
    if let Some(model) = config.kb.rerank_model.as_deref() {
        tracing::info!("kb rerank default model: '{model}' (per-KB rows may override)");
    }
    // Parser registry: builtin always; docreader when its endpoint is
    // configured (kb-parser-engines-design §2 D3).
    let mut engines: Vec<std::sync::Arc<dyn crate::docparse::ParseEngine>> = Vec::new();
    if let Some(docreader) = crate::docparse::docreader::DocreaderEngine::from_config(&config.kb) {
        tracing::info!(
            "kb parse engine 'docreader' configured ({})",
            config.kb.docreader_url.as_deref().unwrap_or_default()
        );
        engines.push(std::sync::Arc::new(docreader));
    }
    if let Some(mineru) = crate::docparse::mineru::MineruEngine::from_config(&config.kb) {
        tracing::info!(
            "kb parse engine 'mineru' configured ({})",
            config.kb.mineru_url.as_deref().unwrap_or_default()
        );
        engines.push(std::sync::Arc::new(mineru));
    }
    if let Some(engine) = crate::docparse::mineru_cloud::MineruCloudEngine::from_config(&config.kb)
    {
        tracing::info!("kb parse engine 'mineru_cloud' configured (api key set)");
        engines.push(std::sync::Arc::new(engine));
    }
    if let Some(engine) = crate::docparse::paddleocr_vl::PaddleOcrVlEngine::from_config(&config.kb)
    {
        tracing::info!(
            "kb parse engine 'paddleocr_vl' configured ({})",
            config
                .kb
                .paddleocr_vl_endpoint
                .as_deref()
                .unwrap_or_default()
        );
        engines.push(std::sync::Arc::new(engine));
    }
    if let Some(engine) =
        crate::docparse::paddleocr_vl_cloud::PaddleOcrVlCloudEngine::from_config(&config.kb)
    {
        tracing::info!("kb parse engine 'paddleocr_vl_cloud' configured (token set)");
        engines.push(std::sync::Arc::new(engine));
    }
    let parsers = std::sync::Arc::new(crate::docparse::ParserRegistry::new(engines));
    Ok(Some(std::sync::Arc::new(KbRuntime {
        vector,
        kbsearch: std::sync::Arc::new(kbsearch),
        embedder,
        reranker: Some(reranker),
        parsers,
    })))
}
pub mod handler;
pub mod images;
pub mod kbsearch;
pub mod models;
pub mod parse_quality;
pub mod pipeline;
pub mod service;
pub mod trace;
pub mod vectors;
