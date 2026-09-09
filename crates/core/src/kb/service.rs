//! KB service — document ingestion orchestration (kb-technical-design §5).
//!
//! Pipeline: upload/online → storage → `KbDocumentCreated` event →
//! `KbProcessDocument` job → anydoc parse → chunk (§3) → embed →
//! vector upsert + KB tantivy index → `KbDocumentReady`. Re-parse is
//! idempotent: delete-then-insert by doc in all three stores (SQL chunks,
//! vector backend, tantivy).

use std::sync::Arc;

use raisfast_agent::ModelProvider;

use crate::config::app::AppConfig;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::event::EventEmitter;
use crate::kb::chunker::{self, ChunkerConfig};
use crate::kb::kbsearch::{KbIndexUnit, KbSearchEngine};
use crate::kb::models::{chunk, document, faq, knowledge_base, wiki_page, wiki_source};
use crate::kb::vectors::{VectorIndex, VectorItem};
use crate::storage::Storage;
use crate::types::snowflake_id::SnowflakeId;

/// Embedding seam: production wraps the OpenAI-compatible provider
/// (`/embeddings`, §4.1); tests inject a deterministic stub.
#[async_trait::async_trait]
pub trait KbEmbedder: Send + Sync {
    async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>>;

    /// Per-KB embedding: the KB row pins its model + dim at creation
    /// (immutable). Production uses the KB's model; tests fall back to
    /// [`KbEmbedder::embed`].
    async fn embed_for(&self, _model: &str, dim: u32, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
        let out = self.embed(texts).await?;
        for v in &out {
            if v.len() as u32 != dim {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "embedding dim mismatch: kb pins {dim}, model returned {}",
                    v.len()
                )));
            }
        }
        Ok(out)
    }
}

/// Production embedder over `ModelProvider::embed` [抄EXT:OpenAI embeddings].
pub struct ProviderEmbedder {
    provider: Arc<dyn ModelProvider>,
    model: String,
    /// Texts per `/embeddings` request.
    batch_size: usize,
}

/// Retry shape [抄WK:keywords_vector_hybrid_indexer.go batchEmbedWithBackoff]:
/// 5 attempts, exponential backoff 200ms → 3.2s.
const EMBED_RETRY_ATTEMPTS: usize = 5;
const EMBED_RETRY_BASE_DELAY_MS: u64 = 200;

impl ProviderEmbedder {
    pub fn new(config: &AppConfig) -> AppResult<Self> {
        // Dedicated embedding provider when configured (chat on DeepSeek +
        // embeddings on Ollama); otherwise share the chat provider.
        let provider = match &config.ai.embedding_base_url {
            Some(base) => Arc::new(raisfast_agent::openai::OpenAiCompatProvider::new(
                base.clone(),
                config
                    .ai
                    .embedding_api_key
                    .clone()
                    .or_else(|| config.ai.api_key.clone()),
            )) as Arc<dyn ModelProvider>,
            None => crate::agent::service::provider_from_config(&config.ai)?,
        };
        Ok(Self {
            provider,
            model: config.ai.embedding_model.clone().unwrap_or_default(),
            batch_size: config.kb.embed_batch_size,
        })
    }

    /// Slice texts into fixed-size batches and embed them sequentially,
    /// retrying each batch with exponential backoff on failure
    /// [抄WK:models/embedding/batch.go BatchEmbedWithPool 语义，串行替代协程池].
    async fn embed_batched(&self, texts: &[&str], model: &str) -> AppResult<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(self.batch_size.max(1)) {
            let mut delay = EMBED_RETRY_BASE_DELAY_MS;
            let mut last_err: Option<String> = None;
            let mut vectors: Option<Vec<Vec<f32>>> = None;
            for attempt in 0..EMBED_RETRY_ATTEMPTS {
                match self.provider.embed(batch, model).await {
                    Ok(v) if v.len() == batch.len() => {
                        vectors = Some(v);
                        break;
                    }
                    Ok(v) => {
                        last_err = Some(format!(
                            "embedding count mismatch: batch {} got {}",
                            batch.len(),
                            v.len()
                        ));
                    }
                    Err(e) => {
                        last_err = Some(e.to_string());
                    }
                }
                tracing::warn!(
                    "embedding batch ({}/{} texts) attempt {}/{} failed: {}",
                    batch.len(),
                    texts.len(),
                    attempt + 1,
                    EMBED_RETRY_ATTEMPTS,
                    last_err.as_deref().unwrap_or_default()
                );
                if attempt + 1 < EMBED_RETRY_ATTEMPTS {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    delay *= 2;
                }
            }
            match vectors {
                Some(v) => out.extend(v),
                None => {
                    return Err(AppError::ServiceUnavailable(format!(
                        "embedding: {}",
                        last_err.unwrap_or_else(|| "unknown error".into())
                    )))
                }
            }
        }
        Ok(out)
    }
}

#[async_trait::async_trait]
impl KbEmbedder for ProviderEmbedder {
    async fn embed_for(&self, model: &str, dim: u32, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
        let out = self.embed_batched(texts, model).await?;
        for v in &out {
            if v.len() as u32 != dim {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "embedding dim mismatch: kb pins {dim}, model '{model}' returned {}",
                    v.len()
                )));
            }
        }
        Ok(out)
    }

    async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
        self.embed_batched(texts, &self.model).await
    }
}

/// Wired dependencies for service functions (built once at startup).
pub struct KbDeps {
    pub pool: crate::db::Pool,
    pub config: Arc<AppConfig>,
    pub storage: Arc<dyn Storage>,
    pub vector: Arc<dyn VectorIndex>,
    pub kbsearch: Arc<KbSearchEngine>,
    pub embedder: Arc<dyn KbEmbedder>,
    /// Chat provider (S1/S9); injected separately so tests can stub it.
    pub provider: Option<Arc<dyn raisfast_agent::ModelProvider>>,
    pub emitter: EventEmitter,
}

/// MIME whitelist for KB uploads — extends the media allowlist with
/// document formats [抄RF:services/media.rs save_file 校验流程].
pub const KB_ALLOWED_MIME: &[&str] = &[
    "text/markdown",
    "text/plain",
    "text/html",
    "text/csv",
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/epub+zip",
    "application/json",
    "application/rtf",
    "application/vnd.oasis.opendocument.text",
];

const MAX_DOC_BYTES: usize = 50 * 1024 * 1024;

/// Uploaded-file entry point: validate → store → insert pending row → event.
pub async fn upload_document(
    deps: &KbDeps,
    kb_id: SnowflakeId,
    filename: &str,
    mime_type: &str,
    data: &[u8],
    user_id: Option<SnowflakeId>,
    tenant_id: &str,
) -> AppResult<document::KbDocument> {
    ensure_kb(deps, kb_id, tenant_id).await?;
    let mime = normalize_mime(mime_type, filename);
    if !KB_ALLOWED_MIME.contains(&mime.as_str()) {
        return Err(AppError::BadRequest(format!(
            "unsupported kb document type: {mime}"
        )));
    }
    if data.is_empty() {
        return Err(AppError::BadRequest("empty file".into()));
    }
    if data.len() > MAX_DOC_BYTES {
        return Err(AppError::PayloadTooLarge("document exceeds 50MB".into()));
    }
    let ext = filename.rsplit('.').next().unwrap_or_default();
    let key = crate::services::media::storage_key("kb", ext);
    deps.storage.put(&key, data, &mime).await?;

    let doc = document::create_document(
        &deps.pool,
        &document::CreateKbDocumentCmd {
            kb_id,
            title: trim_filename(filename),
            source: "upload".into(),
            storage_key: Some(key),
            mime_type: Some(mime),
            size: data.len() as i64,
            created_by: user_id,
        },
        tenant_id,
    )
    .await?;
    deps.emitter.emit(Event::KbDocumentCreated(doc.clone()));
    Ok(doc)
}

/// Online markdown entry point (D7): same pipeline, parse step skipped by
/// treating the stored bytes as markdown.
pub async fn create_online_document(
    deps: &KbDeps,
    kb_id: SnowflakeId,
    title: &str,
    markdown: &str,
    user_id: Option<SnowflakeId>,
    tenant_id: &str,
) -> AppResult<document::KbDocument> {
    ensure_kb(deps, kb_id, tenant_id).await?;
    if markdown.trim().is_empty() {
        return Err(AppError::BadRequest("empty markdown".into()));
    }
    let data = markdown.as_bytes();
    let key = crate::services::media::storage_key("kb", "md");
    deps.storage.put(&key, data, "text/markdown").await?;

    let doc = document::create_document(
        &deps.pool,
        &document::CreateKbDocumentCmd {
            kb_id,
            title: title.to_string(),
            source: "online".into(),
            storage_key: Some(key),
            mime_type: Some("text/markdown".into()),
            size: data.len() as i64,
            created_by: user_id,
        },
        tenant_id,
    )
    .await?;
    deps.emitter.emit(Event::KbDocumentCreated(doc.clone()));
    Ok(doc)
}

/// The `KbProcessDocument` job body — the whole ingest pipeline.
/// Idempotent: safe to re-run at any failure point (delete-then-insert).
pub async fn process_document(
    deps: &KbDeps,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    let Some(doc) = document::find_document_by_id(&deps.pool, doc_id, tenant_id).await? else {
        return Ok(()); // deleted meanwhile — nothing to do
    };
    let Some(kb) = knowledge_base::find_kb_by_id(&deps.pool, doc.kb_id, tenant_id).await? else {
        return Ok(());
    };
    // Per-KB pinned model/dim (immutable since creation, D6-revised).
    let model = kb
        .embedding_model
        .clone()
        .filter(|m| !m.is_empty())
        .ok_or_else(|| {
            AppError::BadRequest("该知识库未配置嵌入模型（创建时必填，之后不可修改）".into())
        })?;
    let dim = u32::try_from(kb.embedding_dim.unwrap_or(0)).unwrap_or(0);
    if dim == 0 {
        return Err(AppError::BadRequest(
            "该知识库未配置向量维度（创建时必填，之后不可修改）".into(),
        ));
    }

    // ① parse → markdown [抄EXT:anydoc]
    document::set_document_status(&deps.pool, doc_id, "parsing", None, None, tenant_id).await?;
    let markdown = match &doc.storage_key {
        Some(key) => {
            let bytes = deps.storage.get(key).await?;
            if bytes.is_empty() {
                return Err(AppError::BadRequest(
                    "document bytes missing in storage".into(),
                ));
            }
            parse_to_markdown(&doc, &bytes)?
        }
        None => return Err(AppError::BadRequest("document has no storage key".into())),
    };

    // ② chunk (§3) + ③ persist chunks
    document::set_document_status(&deps.pool, doc_id, "chunking", None, None, tenant_id).await?;
    // idempotency: wipe this doc's chunks + vector units + fts units first
    chunk::delete_chunks_by_doc(&deps.pool, doc_id).await?;
    deps.kbsearch.delete_document(i64::from(doc_id)).await?;

    let cfg = ChunkerConfig::default();
    let raw_chunks = chunker::chunk_markdown(&markdown, &cfg);
    let now = crate::utils::tz::now_utc();
    let mut inserts = Vec::with_capacity(raw_chunks.len());
    for (i, c) in raw_chunks.iter().enumerate() {
        let id = crate::utils::id::new_snowflake_id();
        inserts.push(chunk::KbChunkInsert {
            id,
            kb_id: doc.kb_id,
            doc_id: Some(doc_id),
            faq_id: None,
            wiki_page_id: None,
            kind: "document".into(),
            parent_id: None, // patched below for children
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
    // link children to parents
    for (i, c) in raw_chunks.iter().enumerate() {
        if let Some(p) = c.parent
            && let Some(parent_insert) = inserts.get_mut(p)
        {
            inserts[i].parent_id = Some(parent_insert.id);
        }
    }
    for ins in &inserts {
        chunk::insert_chunk(&deps.pool, ins).await?;
    }

    // Leaf retrieval units: child chunks when a parent has children,
    // otherwise the childless parent itself (a lone small chunk).
    let has_children: std::collections::HashSet<usize> =
        raw_chunks.iter().filter_map(|c| c.parent).collect();
    let mut leaf_units: Vec<(i64, usize)> = Vec::new(); // (chunk_id, raw index)
    for (i, c) in raw_chunks.iter().enumerate() {
        if c.parent.is_some() || !has_children.contains(&i) {
            leaf_units.push((i64::from(inserts[i].id), i));
        }
    }

    // ④ embed children (parents ride along via expansion in the M3 pipeline)
    document::set_document_status(&deps.pool, doc_id, "embedding", None, None, tenant_id).await?;
    let embed_targets: Vec<(usize, String)> = leaf_units
        .iter()
        .map(|&(_, raw_i)| (raw_i, raw_chunks[raw_i].content.clone()))
        .collect();
    let texts: Vec<&str> = embed_targets.iter().map(|(_, t)| t.as_str()).collect();
    let vectors = deps.embedder.embed_for(&model, dim, &texts).await?;

    let mut items = Vec::with_capacity(embed_targets.len());
    for (idx, (raw_i, _)) in embed_targets.iter().enumerate() {
        let (chunk_id, _) = leaf_units
            .iter()
            .find(|&&(_, ri)| ri == *raw_i)
            .map(|&(cid, ri)| (cid, ri))
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("chunk id lookup failed")))?;
        // persist embedding BLOB (SQL is the truth, D3)
        raisfast_derive::crud_update!(
            &deps.pool,
            "kb_chunks",
            bind: ["embedding" => chunk::pack_embedding(&vectors[idx]), "embedding_model" => kb.embedding_model.clone().unwrap_or_default()],
            where: ("id", chunk_id)
        )?;
        items.push(VectorItem {
            unit_id: chunk_id,
            kb_id: i64::from(doc.kb_id),
            kind: "document".into(),
            embedding: vectors[idx].clone(),
        });
    }

    // ⑤ vector + BM25 index (delete-then-insert idempotency per doc)
    document::set_document_status(&deps.pool, doc_id, "indexing", None, None, tenant_id).await?;
    deps.vector
        .delete(
            i64::from(doc.kb_id),
            &items.iter().map(|v| v.unit_id).collect::<Vec<_>>(),
        )
        .await?;
    deps.vector
        .upsert(i64::from(doc.kb_id), dim, &items)
        .await?;
    let fts_units: Vec<KbIndexUnit> = inserts
        .iter()
        .map(|c| KbIndexUnit {
            unit_id: i64::from(c.id),
            kb_id: i64::from(c.kb_id),
            doc_id: i64::from(doc_id),
            kind: c.kind.clone(),
            text: match &c.breadcrumb {
                Some(b) => format!("{b}\n{}", c.content),
                None => c.content.clone(),
            },
        })
        .collect();
    deps.kbsearch.reindex_document(&fts_units).await?;

    // ⑥ ready
    let total = inserts.len() as i64;
    document::set_document_status(&deps.pool, doc_id, "ready", None, Some(total), tenant_id)
        .await?;
    // Incremental invalidation (§7): pages citing this doc go stale.
    let stale = crate::kb::models::wiki_page::mark_stale_by_doc(&deps.pool, doc_id).await?;
    if !stale.is_empty() {
        tracing::info!(
            "[kb] doc {doc_id} re-parsed: {} wiki page(s) marked stale",
            stale.len()
        );
    }
    deps.emitter.emit(Event::KbDocumentReady(
        document::find_document_by_id(&deps.pool, doc_id, tenant_id)
            .await?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("doc vanished")))?,
    ));
    Ok(())
}

/// Parse raw bytes to markdown. Markdown files pass through unchanged;
/// everything else goes through anydoc [抄EXT:anydoc to_markdown_bytes].
/// PDFs with scanned/image-only pages degrade to per-page extraction with
/// placeholders instead of failing the whole document (pdf-inspector
/// per-page API 混合 OCR 语义；真实 OCR 是后续工作).
fn parse_to_markdown(doc: &document::KbDocument, bytes: &[u8]) -> AppResult<String> {
    let is_markdown = doc
        .mime_type
        .as_deref()
        .is_some_and(|m| m == "text/markdown" || m == "text/plain")
        || doc
            .storage_key
            .as_deref()
            .is_some_and(|k| k.ends_with(".md"));
    if is_markdown {
        return String::from_utf8(bytes.to_vec())
            .map_err(|e| AppError::BadRequest(format!("invalid utf-8 markdown: {e}")));
    }
    match anydoc::to_markdown_bytes(bytes, None) {
        Ok(md) => Ok(md),
        Err(anydoc::ConvertError::NeedsOcr { pages, page_count }) => {
            extract_pdf_skip_ocr(bytes, &pages, page_count)
        }
        Err(e) => Err(AppError::BadRequest(format!("document parse failed: {e}"))),
    }
}

/// Degraded PDF parse: keep text pages, insert placeholders for scanned
/// pages. Fails only when nothing extractable remains.
fn extract_pdf_skip_ocr(bytes: &[u8], ocr_pages: &[u32], page_count: u32) -> AppResult<String> {
    let extracted = pdf_inspector::extract_pages_markdown_mem(bytes, None)
        .map_err(|e| AppError::BadRequest(format!("document parse failed: {e}")))?;
    let mut md = String::new();
    for page in &extracted.pages {
        if page.needs_ocr {
            let reason = page.ocr_reason.as_deref().unwrap_or("scanned page");
            md.push_str(&format!(
                "\n\n> [第 {} 页为扫描件（{}），需要 OCR，已跳过]\n",
                page.page + 1,
                reason
            ));
        } else {
            md.push_str(&page.markdown);
        }
    }
    if md.trim().is_empty() {
        return Err(AppError::BadRequest(format!(
            "document parse failed: pages {ocr_pages:?} of {page_count} need OCR and no extractable text remains"
        )));
    }
    tracing::warn!(
        "[kb] pdf degraded parse: {} of {} pages need OCR (skipped: {ocr_pages:?}), placeholders inserted",
        ocr_pages.len(),
        page_count
    );
    Ok(md)
}

/// Delete a document everywhere (SQL + vector + FTS), then emit.
pub async fn delete_document_everywhere(
    deps: &KbDeps,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    let Some(doc) = document::find_document_by_id(&deps.pool, doc_id, tenant_id).await? else {
        return Ok(());
    };
    let chunks = chunk::find_chunks_by_doc(&deps.pool, doc_id).await?;
    let ids: Vec<i64> = chunks.iter().map(|c| i64::from(c.id)).collect();
    if !ids.is_empty() {
        deps.vector.delete(i64::from(doc.kb_id), &ids).await?;
    }
    deps.kbsearch.delete_document(i64::from(doc_id)).await?;
    chunk::delete_chunks_by_doc(&deps.pool, doc_id).await?;
    crate::kb::models::wiki_source::delete_sources_by_doc(&deps.pool, doc_id).await?;
    document::delete_document(&deps.pool, doc_id, tenant_id).await?;
    // Original bytes on storage: warn-only cleanup (row is already gone;
    // a leaked file is preferable to a failed delete) [抄RF:services/media.rs].
    if let Some(key) = &doc.storage_key
        && let Err(e) = deps.storage.delete(key).await
    {
        tracing::warn!(key = %key, error = %e, "failed to delete kb document file from storage");
    }
    let stale = crate::kb::models::wiki_page::mark_stale_by_doc(&deps.pool, doc_id).await?;
    if !stale.is_empty() {
        tracing::info!(
            "[kb] doc {doc_id} deleted: {} wiki page(s) marked stale",
            stale.len()
        );
    }
    deps.emitter.emit(Event::KbDocumentDeleted(doc));
    Ok(())
}

/// Delete a KB and everything in it (documents, chunks, FAQs, wiki pages,
/// provenance links, vector + FTS indexes, original files), mirroring
/// `delete_document_everywhere` at KB scope.
pub async fn delete_kb_everywhere(
    deps: &KbDeps,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    let Some(kb) = knowledge_base::find_kb_by_id(&deps.pool, kb_id, tenant_id).await? else {
        return Ok(());
    };
    let docs = document::find_documents_by_kb(&deps.pool, kb_id, tenant_id).await?;
    // Vector + FTS: whole-KB wipe (both are keyed by kb_id).
    deps.vector.delete_all(i64::from(kb_id)).await?;
    deps.kbsearch.delete_kb(i64::from(kb_id)).await?;
    // SQL rows, children first (no FK cascade: order matters).
    chunk::delete_chunks_by_kb(&deps.pool, kb_id).await?;
    wiki_source::delete_sources_by_kb(&deps.pool, kb_id).await?;
    wiki_page::delete_pages_by_kb(&deps.pool, kb_id, tenant_id).await?;
    faq::delete_faqs_by_kb(&deps.pool, kb_id, tenant_id).await?;
    document::delete_documents_by_kb(&deps.pool, kb_id, tenant_id).await?;
    knowledge_base::delete_kb(&deps.pool, kb_id, tenant_id).await?;
    // Original bytes on storage: warn-only cleanup (rows are already gone;
    // a leaked file is preferable to a failed delete) [抄RF:services/media.rs].
    for doc in &docs {
        if let Some(key) = &doc.storage_key
            && let Err(e) = deps.storage.delete(key).await
        {
            tracing::warn!(key = %key, error = %e, "failed to delete kb document file from storage");
        }
    }
    tracing::info!(
        "[kb] kb {} ({}) deleted: {} document(s) removed with indexes",
        i64::from(kb_id),
        kb.slug,
        docs.len()
    );
    Ok(())
}

async fn ensure_kb(
    deps: &KbDeps,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<knowledge_base::KbKnowledgeBase> {
    knowledge_base::find_kb_by_id(&deps.pool, kb_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::NotFound("kb_knowledge_base".into()))
}

fn normalize_mime(mime: &str, filename: &str) -> String {
    let m = mime
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if m.is_empty() || m == "application/octet-stream" {
        mime_from_ext(filename).unwrap_or(m)
    } else {
        m
    }
}

fn mime_from_ext(filename: &str) -> Option<String> {
    let ext = filename.rsplit('.').next()?.to_lowercase();
    Some(
        match ext.as_str() {
            "md" | "markdown" => "text/markdown",
            "txt" => "text/plain",
            "html" | "htm" => "text/html",
            "csv" => "text/csv",
            "pdf" => "application/pdf",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "epub" => "application/epub+zip",
            "json" => "application/json",
            "rtf" => "application/rtf",
            "odt" => "application/vnd.oasis.opendocument.text",
            _ => return None,
        }
        .to_string(),
    )
}

fn trim_filename(filename: &str) -> String {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let trimmed = name.trim();
    if trimmed.is_empty() {
        "untitled".into()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kb::vectors::BruteForceIndex;

    /// Deterministic embedder for tests: dim from the hash of first chars.
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

    async fn test_deps() -> KbDeps {
        let pool = crate::test_pool!();
        let config = Arc::new(crate::config::app::AppConfig::test_defaults());
        let bus = crate::eventbus::EventBus::new(16);
        KbDeps {
            pool,
            config,
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-test-uploads", "/uploads")
                    .unwrap(),
            ),
            vector: Arc::new(BruteForceIndex::new()),
            kbsearch: Arc::new(KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(MockEmbedder { dim: 4 }),
            provider: None,
            emitter: EventEmitter::eventbus_only(bus),
        }
    }

    async fn seed_kb(deps: &KbDeps) -> SnowflakeId {
        knowledge_base::create_kb(
            &deps.pool,
            &knowledge_base::CreateKbCmd {
                name: "产品文档".into(),
                description: None,
                slug: "docs".into(),
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

    #[tokio::test]
    async fn online_doc_full_pipeline() {
        let deps = test_deps().await;
        let kb_id = seed_kb(&deps).await;

        let mut markdown =
            "# 安装指南\n\n通过 cargo 安装 raisfast 服务端。\n\n## 配置数据库\n\n".to_string();
        markdown.push_str(&"支持 SQLite、PostgreSQL 与 MySQL 三种后端。".repeat(80));
        let doc = create_online_document(&deps, kb_id, "安装指南", &markdown, None, "default")
            .await
            .unwrap();
        assert_eq!(doc.status, "pending");

        process_document(&deps, doc.id, "default").await.unwrap();

        let doc = document::find_document_by_id(&deps.pool, doc.id, "default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.status, "ready", "error: {:?}", doc.error);
        assert!(doc.chunk_count > 0);

        let chunks = chunk::find_chunks_by_doc(&deps.pool, doc.id).await.unwrap();
        assert_eq!(chunks.len() as i64, doc.chunk_count);
        let children = chunks.iter().filter(|c| c.parent_id.is_some()).count();
        let embedded = chunks.iter().filter(|c| c.embedding.is_some()).count();
        assert!(children > 0, "parent-child must produce children");
        assert_eq!(
            embedded, children,
            "exactly the child chunks carry embeddings"
        );
        // embedding BLOB roundtrip
        let with_vec = chunks.iter().find(|c| c.embedding.is_some()).unwrap();
        let v = chunk::unpack_embedding(with_vec.embedding.as_deref().unwrap());
        assert_eq!(v.len(), 4);
        // breadcrumbs populated for heading sections
        assert!(chunks.iter().any(|c| c.breadcrumb.is_some()));

        // BM25 side: search finds the doc's units in the KB scope
        let hits = deps
            .kbsearch
            .search(i64::from(kb_id), "数据库", 10)
            .await
            .unwrap();
        assert!(!hits.is_empty(), "fts must find the seeded content");

        // vector side: brute force hits a unit of this doc
        let probe = {
            let mut v = vec![0.0_f32; 4];
            v[3] = 1.0;
            v
        };
        let vhits = deps
            .vector
            .search(i64::from(kb_id), &probe, 10, None)
            .await
            .unwrap();
        assert!(!vhits.is_empty());

        // idempotency: reprocess keeps counts stable
        process_document(&deps, doc.id, "default").await.unwrap();
        let chunks2 = chunk::find_chunks_by_doc(&deps.pool, doc.id).await.unwrap();
        assert_eq!(
            chunks2.len(),
            chunks.len(),
            "reprocess must not duplicate chunks"
        );

        // delete cleans everywhere
        delete_document_everywhere(&deps, doc.id, "default")
            .await
            .unwrap();
        assert!(
            chunk::find_chunks_by_doc(&deps.pool, doc.id)
                .await
                .unwrap()
                .is_empty()
        );
        let hits = deps
            .kbsearch
            .search(i64::from(kb_id), "数据库", 10)
            .await
            .unwrap();
        assert!(hits.is_empty(), "fts must be cleared after delete");
    }

    #[tokio::test]
    async fn upload_rejects_unknown_mime() {
        let deps = test_deps().await;
        let kb_id = seed_kb(&deps).await;
        let err = upload_document(
            &deps,
            kb_id,
            "evil.exe",
            "application/x-executable",
            b"MZ..",
            None,
            "default",
        )
        .await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn markdown_upload_passes_through_parser() {
        let deps = test_deps().await;
        let kb_id = seed_kb(&deps).await;
        let doc = upload_document(
            &deps,
            kb_id,
            "notes.md",
            "text/markdown",
            b"# T\n\nhello kb",
            None,
            "default",
        )
        .await
        .unwrap();
        process_document(&deps, doc.id, "default").await.unwrap();
        let doc = document::find_document_by_id(&deps.pool, doc.id, "default")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc.status, "ready");
    }
}

// ── FAQ indexing (M5) ──────────────────────────────────────────────

/// (Re)index a FAQ: embed every question variant as a `kind='faq'` unit
/// into vector + BM25. Idempotent per FAQ (delete-then-insert).
pub async fn index_faq(deps: &KbDeps, faq: &crate::kb::models::faq::KbFaq) -> AppResult<()> {
    let old = crate::kb::models::chunk::find_chunks_by_faq(&deps.pool, faq.id).await?;
    let old_ids: Vec<i64> = old.iter().map(|c| i64::from(c.id)).collect();
    if !old_ids.is_empty() {
        deps.vector.delete(i64::from(faq.kb_id), &old_ids).await?;
        deps.kbsearch.delete_document(i64::from(faq.id)).await?;
    }
    crate::kb::models::chunk::delete_chunks_by_faq(&deps.pool, faq.id).await?;

    let variants = crate::kb::models::faq::question_variants(faq);
    let texts: Vec<&str> = variants.iter().map(String::as_str).collect();
    let kb_row = crate::kb::models::knowledge_base::find_kb_by_id(&deps.pool, faq.kb_id, "default")
        .await?
        .ok_or_else(|| AppError::NotFound("kb_knowledge_base".into()))?;
    let model = kb_row.embedding_model.clone().unwrap_or_default();
    let dim = u32::try_from(kb_row.embedding_dim.unwrap_or(0)).unwrap_or(0);
    let vectors = deps.embedder.embed_for(&model, dim, &texts).await?;
    let now = crate::utils::tz::now_utc();
    let mut items = Vec::with_capacity(variants.len());
    let mut fts = Vec::with_capacity(variants.len());
    for (idx, variant) in variants.iter().enumerate() {
        let chunk_id = crate::utils::id::new_snowflake_id();
        crate::kb::models::chunk::insert_chunk(
            &deps.pool,
            &crate::kb::models::chunk::KbChunkInsert {
                id: chunk_id,
                kb_id: faq.kb_id,
                doc_id: None,
                faq_id: Some(faq.id),
                wiki_page_id: None,
                kind: "faq".into(),
                parent_id: None,
                seq: idx as i64,
                content: variant.clone(),
                breadcrumb: None,
                byte_start: 0,
                byte_end: 0,
                questions: None,
                embedding: Some(crate::kb::models::chunk::pack_embedding(&vectors[idx])),
                embedding_model: None,
                created_at: now,
            },
        )
        .await?;
        items.push(crate::kb::vectors::VectorItem {
            unit_id: i64::from(chunk_id),
            kb_id: i64::from(faq.kb_id),
            kind: "faq".into(),
            embedding: vectors[idx].clone(),
        });
        fts.push(crate::kb::kbsearch::KbIndexUnit {
            unit_id: i64::from(chunk_id),
            kb_id: i64::from(faq.kb_id),
            doc_id: i64::from(faq.id),
            kind: "faq".into(),
            text: variant.clone(),
        });
    }
    let dim = vectors.first().map(|v| v.len() as u32).unwrap_or(0);
    deps.vector
        .upsert(i64::from(faq.kb_id), dim, &items)
        .await?;
    deps.kbsearch.reindex_document(&fts).await?;
    Ok(())
}

/// Remove a FAQ everywhere.
pub async fn deindex_faq(deps: &KbDeps, faq_id: SnowflakeId, kb_id: SnowflakeId) -> AppResult<()> {
    let old = crate::kb::models::chunk::find_chunks_by_faq(&deps.pool, faq_id).await?;
    let ids: Vec<i64> = old.iter().map(|c| i64::from(c.id)).collect();
    if !ids.is_empty() {
        deps.vector.delete(i64::from(kb_id), &ids).await?;
    }
    deps.kbsearch.delete_document(i64::from(faq_id)).await?;
    crate::kb::models::chunk::delete_chunks_by_faq(&deps.pool, faq_id).await?;
    Ok(())
}

#[cfg(test)]
mod faq_tests {
    use super::*;
    use crate::kb::models::faq::{self, CreateFaqCmd};
    use crate::kb::pipeline::{self, AskRequest};
    use crate::kb::vectors::BruteForceIndex;
    use raisfast_agent::{ChatRequest, ChatResponse, ModelProvider, ProviderError};

    struct OneHotEmbedder;
    #[async_trait::async_trait]
    impl KbEmbedder for OneHotEmbedder {
        async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0_f32; 4];
                    v[t.bytes().map(|b| b as usize).sum::<usize>() % 4] = 1.0;
                    v
                })
                .collect())
        }
    }

    struct EchoProvider;
    #[async_trait::async_trait]
    impl ModelProvider for EchoProvider {
        fn name(&self) -> &str {
            "echo"
        }
        async fn chat(
            &self,
            _r: &ChatRequest<'_>,
            _m: &str,
        ) -> Result<ChatResponse, ProviderError> {
            Ok(ChatResponse::text_only("[1] FAQ 命中回答。"))
        }
    }

    async fn deps() -> KbDeps {
        let pool = crate::test_pool!();
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        config.kb.fallback_threshold = 0.05;
        KbDeps {
            pool,
            config: Arc::new(config),
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-faq-test", "/uploads").unwrap(),
            ),
            vector: Arc::new(BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(OneHotEmbedder),
            provider: Some(Arc::new(EchoProvider)),
            emitter: crate::event::EventEmitter::eventbus_only(crate::eventbus::EventBus::new(16)),
        }
    }

    async fn seed_kb(deps: &KbDeps) -> SnowflakeId {
        crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "faq-kb".into(),
                description: None,
                slug: "faq-kb".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn faq_index_withdraw_and_pipeline_injection() {
        let deps = deps().await;
        let kb_id = seed_kb(&deps).await;
        let faq = faq::create_faq(
            &deps.pool,
            &CreateFaqCmd {
                kb_id,
                standard_question: "如何重置密码".into(),
                similar_questions: vec!["忘记密码怎么办".into()],
                answers: vec!["在登录页点击「忘记密码」".into()],
                enabled: true,
                created_by: None,
            },
            "default",
        )
        .await
        .unwrap();
        index_faq(&deps, &faq).await.unwrap();

        // variants indexed as faq units in both stores
        let units = crate::kb::models::chunk::find_chunks_by_faq(&deps.pool, faq.id)
            .await
            .unwrap();
        assert_eq!(units.len(), 2);
        assert!(
            units
                .iter()
                .all(|u| u.kind == "faq" && u.embedding.is_some())
        );
        let hits = deps
            .kbsearch
            .search(i64::from(kb_id), "重置密码", 10)
            .await
            .unwrap();
        assert!(!hits.is_empty());

        // pipeline: ask the FAQ → answered, FAQ unit pinned first
        let ask = AskRequest {
            kb_ids: vec![i64::from(kb_id)],
            doc_ids: Vec::new(),
            question: "如何重置密码".into(),
        };
        let mut outcome = pipeline::prepare_answer(&deps, &ask).await.unwrap();
        pipeline::finish_answer(&deps, &mut outcome).await.unwrap();
        assert_eq!(outcome.status, "answered");
        assert!(
            outcome.context_units.first().is_some_and(|u| u.is_faq),
            "FAQ must lead context"
        );

        // withdraw (disable) removes units from retrieval
        deindex_faq(&deps, faq.id, kb_id).await.unwrap();
        let units = crate::kb::models::chunk::find_chunks_by_faq(&deps.pool, faq.id)
            .await
            .unwrap();
        assert!(units.is_empty());
    }

    #[tokio::test]
    async fn gaps_list_uncovered_queries() {
        let deps = deps().await;
        let kb_id = seed_kb(&deps).await;
        crate::kb::models::query_log::insert_log(
            &deps.pool,
            Some(kb_id),
            "什么是量子纠缠",
            Some("知识库未覆盖该问题。"),
            None,
            "uncovered",
            Some(0.0),
            None,
        )
        .await
        .unwrap();
        crate::kb::models::query_log::insert_log(
            &deps.pool,
            Some(kb_id),
            "什么是量子纠缠",
            Some("知识库未覆盖该问题。"),
            None,
            "uncovered",
            Some(0.0),
            None,
        )
        .await
        .unwrap();
        let gaps = crate::kb::models::query_log::list_gaps(&deps.pool, 10)
            .await
            .unwrap();
        let entry = gaps.iter().find(|(q, _, _)| q == "什么是量子纠缠");
        assert!(
            entry.is_some_and(|(_, c, _)| *c >= 2),
            "gaps must aggregate counts: {gaps:?}"
        );
    }
}
