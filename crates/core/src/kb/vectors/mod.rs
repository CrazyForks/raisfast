//! Vector index abstraction — the pluggable backend seam of the KB
//! subsystem (technical design §4.2).
//!
//! Contract: SQL is always the source of truth for embeddings (BLOB columns
//! on `kb_chunks`); a `VectorIndex` is a rebuildable acceleration layer.
//! Swapping or losing a backend never migrates data — `rebuild` restores
//! the index from SQL (job `KbRebuildVectorIndex`, technical design §5).
//!
//! Backends:
//! - `QdrantIndex` (default, feature `kb-qdrant`): external Qdrant service,
//!   one collection per KB (`{prefix}_{kb_id}`) [抄EXT:qdrant]
//! - `BruteForce`: in-memory cosine scan, zero-dependency escape hatch and
//!   test backend (technical design §4.2)

#[cfg(feature = "kb-qdrant")]
mod qdrant;
#[cfg(feature = "kb-qdrant")]
pub use self::qdrant::QdrantIndex;

mod bruteforce;
pub use self::bruteforce::BruteForceIndex;

use crate::config::app::AppConfig;
use crate::errors::app_error::{AppError, AppResult};

/// One embeddable retrieval unit to index.
///
/// `unit_id` is the `kb_chunks.id` snowflake; `kind` mirrors the chunk kind
/// (`document` | `faq` | `wiki_page`) and doubles as a payload filter for
/// future scoped searches [抄WK:types/chunk.go ChunkType*].
#[derive(Debug, Clone)]
pub struct VectorItem {
    pub unit_id: i64,
    pub kb_id: i64,
    pub kind: String,
    pub embedding: Vec<f32>,
}

/// A search hit: the unit id plus its similarity score.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VectorHit {
    pub unit_id: i64,
    pub score: f32,
}

/// Vector index backend interface [抄RF:search.rs SearchEngine trait 同构 +
/// 抄WK:types/interfaces/vectorstore.go 多后端思想].
#[async_trait::async_trait]
pub trait VectorIndex: Send + Sync {
    /// Insert or update vectors under one KB. Creates the KB's collection
    /// on first use when the backend is collection-based.
    async fn upsert(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()>;

    /// Remove specific units from one KB.
    async fn delete(&self, kb_id: i64, unit_ids: &[i64]) -> AppResult<()>;

    /// Wipe one KB's entire index (used by re-parse idempotency and rebuild).
    async fn delete_all(&self, kb_id: i64) -> AppResult<()>;

    /// Cosine-similarity search within one KB, optionally filtered by kind.
    async fn search(
        &self,
        kb_id: i64,
        embedding: &[f32],
        top_k: usize,
        kind: Option<&str>,
    ) -> AppResult<Vec<VectorHit>>;

    /// Full re-index of one KB from the SQL source of truth.
    async fn rebuild(&self, kb_id: i64, dim: u32, items: &[VectorItem]) -> AppResult<()>;

    /// Human-readable backend name for health reporting and admin UI.
    fn backend_name(&self) -> &str;
}

/// D6 startup validation (technical design §4.1/§11): with the KB enabled,
/// the embedding model/dim must be explicit and the selected backend's
/// prerequisites present. Fails loudly instead of silently indexing with a
/// wrong dimension.
pub fn validate_kb_config(config: &AppConfig) -> AppResult<()> {
    if !config.kb.enabled {
        return Ok(());
    }
    if config.ai.embedding_model.is_none() || config.ai.embedding_dim.is_none() {
        return Err(AppError::BadRequest(
            "RAISFAST_KB_ENABLED=true requires RAISFAST_AI_EMBEDDING_MODEL and \
             RAISFAST_AI_EMBEDDING_DIM to be set explicitly (D6, kb-technical-design §4.1)"
                .to_string(),
        ));
    }
    if !config.ai.enabled || config.ai.model.as_deref().unwrap_or_default().is_empty() {
        return Err(AppError::BadRequest(
            "RAISFAST_KB_ENABLED=true requires RAISFAST_AI_ENABLED=true and RAISFAST_AI_MODEL \
             (the QA pipeline generates answers via the chat provider)"
                .to_string(),
        ));
    }
    match config.kb.vector_backend.as_str() {
        "qdrant" => {
            if config.kb.qdrant_url.is_none() {
                return Err(AppError::BadRequest(
                    "kb vector backend 'qdrant' requires RAISFAST_KB_QDRANT_URL \
                     (or switch backend via RAISFAST_KB_VECTOR_BACKEND=bruteforce)"
                        .to_string(),
                ));
            }
        }
        "bruteforce" => {}
        other => {
            return Err(AppError::BadRequest(format!(
                "unknown RAISFAST_KB_VECTOR_BACKEND '{other}' (expected qdrant|bruteforce)"
            )));
        }
    }
    Ok(())
}

/// Build the configured vector index backend. Call [`validate_kb_config`]
/// first; this factory also validates defensively.
pub fn build_vector_index(config: &AppConfig) -> AppResult<std::sync::Arc<dyn VectorIndex>> {
    validate_kb_config(config)?;
    match config.kb.vector_backend.as_str() {
        #[cfg(feature = "kb-qdrant")]
        "qdrant" => {
            let url = config.kb.qdrant_url.clone().unwrap_or_default();
            let index = QdrantIndex::new(
                &url,
                config.kb.qdrant_api_key.clone(),
                &config.kb.qdrant_prefix,
            )
            .map_err(|e| AppError::ServiceUnavailable(format!("qdrant connect: {e}")))?;
            tracing::info!("kb vector backend: qdrant ({url})");
            Ok(std::sync::Arc::new(index))
        }
        #[cfg(not(feature = "kb-qdrant"))]
        "qdrant" => Err(AppError::ServiceUnavailable(
            "kb vector backend 'qdrant' requires building with feature 'kb-qdrant'".to_string(),
        )),
        "bruteforce" => {
            tracing::info!("kb vector backend: bruteforce (in-memory cosine)");
            Ok(std::sync::Arc::new(BruteForceIndex::new()))
        }
        other => Err(AppError::BadRequest(format!(
            "unknown RAISFAST_KB_VECTOR_BACKEND '{other}'"
        ))),
    }
}
