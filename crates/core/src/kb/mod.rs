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
pub mod distill;
pub mod eval;

/// Runtime singletons shared by handlers and workers (built once at boot,
/// `None` when the KB is disabled).
pub struct KbRuntime {
    pub vector: std::sync::Arc<dyn vectors::VectorIndex>,
    pub kbsearch: std::sync::Arc<kbsearch::KbSearchEngine>,
    pub embedder: std::sync::Arc<dyn service::KbEmbedder>,
    /// Chat provider for S1 query understanding and S9 answer generation.
    pub provider: std::sync::Arc<dyn raisfast_agent::ModelProvider>,
}

/// Build the KB runtime from config; `Ok(None)` when the KB is disabled,
/// loud error when enabled-but-misconfigured (D6).
pub fn build_kb_runtime(
    config: &crate::config::app::AppConfig,
) -> crate::errors::app_error::AppResult<Option<std::sync::Arc<KbRuntime>>> {
    if !config.kb.enabled {
        return Ok(None);
    }
    vectors::validate_kb_config(config)?;
    let vector = vectors::build_vector_index(config)?;
    let kbsearch_dir = std::path::Path::new(&config.storage_root_dir).join("kb_search_index");
    let kbsearch = kbsearch::KbSearchEngine::open(&kbsearch_dir)?;
    let embedder: std::sync::Arc<dyn service::KbEmbedder> =
        std::sync::Arc::new(service::ProviderEmbedder::new(config)?);
    let provider = crate::agent::service::provider_from_config(&config.ai)?;
    Ok(Some(std::sync::Arc::new(KbRuntime {
        vector,
        kbsearch: std::sync::Arc::new(kbsearch),
        embedder,
        provider,
    })))
}
pub mod handler;
pub mod kbsearch;
pub mod models;
pub mod pipeline;
pub mod service;
pub mod vectors;
