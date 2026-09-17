//! KB image recognition — parse-time extraction + async VLM recognition
//! (kb-image-recognition-design §3).
//!
//! Extraction splits by source [抄WK:types/chunk.go ImageInfo 语义]:
//! - `embedded`: bytes inside DOCX/ODT/EPUB/PPTX… via anydoc's Document
//!   model (`Document::assets`; PDFs have no document-model form — declared
//!   v1 limitation), saved to storage and fed to the VLM as a data URI.
//! - `external`: `![alt](http…)` references in the parsed markdown; the URL
//!   is passed to the VLM as-is (server-side fetch), position mapped to the
//!   containing chunk via byte offsets.
//!
//! Recognition is a post-ready job (`KbImageRecognize`): per image one
//! Caption call + one OCR call [抄WK:image_multimodal.go 双 prompt 照抄],
//! results land on `kb_images` + a standalone retrieval chunk per image, and
//! the containing chunk gets an `image_info` display refresh.

use base64::Engine as _;
use comrak::nodes::NodeValue;
use comrak::{Arena, Options, parse_document};
use raisfast_agent::messages::{ChatMessage, ChatRole};

use crate::errors::app_error::{AppError, AppResult};
use crate::kb::service::KbDeps;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::prompt_file::prompt_file;

/// Parsed `kb_knowledge_bases.image_config`
/// [抄WK:knowledgebase.go ImageProcessingConfig + VLMConfig 形态].
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ImageConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model: String,
    /// Caption language; empty = follow the request/document language.
    #[serde(default)]
    pub caption_language: String,
    /// KB-specific image interpretation guidance appended to the caption
    /// prompt [抄WK:VLMConfig.CustomInstructions 语义].
    #[serde(default)]
    pub custom_instructions: String,
}

/// Resolved recognition config: KB row `image_config.model` → global env
/// `RAISFAST_KB_IMAGE_MODEL`; recognition off when neither yields a model.
pub fn resolve_image_config(
    deps: &KbDeps,
    kb: &crate::kb::models::knowledge_base::KbKnowledgeBase,
) -> Option<ImageConfig> {
    let mut cfg: ImageConfig = kb
        .image_config
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if cfg.model.trim().is_empty() {
        cfg.model = deps.config.kb.image_model.clone().unwrap_or_default();
    }
    if !cfg.enabled || cfg.model.trim().is_empty() {
        return None;
    }
    Some(cfg)
}

// ── parse-time extraction ────────────────────────────────────────────────

/// Embedded image assets of a non-PDF document (best-effort second parse
/// via anydoc's Document API — markdown itself comes from
/// `to_markdown_bytes`; PDFs have no document-model form and yield none).
pub fn extract_embedded_images(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    match anydoc::to_document(bytes, None) {
        Ok(doc) => doc
            .assets
            .into_iter()
            .filter(|a| a.media_type.starts_with("image/"))
            .map(|a| (a.media_type, a.bytes))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// External image references in the parsed markdown: `(url, byte_offset)`.
/// Walked via the comrak AST [与 chunker 同一解析栈]; the offset maps to the
/// containing chunk (`kb_chunks.byte_start/end` are byte offsets into this
/// same markdown).
pub fn extract_external_images(markdown: &str) -> Vec<(String, usize)> {
    // Byte offset of each line start, for (line, column) → byte offset.
    let mut line_starts = Vec::with_capacity(markdown.matches('\n').count() + 1);
    let mut off = 0usize;
    for line in markdown.split_inclusive('\n') {
        line_starts.push(off);
        off += line.len();
    }
    // `LineColumn.column` is a 1-based UTF-8 byte offset within the line —
    // directly combinable with line-start byte offsets.
    let to_offset = |pos: &comrak::nodes::LineColumn| -> usize {
        line_starts
            .get(pos.line.saturating_sub(1))
            .map(|start| start + pos.column.saturating_sub(1))
            .unwrap_or(0)
    };

    let mut options = Options::default();
    options.render.sourcepos = true;
    options.parse.smart = false;
    let arena = Arena::new();
    let root = parse_document(&arena, markdown, &options);
    let mut out = Vec::new();
    for node in root.descendants() {
        if let NodeValue::Image(link) = &node.data.borrow().value {
            let url = link.url.clone();
            if url.starts_with("http://") || url.starts_with("https://") {
                out.push((url, to_offset(&node.data.borrow().sourcepos.start)));
            }
        }
    }
    out
}

/// Register a document's images at parse time. Called from
/// `process_document` after chunking (external images need the chunk id
/// mapping). Idempotency: callers wipe `kb_images` for the doc first.
pub async fn register_document_images(
    deps: &KbDeps,
    doc: &crate::kb::models::document::KbDocument,
    raw_bytes: Option<&[u8]>,
    markdown: &str,
    chunk_rows: &[(SnowflakeId, i64, i64)], // (chunk_id, byte_start, byte_end)
    tenant_id: &str,
) -> AppResult<usize> {
    let mut count = 0usize;
    // Embedded assets → storage + doc-level rows.
    if let Some(bytes) = raw_bytes {
        for (mime, data) in extract_embedded_images(bytes) {
            if data.is_empty() {
                continue;
            }
            let ext = mime.rsplit('/').next().unwrap_or("bin");
            let key = crate::services::media::storage_key("kb-image", ext);
            deps.storage.put(&key, &data, &mime).await?;
            crate::kb::models::image::insert_image(
                &deps.pool,
                &crate::kb::models::image::CreateKbImageCmd {
                    kb_id: doc.kb_id,
                    doc_id: doc.id,
                    chunk_id: None,
                    storage_key: Some(key),
                    mime_type: mime,
                    bytes: Some(data.len() as i64),
                    source: "embedded",
                    original_url: None,
                },
                tenant_id,
            )
            .await?;
            count += 1;
        }
    }
    // External references → rows with containing-chunk mapping.
    for (url, offset) in extract_external_images(markdown) {
        let chunk_id = chunk_rows
            .iter()
            .find(|(_, start, end)| offset >= *start as usize && offset < *end as usize)
            .map(|(id, _, _)| *id);
        crate::kb::models::image::insert_image(
            &deps.pool,
            &crate::kb::models::image::CreateKbImageCmd {
                kb_id: doc.kb_id,
                doc_id: doc.id,
                chunk_id,
                storage_key: None,
                mime_type: "image/external".into(),
                bytes: None,
                source: "external",
                original_url: Some(url),
            },
            tenant_id,
        )
        .await?;
        count += 1;
    }
    Ok(count)
}

// ── recognition job body ─────────────────────────────────────────────────

/// VLM image reference fed to the model: data URI (embedded) or URL
/// (external) — both ride `ChatMessage.images` (P1).
async fn image_vlm_ref(
    deps: &KbDeps,
    img: &crate::kb::models::image::KbImage,
) -> AppResult<String> {
    match (&img.storage_key, &img.original_url) {
        (Some(key), _) => {
            let bytes = deps.storage.get(key).await?;
            Ok(format!(
                "data:{};base64,{}",
                img.mime_type,
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        }
        (None, Some(url)) => Ok(url.clone()),
        (None, None) => Err(AppError::Internal(anyhow::anyhow!(
            "image {} has neither bytes nor url",
            i64::from(img.id)
        ))),
    }
}

async fn vlm_call(
    deps: &KbDeps,
    tenant: &str,
    model: &str,
    system: &str,
    user: &str,
    image_ref: &str,
) -> AppResult<String> {
    let messages = [
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
            images: vec![image_ref.to_string()],
            tool_calls: None,
            tool_call_id: None,
        },
    ];
    let request = raisfast_agent::provider::ChatRequest {
        messages: &messages,
        tools: None,
        temperature: Some(0.0),
        max_tokens: None,
        stop: None,
    };
    crate::kb::service::kb_chat_model(deps, tenant, Some(model), &request).await
}

/// The `KbImageRecognize` job body: recognize every pending image of a doc.
/// Failures mark individual images and never fail the job (增益不是依赖，
/// 与 rerank 容错同口径). Run-traced for the diagnostics plane
/// (kb_runs kind=`ingest_image`, kb-image-recognition-design §4).
pub async fn recognize_document(
    deps: &KbDeps,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<usize> {
    recognize_document_traced(deps, doc_id, tenant_id, None, 1).await
}

/// Traced variant — the worker handler passes job provenance (DR6).
pub async fn recognize_document_traced(
    deps: &KbDeps,
    doc_id: SnowflakeId,
    tenant_id: &str,
    job_id: Option<SnowflakeId>,
    attempt: i64,
) -> AppResult<usize> {
    let mut trace = crate::kb::trace::RunRecorder::create(
        crate::kb::trace::TraceMode::parse(&deps.config.kb.trace_mode),
        crate::kb::trace::RunSpec {
            kind: crate::kb::models::kb_run::KIND_INGEST_IMAGE,
            trigger_src: "job",
            tenant_id: tenant_id.to_string(),
            kb_id: None,
            doc_id: Some(doc_id),
            agent_id: None,
            session_id: None,
            job_id,
            attempt,
            long_running: true,
        },
    );
    let result = recognize_document_inner(deps, doc_id, tenant_id, &mut trace).await;
    match &result {
        Ok(_) => {
            trace
                .finish(&deps.pool, crate::kb::models::kb_run::STATUS_OK, None)
                .await;
        }
        Err(e) => {
            trace.fail_stage(&e.to_string());
            trace
                .finish(
                    &deps.pool,
                    crate::kb::models::kb_run::STATUS_FAILED,
                    Some(&e.to_string()),
                )
                .await;
        }
    }
    result
}

async fn recognize_document_inner(
    deps: &KbDeps,
    doc_id: SnowflakeId,
    tenant_id: &str,
    trace: &mut crate::kb::trace::RunRecorder,
) -> AppResult<usize> {
    let Some(doc) =
        crate::kb::models::document::find_document_by_id(&deps.pool, doc_id, tenant_id).await?
    else {
        return Err(AppError::NotFound("kb_document".into()));
    };
    let Some(kb) =
        crate::kb::models::knowledge_base::find_kb_by_id(&deps.pool, doc.kb_id, tenant_id).await?
    else {
        return Err(AppError::NotFound("kb_knowledge_base".into()));
    };
    trace.set_kb(kb.id);
    trace.begin(&deps.pool, &deps.config).await;
    let pending =
        crate::kb::models::image::find_pending_by_doc(&deps.pool, doc_id, tenant_id).await?;
    if pending.is_empty() {
        trace.stage("recognize");
        trace.end_stage(
            crate::kb::trace::STAGE_SKIPPED,
            serde_json::json!({ "reason": "no_pending_images" }),
            None,
        );
        return Ok(0);
    }
    // Config may have been disabled between registration and the job run —
    // skip what's left instead of failing.
    let Some(cfg) = resolve_image_config(deps, &kb) else {
        for img in &pending {
            let _ = crate::kb::models::image::set_image_failed(
                &deps.pool,
                img.id,
                "recognition disabled",
                tenant_id,
            )
            .await;
        }
        trace.stage("recognize");
        trace.end_stage(
            crate::kb::trace::STAGE_SKIPPED,
            serde_json::json!({ "reason": "recognition_disabled", "skipped": pending.len() }),
            None,
        );
        return Ok(0);
    };
    let model = cfg.model.trim().to_string();
    let language = if cfg.caption_language.trim().is_empty() {
        "the document's language".to_string()
    } else {
        cfg.caption_language.trim().to_string()
    };
    let caption_system = match cfg.custom_instructions.is_empty() {
        false => format!(
            "{}\nAdditional guidance: {}",
            prompt_file!("src/kb/prompts/image_caption.md"),
            cfg.custom_instructions
        ),
        true => prompt_file!("src/kb/prompts/image_caption.md"),
    };
    let ocr_system = prompt_file!("src/kb/prompts/image_ocr.md");

    trace.stage("recognize");
    let mut done = 0usize;
    let mut per_image: Vec<serde_json::Value> = Vec::with_capacity(pending.len());
    for img in &pending {
        let result = recognize_one(
            deps,
            tenant_id,
            &model,
            &language,
            &caption_system,
            &ocr_system,
            img,
        )
        .await;
        match result {
            Ok((caption, ocr)) => {
                crate::kb::models::image::set_image_result(
                    &deps.pool,
                    img.id,
                    caption.as_deref(),
                    ocr.as_deref(),
                    tenant_id,
                )
                .await?;
                done += 1;
                per_image.push(serde_json::json!({
                    "id": i64::from(img.id),
                    "status": "done",
                    "caption": crate::kb::trace::snippet(caption.as_deref().unwrap_or_default()),
                }));
            }
            Err(e) => {
                tracing::warn!("[kb] image {} recognition failed: {e}", i64::from(img.id));
                let _ = crate::kb::models::image::set_image_failed(
                    &deps.pool,
                    img.id,
                    &e.to_string(),
                    tenant_id,
                )
                .await;
                per_image.push(serde_json::json!({
                    "id": i64::from(img.id),
                    "status": "failed",
                    "error": e.to_string(),
                }));
            }
        }
    }
    trace.end_stage(
        crate::kb::trace::STAGE_OK,
        serde_json::json!({
            "model": model,
            "pending": pending.len(),
            "done": done,
            "images": per_image,
        }),
        None,
    );

    // Publish recognized images into retrieval: standalone chunks + the
    // containing chunk's display refresh.
    trace.stage("publish");
    match publish_recognized(deps, &kb, doc_id, tenant_id).await {
        Ok(chunks) => {
            trace.end_stage(
                crate::kb::trace::STAGE_OK,
                serde_json::json!({ "chunks": chunks }),
                None,
            );
        }
        Err(e) => {
            trace.fail_stage(&e.to_string());
            return Err(e);
        }
    }
    Ok(done)
}

/// One image = caption call + OCR call [抄WK:image_multimodal.go 双 prompt].
/// OCR replying "No text content" maps to `None`.
async fn recognize_one(
    deps: &KbDeps,
    tenant: &str,
    model: &str,
    language: &str,
    caption_system: &str,
    ocr_system: &str,
    img: &crate::kb::models::image::KbImage,
) -> AppResult<(Option<String>, Option<String>)> {
    let image_ref = image_vlm_ref(deps, img).await?;
    let caption = vlm_call(
        deps,
        tenant,
        model,
        caption_system,
        &format!("Describe this image in {language}."),
        &image_ref,
    )
    .await?;
    let caption = caption.trim().to_string();
    let caption = if caption.is_empty() {
        None
    } else {
        Some(caption)
    };
    let ocr = vlm_call(
        deps,
        tenant,
        model,
        ocr_system,
        "Extract the text content of this image.",
        &image_ref,
    )
    .await?;
    let ocr = match ocr.trim() {
        "" | "No text content" => None,
        other => Some(other.to_string()),
    };
    Ok((caption, ocr))
}

/// Standalone retrieval chunk per done image + `image_info` refresh on the
/// containing chunk [抄WK:creates child chunks 语义].
async fn publish_recognized(
    deps: &KbDeps,
    kb: &crate::kb::models::knowledge_base::KbKnowledgeBase,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<usize> {
    let images =
        crate::kb::models::image::list_images_by_doc(&deps.pool, doc_id, tenant_id).await?;
    let done: Vec<_> = images.iter().filter(|i| i.status == "done").collect();
    if done.is_empty() {
        return Ok(0);
    }
    // Embedding model pinned on the KB row (same discipline as document
    // chunks and wiki units).
    let model = kb.embedding_model.clone().unwrap_or_default();
    let dim = u32::try_from(kb.embedding_dim.unwrap_or(0)).unwrap_or(0);
    if model.is_empty() || dim == 0 {
        tracing::warn!("[kb] image chunks skipped: kb has no embedding model");
        return Ok(0);
    }

    // Wipe previous image chunks of this doc (idempotent re-runs), then
    // recreate from current results.
    let old = crate::kb::models::chunk::find_chunks_by_doc(&deps.pool, doc_id).await?;
    let old_image: Vec<i64> = old
        .iter()
        .filter(|c| c.kind == "image")
        .map(|c| i64::from(c.id))
        .collect();
    if !old_image.is_empty() {
        deps.vector.delete(i64::from(kb.id), &old_image).await?;
        for cid in &old_image {
            deps.kbsearch.delete_document(*cid).await?;
        }
        crate::kb::models::chunk::delete_chunks_by_ids(&deps.pool, &old_image).await?;
    }

    let now = crate::utils::tz::now_utc();
    let mut inserts = Vec::new();
    let mut texts: Vec<String> = Vec::new();
    for (i, img) in done.iter().enumerate() {
        let mut content = String::new();
        if let Some(c) = &img.caption {
            content.push_str(c);
        }
        if let Some(o) = &img.ocr_text {
            if !content.is_empty() {
                content.push_str("\n\n");
            }
            content.push_str(o);
        }
        if content.trim().is_empty() {
            continue;
        }
        inserts.push(crate::kb::models::chunk::KbChunkInsert {
            id: crate::utils::id::new_snowflake_id(),
            kb_id: kb.id,
            doc_id: Some(doc_id),
            faq_id: None,
            wiki_page_id: None,
            kind: "image".into(),
            parent_id: None,
            seq: 100_000 + i as i64, // image chunks sort after text chunks
            content: content.clone(),
            breadcrumb: img.caption.clone(),
            byte_start: 0,
            byte_end: 0,
            questions: None,
            embedding: None,
            embedding_model: None,
            created_at: now,
        });
        texts.push(content);
    }
    if inserts.is_empty() {
        return Ok(0);
    }
    for ins in &inserts {
        crate::kb::models::chunk::insert_chunk(&deps.pool, ins).await?;
    }
    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    let vectors = deps
        .embedder
        .embed_for(tenant_id, &model, dim, &text_refs)
        .await?;
    let mut items = Vec::with_capacity(inserts.len());
    let mut fts = Vec::with_capacity(inserts.len());
    for (idx, ins) in inserts.iter().enumerate() {
        crate::kb::models::chunk::update_embedding(&deps.pool, ins.id, &vectors[idx]).await?;
        items.push(crate::kb::vectors::VectorItem {
            unit_id: i64::from(ins.id),
            kb_id: i64::from(kb.id),
            kind: "image".into(),
            embedding: vectors[idx].clone(),
        });
        fts.push(crate::kb::kbsearch::KbIndexUnit {
            unit_id: i64::from(ins.id),
            kb_id: i64::from(kb.id),
            doc_id: i64::from(doc_id),
            kind: "image".into(),
            text: format!(
                "{}\n{}",
                ins.breadcrumb.clone().unwrap_or_default(),
                ins.content
            ),
        });
    }
    deps.vector.upsert(i64::from(kb.id), dim, &items).await?;
    deps.kbsearch.reindex_document(&fts).await?;

    // Containing-chunk display refresh (bounded caption preview).
    for img in done.iter().filter(|i| i.chunk_id.is_some()) {
        let info = serde_json::json!({
            "caption": img.caption.as_deref().unwrap_or_default(),
            "ocr": img.ocr_text.as_deref().map(|t| t.chars().take(200).collect::<String>()),
            "url": img.original_url.as_deref().unwrap_or_default(),
        });
        if let Some(cid) = img.chunk_id {
            crate::kb::models::image::set_chunk_image_info(&deps.pool, cid, Some(info)).await?;
        }
    }
    Ok(inserts.len())
}
