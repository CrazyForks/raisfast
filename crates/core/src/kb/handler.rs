//! KB HTTP handlers — thin layer (kb-technical-design §10).
//!
//! `/api/v1/admin/kb/*` are admin-scoped governance endpoints (upload,
//! online entry, list/get/delete, reparse, chunker preview, rebuild).

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::kb::chunker::{self, ChunkerConfig};
use crate::kb::models::{chunk, document, knowledge_base};
use crate::kb::service::{self, KbDeps};
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::worker::JobQueue as _;

/// Register routes. Paths are prefixed `/api/v1` by `reg_route!`.
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/knowledge-bases",
        post,
        admin_create_kb,
        "system",
        "admin/kb/knowledge-bases",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/knowledge-bases",
        get,
        admin_list_kbs,
        "system",
        "admin/kb/knowledge-bases",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents",
        post,
        admin_upload_document,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents/online",
        post,
        admin_create_online_document,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents",
        get,
        admin_list_documents,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents/{id}",
        get,
        admin_get_document,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents/{id}",
        delete,
        admin_delete_document,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/documents/{id}/reparse",
        post,
        admin_reparse_document,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/chunker/preview",
        post,
        admin_chunker_preview,
        "system",
        "admin/kb/chunker",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/distill",
        post,
        admin_distill_wiki,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages",
        get,
        admin_list_wiki_pages,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}",
        get,
        admin_get_wiki_page,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}",
        put,
        admin_edit_wiki_page,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}/approve",
        post,
        admin_approve_wiki_page,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}/reject",
        post,
        admin_reject_wiki_page,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}/revisions",
        get,
        admin_wiki_revisions,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/wiki/pages/{id}/diff/{a}/{b}",
        get,
        admin_wiki_diff,
        "system",
        "admin/kb/wiki",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/chunks/{id}",
        put,
        admin_edit_chunk,
        "system",
        "admin/kb/chunks",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/faqs",
        post,
        admin_create_faq,
        "system",
        "admin/kb/faqs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/faqs/{id}",
        delete,
        admin_delete_faq,
        "system",
        "admin/kb/faqs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/faqs/{id}/enabled",
        post,
        admin_set_faq_enabled,
        "system",
        "admin/kb/faqs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/faqs/from-log/{log_id}",
        post,
        admin_faq_from_log,
        "system",
        "admin/kb/faqs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/gaps",
        get,
        admin_kb_gaps,
        "system",
        "admin/kb/gaps",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/kb/ask",
        post,
        public_ask,
        "system",
        "kb/ask",
        "authed"
    );
    reg_route!(
        r,
        registry,
        config.api_restful,
        "/kb/search",
        post,
        public_search,
        "system",
        "kb/search",
        "authed"
    )
}

impl AppState {
    /// Build the KB dependency set from app state.
    fn kb_deps(&self) -> AppResult<KbDeps> {
        let kb = self
            .kb_runtime
            .as_ref()
            .ok_or_else(|| AppError::ServiceUnavailable("knowledge base is disabled".into()))?;
        Ok(KbDeps {
            pool: self.pool.clone(),
            config: self.config.clone(),
            storage: self.storage.clone(),
            vector: kb.vector.clone(),
            kbsearch: kb.kbsearch.clone(),
            embedder: kb.embedder.clone(),
            provider: Some(kb.provider.clone()),
            emitter: self.emitter.clone(),
        })
    }
}

// ── knowledge bases ────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateKbRequest {
    name: String,
    slug: String,
    #[serde(default = "default_kb_kind")]
    kind: String,
}

fn default_kb_kind() -> String {
    "document".into()
}

async fn admin_create_kb(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateKbRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    state.kb_deps()?;
    if !matches!(req.kind.as_str(), "document" | "faq") {
        return Err(AppError::BadRequest(
            "kind must be 'document' or 'faq' (wiki is an indexing strategy on document KBs, D4)"
                .into(),
        ));
    }
    let kb = knowledge_base::create_kb(
        &state.pool,
        &knowledge_base::CreateKbCmd {
            name: req.name,
            slug: req.slug,
            kind: req.kind,
            indexing_strategy: None,
            embedding_model: state.config.ai.embedding_model.clone(),
            embedding_dim: state.config.ai.embedding_dim.map(i64::from),
        },
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "id": kb.id, "slug": kb.slug, "kind": kb.kind }),
    ))
}

async fn admin_list_kbs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let page: i64 = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let page_size: i64 = q
        .get("page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let (kbs, total) =
        knowledge_base::list_kbs(&state.pool, page, page_size, &tenant_of(&auth)).await?;
    Ok(ApiResponse::success(
        json!({ "items": kbs, "total": total, "page": page, "page_size": page_size }),
    ))
}

// ── documents ──────────────────────────────────────────────────────

async fn admin_upload_document(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;

    // field 1: kb_id
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart read failed: {e}")))?
        .ok_or_else(|| AppError::BadRequest("kb_id field missing".into()))?;
    let kb_id: SnowflakeId = field
        .text()
        .await
        .map_err(|e| AppError::BadRequest(format!("kb_id read failed: {e}")))?
        .parse::<i64>()
        .map_err(|_| AppError::BadRequest("kb_id must be a snowflake integer".into()))?
        .into();

    // field 2: file
    let field = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart read failed: {e}")))?
        .ok_or_else(|| AppError::BadRequest("file field missing".into()))?;
    let filename = field.file_name().unwrap_or("untitled").to_string();
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();
    let data = field
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("file read failed: {e}")))?;

    let doc = service::upload_document(
        &deps,
        kb_id,
        &filename,
        &content_type,
        &data,
        auth.user_id().map(crate::types::snowflake_id::SnowflakeId),
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "id": doc.id, "status": doc.status, "title": doc.title }),
    ))
}

#[derive(Deserialize)]
struct OnlineDocRequest {
    kb_id: SnowflakeId,
    title: String,
    markdown: String,
}

async fn admin_create_online_document(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<OnlineDocRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let doc = service::create_online_document(
        &deps,
        req.kb_id,
        &req.title,
        &req.markdown,
        auth.user_id().map(crate::types::snowflake_id::SnowflakeId),
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "id": doc.id, "status": doc.status, "title": doc.title }),
    ))
}

async fn admin_list_documents(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let kb_id: i64 = q
        .get("kb_id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("kb_id query param required".into()))?;
    let page: i64 = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let page_size: i64 = q
        .get("page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let (docs, total) = document::list_documents(
        &state.pool,
        SnowflakeId(kb_id),
        q.get("status").map(String::as_str),
        page,
        page_size,
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "items": docs, "total": total, "page": page, "page_size": page_size }),
    ))
}

async fn admin_get_document(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = crate::kb::handler::parse_snowflake(&id)?;
    let doc = document::find_document_by_id(&state.pool, id, &tenant_of(&auth))
        .await?
        .ok_or_else(|| AppError::NotFound("kb_document".into()))?;
    let chunks = chunk::find_chunks_by_doc(&state.pool, id).await?;
    Ok(ApiResponse::success(
        json!({ "document": doc, "chunk_count": chunks.len() }),
    ))
}

async fn admin_delete_document(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    service::delete_document_everywhere(&deps, id, &tenant_of(&auth)).await?;
    Ok(ApiResponse::success(json!({ "deleted": true })))
}

async fn admin_reparse_document(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    service::process_document(&deps, id, &tenant_of(&auth)).await?;
    let doc = document::find_document_by_id(&state.pool, id, &tenant_of(&auth))
        .await?
        .ok_or_else(|| AppError::NotFound("kb_document".into()))?;
    Ok(ApiResponse::success(
        json!({ "id": doc.id, "status": doc.status }),
    ))
}

// ── chunker preview [抄WK:CHUNKING.md Debugging + POST /api/v1/chunker/preview] ──

#[derive(Deserialize)]
struct ChunkerPreviewRequest {
    markdown: String,
    #[serde(default)]
    parent_size: Option<usize>,
    #[serde(default)]
    child_size: Option<usize>,
}

async fn admin_chunker_preview(
    auth: AuthUser,
    State(_state): State<AppState>,
    Json(req): Json<ChunkerPreviewRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    if req.markdown.len() > 64 * 1024 {
        return Err(AppError::PayloadTooLarge("sample exceeds 64KB".into()));
    }
    let defaults = ChunkerConfig::default();
    let cfg = ChunkerConfig {
        parent_size: req
            .parent_size
            .unwrap_or(defaults.parent_size)
            .clamp(64, 8192),
        child_size: req
            .child_size
            .unwrap_or(defaults.child_size)
            .clamp(64, 2048),
    };
    let chunks = chunker::chunk_markdown(&req.markdown, &cfg);
    let items: Vec<Value> = chunks
        .iter()
        .map(|c| {
            json!({
                "breadcrumb": c.breadcrumb,
                "is_child": c.parent.is_some(),
                "bytes": c.content.len(),
                "byte_start": c.byte_start,
                "byte_end": c.byte_end,
                "preview": preview(&c.content, 120),
            })
        })
        .collect();
    Ok(ApiResponse::success(json!({
        "strategy": "markdown_ast (text-splitter MarkdownSplitter)",
        "parent_size": cfg.parent_size,
        "child_size": cfg.child_size,
        "total": chunks.len(),
        "parents": chunks.iter().filter(|c| c.parent.is_none()).count(),
        "children": chunks.iter().filter(|c| c.parent.is_some()).count(),
        "items": items,
    })))
}

fn tenant_of(auth: &AuthUser) -> String {
    auth.tenant_id()
        .unwrap_or(crate::constants::DEFAULT_TENANT)
        .to_string()
}

fn parse_snowflake(raw: &str) -> AppResult<SnowflakeId> {
    raw.parse::<i64>()
        .map(SnowflakeId)
        .map_err(|_| AppError::BadRequest("id must be a snowflake integer".into()))
}

fn preview(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if truncated.chars().count() < text.chars().count() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

// ── public QA endpoints (authed; D4 kb_ids binding, D5 single-turn) ──

#[derive(Deserialize)]
struct AskRequestDto {
    question: String,
    #[serde(default)]
    kb_ids: Option<Vec<i64>>,
    #[serde(default)]
    stream: bool,
}

#[derive(serde::Serialize)]
struct ReferenceDto {
    n: usize,
    unit_id: crate::types::snowflake_id::SnowflakeId,
    kind: String,
    title: String,
    snippet: String,
    score: f32,
}

impl From<crate::kb::pipeline::Reference> for ReferenceDto {
    fn from(r: crate::kb::pipeline::Reference) -> Self {
        Self {
            n: r.n,
            unit_id: crate::types::snowflake_id::SnowflakeId(r.unit_id),
            kind: r.kind,
            title: r.title,
            snippet: r.snippet,
            score: r.score,
        }
    }
}

async fn public_ask(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<AskRequestDto>,
) -> AppResult<axum::response::Response> {
    use axum::response::sse::{Event as SseEvent, Sse};

    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let ask = crate::kb::pipeline::AskRequest {
        kb_ids: req.kb_ids.unwrap_or_default(),
        question: req.question,
    };
    let user_id = auth.user_id().map(crate::types::snowflake_id::SnowflakeId);

    let mut outcome = crate::kb::pipeline::prepare_answer(&deps, &ask).await?;

    if req.stream {
        // SSE: token deltas, then a final `result` event with status,
        // references and the log id [抄RF:agent/handler.rs SSE 模式].
        let (tx, rx) =
            tokio::sync::mpsc::unbounded_channel::<Result<SseEvent, std::convert::Infallible>>();
        let deps = deps;
        tokio::spawn(async move {
            let mut on_delta = |delta: &str| {
                let _ = tx.send(Ok(SseEvent::default().event("delta").data(delta)));
            };
            let result =
                crate::kb::pipeline::finish_answer_streaming(&deps, &mut outcome, &mut on_delta)
                    .await;
            let log_id = log_ask(&deps, &ask, &outcome, user_id).await;
            let payload = match result {
                Ok(()) => serde_json::json!({
                    "status": outcome.status,
                    "references": outcome.references.iter().cloned().map(ReferenceDto::from).collect::<Vec<_>>(),
                    "log_id": log_id.map(i64::from),
                }),
                Err(e) => serde_json::json!({ "status": "error", "error": e.to_string() }),
            };
            let _ = tx.send(Ok(SseEvent::default()
                .event("result")
                .data(payload.to_string())));
        });
        let stream = tokio_stream::wrappers::UnboundedReceiverStream::new(rx);
        Ok(Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::default())
            .into_response())
    } else {
        crate::kb::pipeline::finish_answer(&deps, &mut outcome).await?;
        let log_id = log_ask(&deps, &ask, &outcome, user_id).await;
        let body = serde_json::json!({
            "status": outcome.status,
            "answer": outcome.answer,
            "references": outcome.references.iter().cloned().map(ReferenceDto::from).collect::<Vec<_>>(),
            "log_id": log_id.map(i64::from),
        });
        Ok(ApiResponse::success(body).into_response())
    }
}

/// Persist the query log row (§9 feedback data plane).
async fn log_ask(
    deps: &crate::kb::service::KbDeps,
    ask: &crate::kb::pipeline::AskRequest,
    outcome: &crate::kb::pipeline::AskOutcome,
    user_id: Option<crate::types::snowflake_id::SnowflakeId>,
) -> Option<crate::types::snowflake_id::SnowflakeId> {
    let cited: serde_json::Value = serde_json::json!(
        outcome
            .references
            .iter()
            .map(|r| r.unit_id)
            .collect::<Vec<i64>>()
    );
    let kb_id = ask
        .kb_ids
        .first()
        .copied()
        .map(crate::types::snowflake_id::SnowflakeId);
    crate::kb::models::query_log::insert_log(
        &deps.pool,
        kb_id,
        &ask.question,
        Some(&outcome.answer),
        Some(&cited),
        outcome.status,
        Some(outcome.top_score),
        user_id,
    )
    .await
    .ok()
}

#[derive(Deserialize)]
struct SearchRequestDto {
    query: String,
    #[serde(default)]
    kb_ids: Option<Vec<i64>>,
    #[serde(default)]
    top_k: Option<u32>,
}

/// Unit-level search without generation — the retrieval seam agents consume
/// (§10; [抄WK:agent/tools/knowledge_search.go 模式]).
async fn public_search(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SearchRequestDto>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let ask = crate::kb::pipeline::AskRequest {
        kb_ids: req.kb_ids.unwrap_or_default(),
        question: req.query,
    };
    let mut outcome = crate::kb::pipeline::prepare_answer(&deps, &ask).await?;
    if let Some(k) = req.top_k {
        outcome.context_units.truncate(k as usize);
    }
    let items: Vec<serde_json::Value> = outcome
        .context_units
        .iter()
        .map(|u| {
            serde_json::json!({
                "unit_id": u.unit_id,
                "kind": u.kind,
                "title": u.title,
                "score": u.score,
                "is_faq": u.is_faq,
                "content": u.content,
            })
        })
        .collect();
    Ok(ApiResponse::success(serde_json::json!({
        "items": items,
        "status": outcome.status,
        "top_score": outcome.top_score,
    })))
}

// ── wiki governance (M4) ───────────────────────────────────────────

#[derive(Deserialize)]
struct DistillRequest {
    kb_id: SnowflakeId,
    doc_ids: Vec<i64>,
}

async fn admin_distill_wiki(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<DistillRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let _ = state.kb_deps()?;
    // Distillation is slow and LLM-heavy: run as a low-priority job (§7
    // concurrency governance).
    let mut new_job = crate::worker::NewJob::from(crate::worker::Job::KbDistillWiki {
        kb_id: req.kb_id,
        doc_ids: req.doc_ids,
        tenant_id: tenant_of(&auth),
    });
    new_job.priority = -5; // slow LLM work must not starve online jobs (§7)
    let queue = crate::worker::DefaultJobQueue::new(state.pool.clone());
    queue.enqueue(new_job).await?;
    Ok(ApiResponse::success(json!({ "queued": true })))
}

async fn admin_list_wiki_pages(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let kb_id: i64 = q
        .get("kb_id")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| AppError::BadRequest("kb_id required".into()))?;
    let page: i64 = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(1);
    let page_size: i64 = q
        .get("page_size")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let (pages, total) = knowledge_base_list(
        &state.pool,
        SnowflakeId(kb_id),
        q.get("status").map(String::as_str),
        page,
        page_size,
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "items": pages, "total": total, "page": page, "page_size": page_size }),
    ))
}

async fn knowledge_base_list(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    status: Option<&str>,
    page: i64,
    page_size: i64,
    tenant: &str,
) -> AppResult<(Vec<serde_json::Value>, i64)> {
    let (pages, total) =
        crate::kb::models::wiki_page::list_pages(pool, kb_id, status, page, page_size, tenant)
            .await?;
    let items = pages
        .into_iter()
        .map(|p| {
            json!({
                "id": p.id, "title": p.title, "slug": p.slug, "status": p.status,
                "summary": p.summary, "current_revision": p.current_revision,
                "updated_at": p.updated_at,
            })
        })
        .collect();
    Ok((items, total))
}

async fn admin_get_wiki_page(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let page = crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant_of(&auth))
        .await?
        .ok_or_else(|| AppError::NotFound("kb_wiki_page".into()))?;
    Ok(ApiResponse::success(json!({ "page": page })))
}

#[derive(Deserialize)]
struct EditWikiPageRequest {
    content: String,
    #[serde(default)]
    summary: Option<String>,
}

async fn admin_edit_wiki_page(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<EditWikiPageRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    let Some(existing) =
        crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant).await?
    else {
        return Err(AppError::NotFound("kb_wiki_page".into()));
    };
    // Snapshot the pre-edit content (D2 revision discipline).
    let snapshot = json!({ "title": existing.title, "slug": existing.slug, "content": existing.content,
        "summary": existing.summary, "status": existing.status });
    insert_wiki_snapshot(
        &state.pool,
        id,
        existing.current_revision,
        &snapshot,
        &tenant,
    )
    .await?;
    let status = existing.status.as_str();
    let next_status = if status == "published" {
        "published"
    } else {
        "draft"
    };
    crate::kb::models::wiki_page::update_page_content(
        &state.pool,
        id,
        &req.content,
        req.summary.as_deref(),
        next_status,
        &tenant,
    )
    .await?;
    // Published pages re-index immediately after edits.
    if next_status == "published" {
        let page = crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant)
            .await?
            .ok_or_else(|| AppError::NotFound("kb_wiki_page".into()))?;
        crate::kb::distill::publish_page(
            &deps,
            id,
            SnowflakeId(auth.user_id().unwrap_or_default()),
            &tenant,
        )
        .await?;
        state
            .emitter
            .emit(crate::event::Event::KbWikiPageUpdated(page));
    }
    Ok(ApiResponse::success(
        json!({ "id": id, "status": next_status }),
    ))
}

async fn insert_wiki_snapshot(
    pool: &crate::db::Pool,
    record_id: SnowflakeId,
    revision_number: i64,
    snapshot: &Value,
    _tenant: &str,
) -> AppResult<()> {
    raisfast_derive::crud_insert!(
        pool,
        "content_revisions",
        [
            "content_type" => "kb_wiki_page",
            "record_id" => record_id,
            "revision_number" => revision_number,
            "snapshot" => snapshot.clone(),
            "created_by" => SnowflakeId(0)
        ]
    )?;
    Ok(())
}

async fn admin_approve_wiki_page(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    let reviewer = SnowflakeId(auth.user_id().unwrap_or_default());
    crate::kb::distill::publish_page(&deps, id, reviewer, &tenant).await?;
    let page = crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant)
        .await?
        .ok_or_else(|| AppError::NotFound("kb_wiki_page".into()))?;
    state
        .emitter
        .emit(crate::event::Event::KbWikiPagePublished(page.clone()));
    Ok(ApiResponse::success(
        json!({ "id": page.id, "status": page.status, "revision": page.current_revision }),
    ))
}

async fn admin_reject_wiki_page(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    crate::kb::models::wiki_page::set_page_status(&state.pool, id, "archived", &tenant).await?;
    Ok(ApiResponse::success(
        json!({ "id": id, "status": "archived" }),
    ))
}

async fn admin_wiki_revisions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let (revisions, total) =
        crate::services::content_revision::list_revisions(&state.pool, "kb_wiki_page", id, 1, 50)
            .await?;
    Ok(ApiResponse::success(
        json!({ "items": revisions, "total": total }),
    ))
}

async fn admin_wiki_diff(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, a, b)): Path<(String, String, String)>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    // a/b are either revision ids or the literal "current".
    let tenant = tenant_of(&auth);
    let current = crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant)
        .await?
        .ok_or_else(|| AppError::NotFound("kb_wiki_page".into()))?;
    // Line-level diff via similar (§7); "current" compares against the
    // live page content, numeric ids against revision snapshots.
    let rev_a: Option<String> = if a == "current" {
        None
    } else {
        Some(a.clone())
    };
    let rev_b: Option<String> = if b == "current" {
        None
    } else {
        Some(b.clone())
    };
    let old_content = match rev_a {
        None => current.content.clone(),
        Some(rid) => revision_content(&state.pool, id, &rid).await?,
    };
    let new_content = match rev_b {
        None => current.content.clone(),
        Some(rid) => revision_content(&state.pool, id, &rid).await?,
    };
    let lines = crate::kb::distill::line_diff(&old_content, &new_content);
    Ok(ApiResponse::success(json!({ "lines": lines })))
}

async fn revision_content(
    pool: &crate::db::Pool,
    record_id: SnowflakeId,
    revision_id: &str,
) -> AppResult<String> {
    let rid: i64 = revision_id
        .parse()
        .map_err(|_| AppError::BadRequest("revision id must be numeric".into()))?;
    let rev = crate::services::content_revision::get_revision(
        pool,
        "kb_wiki_page",
        record_id,
        SnowflakeId(rid),
    )
    .await
    .map_err(|_| AppError::NotFound("content_revision".into()))?;
    Ok(rev
        .snapshot
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string())
}

#[derive(Deserialize)]
struct EditChunkRequest {
    content: String,
}

/// Chunk editing with revision + automatic re-embedding/re-indexing
/// [抄WK:CHANGELOG v0.7.2 chunk 编辑+版本+自动重索引].
async fn admin_edit_chunk(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<EditChunkRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let _tenant = tenant_of(&auth);
    let Some(chunk) = crate::kb::models::chunk::find_chunk_by_id(&state.pool, id).await? else {
        return Err(AppError::NotFound("kb_chunk".into()));
    };
    // Snapshot the pre-edit content.
    let snapshot =
        json!({ "content": chunk.content, "kind": chunk.kind, "breadcrumb": chunk.breadcrumb });
    raisfast_derive::crud_insert!(
        &state.pool,
        "content_revisions",
        [
            "content_type" => "kb_chunk",
            "record_id" => id,
            "revision_number" => 1_i64,
            "snapshot" => snapshot,
            "created_by" => SnowflakeId(auth.user_id().unwrap_or_default())
        ]
    )?;
    // Update content + re-embed + re-index both paths.
    crate::kb::models::chunk::update_content(&state.pool, id, &req.content).await?;
    let vectors = deps.embedder.embed(&[req.content.as_str()]).await?;
    let vector = vectors.first().cloned().unwrap_or_default();
    crate::kb::models::chunk::update_embedding(&state.pool, id, &vector).await?;
    deps.vector
        .upsert(
            i64::from(chunk.kb_id),
            vector.len() as u32,
            &[crate::kb::vectors::VectorItem {
                unit_id: i64::from(id),
                kb_id: i64::from(chunk.kb_id),
                kind: chunk.kind.clone(),
                embedding: vector,
            }],
        )
        .await?;
    let doc_key = chunk.doc_id.map(i64::from).unwrap_or_else(|| i64::from(id));
    deps.kbsearch
        .reindex_document(&[crate::kb::kbsearch::KbIndexUnit {
            unit_id: i64::from(id),
            kb_id: i64::from(chunk.kb_id),
            doc_id: doc_key,
            kind: chunk.kind.clone(),
            text: req.content.clone(),
        }])
        .await?;
    Ok(ApiResponse::success(json!({ "id": id, "reindexed": true })))
}

// ── FAQ management + sedimentation loop (M5, §8/§9) ────────────────

#[derive(Deserialize)]
struct CreateFaqRequest {
    kb_id: SnowflakeId,
    standard_question: String,
    #[serde(default)]
    similar_questions: Vec<String>,
    answers: Vec<String>,
    #[serde(default = "default_true_faq")]
    enabled: bool,
}

fn default_true_faq() -> bool {
    true
}

async fn admin_create_faq(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateFaqRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let faq = crate::kb::models::faq::create_faq(
        &state.pool,
        &crate::kb::models::faq::CreateFaqCmd {
            kb_id: req.kb_id,
            standard_question: req.standard_question,
            similar_questions: req.similar_questions,
            answers: req.answers,
            enabled: req.enabled,
            created_by: auth.user_id().map(SnowflakeId),
        },
        &tenant_of(&auth),
    )
    .await?;
    if faq.enabled {
        crate::kb::service::index_faq(&deps, &faq).await?;
    }
    Ok(ApiResponse::success(
        json!({ "id": faq.id, "enabled": faq.enabled }),
    ))
}

async fn admin_delete_faq(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let Some(faq) = crate::kb::models::faq::find_faqs_by_ids(&state.pool, &[i64::from(id)])
        .await?
        .into_iter()
        .next()
    else {
        return Err(AppError::NotFound("kb_faq".into()));
    };
    crate::kb::service::deindex_faq(&deps, id, faq.kb_id).await?;
    crate::kb::models::faq::delete_faq(&state.pool, id, &tenant_of(&auth)).await?;
    Ok(ApiResponse::success(json!({ "deleted": true })))
}

#[derive(Deserialize)]
struct SetFaqEnabledRequest {
    enabled: bool,
}

async fn admin_set_faq_enabled(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetFaqEnabledRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    crate::kb::models::faq::set_faq_enabled(&state.pool, id, req.enabled, &tenant_of(&auth))
        .await?;
    let Some(faq) = crate::kb::models::faq::find_faqs_by_ids(&state.pool, &[i64::from(id)])
        .await?
        .into_iter()
        .next()
    else {
        return Err(AppError::NotFound("kb_faq".into()));
    };
    // enable → index; disable → withdraw from retrieval.
    if req.enabled {
        crate::kb::service::index_faq(&deps, &faq).await?;
    } else {
        crate::kb::service::deindex_faq(&deps, id, faq.kb_id).await?;
    }
    Ok(ApiResponse::success(
        json!({ "id": id, "enabled": req.enabled }),
    ))
}

/// Draft a FAQ from a logged query (LLM-assisted, disabled until human
/// review — P1: nothing auto-writes the human-guaranteed layer).
async fn admin_faq_from_log(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(log_id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let log_id = parse_snowflake(&log_id)?;
    let Some(log) = crate::kb::models::query_log::find_log_by_id(&state.pool, log_id).await? else {
        return Err(AppError::NotFound("kb_query_log".into()));
    };
    let provider = deps
        .provider
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("kb chat provider unavailable".into()))?;
    let model = deps.config.ai.model.as_deref().unwrap_or_default();
    let messages = vec![
        raisfast_agent::ChatMessage {
            role: raisfast_agent::ChatRole::System,
            content: Some(
                "你是 FAQ 编辑。根据用户问题与历史回答草拟一条 FAQ。只输出 JSON：\
             {\"question\":\"标准问\",\"answer\":\"标准答\"}。"
                    .to_string(),
            ),
            tool_calls: None,
            tool_call_id: None,
        },
        raisfast_agent::ChatMessage {
            role: raisfast_agent::ChatRole::User,
            content: Some(format!(
                "问题：{}
历史回答：{}",
                log.question,
                log.answer.unwrap_or_default()
            )),
            tool_calls: None,
            tool_call_id: None,
        },
    ];
    let request = raisfast_agent::ChatRequest {
        messages: &messages,
        tools: None,
        temperature: Some(0.2),
        max_tokens: Some(400),
        stop: None,
    };
    let reply = provider
        .chat(&request, model)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("faq draft: {e}")))?;
    let text = reply.text.unwrap_or_default();
    let start = text.find('{');
    let end = text.rfind('}');
    let (Some(start), Some(end)) = (start, end) else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "LLM FAQ draft not JSON: {text}"
        )));
    };
    let draft: Value = serde_json::from_str(&text[start..=end])
        .map_err(|e| AppError::Internal(anyhow::anyhow!("FAQ draft parse: {e}")))?;
    let question = draft
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let answer = draft
        .get("answer")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if question.is_empty() || answer.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "FAQ draft missing fields"
        )));
    }
    let kb_id = log.kb_id.unwrap_or(SnowflakeId(0));
    let faq = crate::kb::models::faq::create_faq(
        &state.pool,
        &crate::kb::models::faq::CreateFaqCmd {
            kb_id,
            standard_question: question.to_string(),
            similar_questions: vec![log.question.clone()],
            answers: vec![answer.to_string()],
            enabled: false, // human gate (P1)
            created_by: auth.user_id().map(SnowflakeId),
        },
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "id": faq.id, "enabled": false, "review_required": true }),
    ))
}

async fn admin_kb_gaps(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let gaps = crate::kb::models::query_log::list_gaps(&state.pool, 50).await?;
    let items: Vec<Value> = gaps
        .into_iter()
        .map(|(question, count)| json!({ "question": question, "count": count }))
        .collect();
    Ok(ApiResponse::success(json!({ "items": items })))
}
