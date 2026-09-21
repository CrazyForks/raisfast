//! KB HTTP handlers — thin layer (kb-technical-design §10).
//!
//! `/api/v1/admin/kb/*` are admin-scoped governance endpoints (upload,
//! online entry, list/get/delete, reparse, chunker preview, rebuild).

use axum::Json;
use axum::extract::{Multipart, Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row as _;

use crate::AppState;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::kb::chunker::{self, ChunkerConfig};
use crate::kb::models::{chunk, document, kb_run, knowledge_base};
use crate::kb::service::{self, KbDeps};
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::prompt_file::prompt_file;
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
        "/admin/kb/parser-engines",
        get,
        admin_parser_engines,
        "system",
        "admin/kb/knowledge-bases",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/document-conversions",
        post,
        admin_create_document_conversion,
        "system",
        "document-conversions",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/document-conversions/{id}",
        get,
        admin_get_document_conversion,
        "system",
        "document-conversions",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/image-recognitions",
        post,
        admin_create_image_recognition,
        "system",
        "image-recognitions",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/image-recognitions/{id}",
        get,
        admin_get_image_recognition,
        "system",
        "image-recognitions",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/logs",
        get,
        admin_list_parse_logs,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/engines",
        get,
        admin_list_docparse_engines,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/engines/{name}",
        put,
        admin_update_docparse_engine,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/tokens",
        get,
        admin_list_docparse_tokens,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/tokens",
        post,
        admin_create_docparse_token,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/tokens/{id}/disable",
        post,
        admin_disable_docparse_token,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/tokens/{id}/enable",
        post,
        admin_enable_docparse_token,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/docparse/tokens/{id}",
        delete,
        admin_delete_docparse_token,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/knowledge-bases/{id}",
        put,
        admin_update_kb,
        "system",
        "admin/kb/knowledge-bases",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/knowledge-bases/{id}",
        delete,
        admin_delete_kb,
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
        axum::routing::post(admin_upload_document)
            .layer(axum::extract::DefaultBodyLimit::max(config.max_upload_size)),
        "system",
        "admin/kb/documents",
        "admin",
        layered
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
        "/admin/kb/documents/{id}/jobs",
        get,
        admin_document_jobs,
        "system",
        "admin/kb/documents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/runs",
        get,
        admin_list_runs,
        "system",
        "admin/kb/runs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/runs/{id}",
        get,
        admin_get_run,
        "system",
        "admin/kb/runs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/health",
        get,
        crate::kb::diagnostics::admin_kb_health,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/diagnostics",
        get,
        crate::kb::diagnostics::admin_kb_diagnostics,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/index-stats",
        get,
        crate::kb::diagnostics::admin_kb_index_stats,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/vector-search-test",
        get,
        crate::kb::diagnostics::admin_kb_vector_search_test,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/bm25-search-test",
        get,
        crate::kb::diagnostics::admin_kb_bm25_search_test,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/chunk-keywords",
        get,
        crate::kb::diagnostics::admin_kb_chunk_keywords,
        "system",
        "admin/kb/diagnostics",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/vector/rebuild",
        post,
        crate::kb::diagnostics::admin_rebuild_vector,
        "system",
        "admin/kb/diagnostics",
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
        "/admin/kb/wiki/pages/{id}/restore/{rev}",
        post,
        admin_restore_wiki_page,
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
        "/admin/kb/chunks/{id}",
        get,
        admin_get_chunk,
        "system",
        "admin/kb/chunks",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/chunks/{id}",
        delete,
        admin_delete_chunk,
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
        "/admin/kb/faqs",
        get,
        admin_list_faqs,
        "system",
        "admin/kb/faqs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/faqs/{id}",
        put,
        admin_update_faq,
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
        "/admin/kb/documents/{id}/reader",
        get,
        admin_document_reader,
        "system",
        "kb/documents",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/images",
        get,
        admin_list_images,
        "system",
        "kb/images",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/images/{id}/preview",
        get,
        admin_image_preview,
        "system",
        "kb/images",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/chunks",
        get,
        admin_list_chunks,
        "system",
        "admin/kb/chunks",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        config.api_restful,
        "/admin/kb/stats",
        get,
        admin_kb_stats,
        "system",
        "admin/kb/stats",
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
    pub(crate) fn kb_deps(&self) -> AppResult<KbDeps> {
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
            reranker: kb.reranker.clone(),
            parsers: kb.parsers.clone(),
            router: self.llm_router.clone(),
            emitter: self.emitter.clone(),
        })
    }
}

// ── knowledge bases ────────────────────────────────────────────────

#[derive(Deserialize)]
struct CreateKbRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    slug: String,
    #[serde(default = "default_kb_kind")]
    kind: String,
    /// Embedding model pinned at creation (immutable afterwards, WeKnora
    /// vector_store_id precedent). Defaults to option llm.default_embedding_model.
    #[serde(default)]
    embedding_model: Option<String>,
    /// Dimension pinned with the model (or directory params.dimension).
    #[serde(default)]
    embedding_dim: Option<u32>,
    /// S5 rerank model for this KB (mutable afterwards — rerank is
    /// query-time behavior with nothing baked). Empty = global
    /// `RAISFAST_KB_RERANK_MODEL` default; neither = passthrough.
    #[serde(default)]
    rerank_model: Option<String>,
    /// Per-KB rerank window override (§6.1.4); null = global default.
    #[serde(default)]
    rerank_window: Option<u32>,
    /// Per-KB rerank score floor override (0..=1); null = global default.
    #[serde(default)]
    rerank_threshold: Option<f64>,
    /// S9 generation model for this KB (empty = global default → tenant
    /// default). Freely editable — query-time behavior, nothing baked.
    #[serde(default)]
    chat_model: Option<String>,
    /// Wiki distillation model for this KB (empty = global default →
    /// tenant default).
    #[serde(default)]
    distill_model: Option<String>,
    /// VLM image recognition config
    /// `{enabled, model, caption_language, custom_instructions}`.
    #[serde(default)]
    image_config: Option<serde_json::Value>,
    /// Parser engine routing rules `{"rules": [...]}` (validated against
    /// the registry; kb-parser-engines-design §2 D2).
    #[serde(default)]
    parser_config: Option<serde_json::Value>,
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
    validate_rerank_params(req.rerank_window, req.rerank_threshold)?;
    if let Some(pc) = &req.parser_config {
        let deps = state.kb_deps()?;
        crate::docparse::validate_parser_config(&deps.parsers, pc)?;
    }
    let tenant = auth.tenant_id();
    let model = resolve_kb_model(&state, tenant, &req).await?;
    let dim = resolve_kb_dim(&state, tenant, &model, &req).await?;
    let kb = knowledge_base::create_kb(
        &state.pool,
        &knowledge_base::CreateKbCmd {
            name: req.name,
            description: req.description,
            slug: req.slug,
            kind: req.kind,
            indexing_strategy: None,
            embedding_model: Some(model),
            embedding_dim: Some(i64::from(dim)),
            rerank_model: req.rerank_model.filter(|m| !m.is_empty()),
            rerank_window: req.rerank_window.map(i64::from),
            rerank_threshold: req.rerank_threshold,
            chat_model: req.chat_model.filter(|m| !m.is_empty()),
            distill_model: req.distill_model.filter(|m| !m.is_empty()),
            image_config: req.image_config,
            parser_config: req.parser_config,
        },
        &tenant_of(&auth),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "id": kb.id, "slug": kb.slug, "kind": kb.kind, "description": kb.description }),
    ))
}

#[derive(Deserialize)]
struct UpdateKbRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    slug: String,
    status: String,
    /// Rerank trio + generation model (freely editable — query-time
    /// behavior, §6.1).
    #[serde(default)]
    rerank_model: Option<String>,
    #[serde(default)]
    rerank_window: Option<u32>,
    #[serde(default)]
    rerank_threshold: Option<f64>,
    #[serde(default)]
    chat_model: Option<String>,
    #[serde(default)]
    distill_model: Option<String>,
    #[serde(default)]
    image_config: Option<serde_json::Value>,
    #[serde(default)]
    parser_config: Option<serde_json::Value>,
}

/// Update mutable KB metadata (name/slug/description/status) plus the
/// rerank trio. kind / embedding_model / embedding_dim are immutable after
/// creation.
async fn admin_update_kb(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateKbRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    state.kb_deps()?;
    if req.name.trim().is_empty() || req.slug.trim().is_empty() {
        return Err(AppError::BadRequest(
            "name and slug must not be empty".into(),
        ));
    }
    if !matches!(req.status.as_str(), "active" | "archived") {
        return Err(AppError::BadRequest(
            "status must be 'active' or 'archived'".into(),
        ));
    }
    validate_rerank_params(req.rerank_window, req.rerank_threshold)?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    if knowledge_base::find_kb_by_id(&state.pool, id, &tenant)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("kb_knowledge_base".into()));
    }
    knowledge_base::update_kb(
        &state.pool,
        id,
        &knowledge_base::UpdateKbCmd {
            name: req.name.trim().to_owned(),
            description: req
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            slug: req.slug.trim().to_owned(),
            status: req.status,
            rerank_model: req.rerank_model.filter(|m| !m.is_empty()),
            rerank_window: req.rerank_window.map(i64::from),
            rerank_threshold: req.rerank_threshold,
            chat_model: req.chat_model.filter(|m| !m.is_empty()),
            distill_model: req.distill_model.filter(|m| !m.is_empty()),
            image_config: req.image_config,
            parser_config: req.parser_config,
        },
        &tenant,
    )
    .await?;
    Ok(ApiResponse::success(json!({ "id": id })))
}

/// Shared validation for the rerank trio on create/update.
fn validate_rerank_params(window: Option<u32>, threshold: Option<f64>) -> AppResult<()> {
    if window.is_some_and(|w| w == 0) {
        return Err(AppError::BadRequest(
            "rerank_window must be a positive integer".into(),
        ));
    }
    if threshold.is_some_and(|t| !(0.0..=1.0).contains(&t)) {
        return Err(AppError::BadRequest(
            "rerank_threshold must be within 0..=1".into(),
        ));
    }
    Ok(())
}

/// Delete a KB and everything in it (documents, chunks, FAQs, wiki pages,
/// provenance links, vector/FTS indexes, original files).
async fn admin_delete_kb(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    if knowledge_base::find_kb_by_id(&state.pool, id, &tenant)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("kb_knowledge_base".into()));
    }
    crate::kb::service::delete_kb_everywhere(&deps, id, &tenant).await?;
    Ok(ApiResponse::success(json!({ "deleted": true })))
}

/// Per-KB model: request > 租户 options `llm.default_embedding_model`
/// （§10.2；模型访问唯一入口 = llm 底座）。model+dim 必须在创建时确定
/// （此后不可变，WeKnora `vector_store_id` precedent）。
async fn resolve_kb_model(
    state: &AppState,
    tenant: Option<&str>,
    req: &CreateKbRequest,
) -> AppResult<String> {
    if let Some(m) = req.embedding_model.clone().filter(|m| !m.is_empty()) {
        return Ok(m);
    }
    for scope in [tenant, None] {
        if let Some(row) =
            crate::models::options::find_by_key(&state.pool, "llm.default_embedding_model", scope)
                .await?
            && let Some(v) = row.value.as_str()
            && !v.is_empty()
        {
            return Ok(v.to_owned());
        }
    }
    Err(AppError::BadRequest(
        "embedding_model required: pass it (with embedding_dim), or set option \
         llm.default_embedding_model"
            .into(),
    ))
}

async fn resolve_kb_dim(
    state: &AppState,
    tenant: Option<&str>,
    model: &str,
    req: &CreateKbRequest,
) -> AppResult<u32> {
    if let Some(d) = req.embedding_dim.filter(|d| *d > 0) {
        return Ok(d);
    }
    // 目录 `params.dimension`（§5.4）。
    if let Some(info) = state
        .llm_router
        .model_info(tenant.unwrap_or("default"), model)
        && let Some(d) = info
            .params
            .as_ref()
            .and_then(|p| p.get("dimension"))
            .and_then(serde_json::Value::as_u64)
        && d > 0
    {
        return Ok(d as u32);
    }
    Err(AppError::BadRequest(
        "embedding_dim required (pass it, or set llm_models.params.dimension)".into(),
    ))
}

/// Registered parse engines (KB form dropdown source — engine names are
/// a server-side registry concern, never a frontend hardcoded list).
async fn admin_parser_engines(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let items: Vec<&str> = deps.parsers.names();
    Ok(ApiResponse::success(json!({ "items": items })))
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

// ── document conversions（独立文档转换服务，design: dev-docs/document/service-design.md）──

/// 提交转换 job（multipart: file 必填；engine / extract_images 可选）。
async fn admin_create_document_conversion(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<Value>> {
    // 转换/识别为对外服务（M3）：admin JWT 与平台 api-token 身份均可提交
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();

    let mut filename = String::new();
    let mut data: axum::body::Bytes = axum::body::Bytes::new();
    let mut engine: Option<String> = None;
    let mut extract_images = true;
    let mut callback_url: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart read failed: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "file" => {
                filename = field.file_name().unwrap_or("untitled").to_string();
                data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("file read failed: {e}")))?;
            }
            "engine" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("engine read failed: {e}")))?;
                engine = Some(v.trim().to_string()).filter(|s| !s.is_empty());
            }
            "extract_images" => {
                let v = field.text().await.map_err(|e| {
                    AppError::BadRequest(format!("extract_images read failed: {e}"))
                })?;
                extract_images = v.trim().eq_ignore_ascii_case("true");
            }
            "callback_url" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("callback_url read failed: {e}")))?;
                callback_url = Some(v.trim().to_string()).filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }
    if data.is_empty() {
        return Err(AppError::BadRequest("file field missing".into()));
    }

    let queue = crate::worker::DefaultJobQueue::new(state.pool.clone());
    let job_id = crate::docparse::conversion::submit(
        &state.pool,
        &deps.storage,
        &deps.parsers,
        &queue,
        &tenant,
        &filename,
        &data,
        engine.as_deref(),
        extract_images,
        callback_url.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "job_id": job_id, "status": "queued" }),
    ))
}

/// 轮询转换 job：meta.json 是状态文档；completed 时附 markdown 内联。
async fn admin_get_document_conversion(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let Some((mut meta, markdown)) =
        crate::docparse::conversion::read_status(&deps.storage, &id).await?
    else {
        return Err(AppError::NotFound("document_conversion".into()));
    };
    let _ = auth;
    if let Some(md) = markdown {
        meta["markdown"] = json!(md);
    }
    Ok(ApiResponse::success(meta))
}

// ── image recognitions（独立图像识别服务，M2）────────────────────────

/// 提交识别 job（multipart: image 必填；model / prompt 可选）。
async fn admin_create_image_recognition(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();

    let mut filename = String::new();
    let mut data: axum::body::Bytes = axum::body::Bytes::new();
    let mut model: Option<String> = None;
    let mut prompt: Option<String> = None;
    let mut callback_url: Option<String> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart read failed: {e}")))?
    {
        match field.name().unwrap_or_default() {
            "image" => {
                filename = field.file_name().unwrap_or("image.png").to_string();
                data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("image read failed: {e}")))?;
            }
            "model" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("model read failed: {e}")))?;
                model = Some(v.trim().to_string()).filter(|s| !s.is_empty());
            }
            "prompt" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("prompt read failed: {e}")))?;
                prompt = Some(v).filter(|s| !s.is_empty());
            }
            "callback_url" => {
                let v = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("callback_url read failed: {e}")))?;
                callback_url = Some(v.trim().to_string()).filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }
    if data.is_empty() {
        return Err(AppError::BadRequest("image field missing".into()));
    }

    let queue = crate::worker::DefaultJobQueue::new(state.pool.clone());
    let job_id = crate::docparse::recognition::submit(
        &state.pool,
        &deps.storage,
        &queue,
        &tenant,
        &filename,
        &data,
        model.as_deref(),
        prompt.as_deref(),
        callback_url.as_deref(),
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "job_id": job_id, "status": "queued" }),
    ))
}

async fn admin_get_image_recognition(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let Some(meta) = crate::docparse::recognition::read_status(&deps.storage, &id).await? else {
        return Err(AppError::NotFound("image_recognition".into()));
    };
    Ok(ApiResponse::success(meta))
}

/// 解析用量账本查询（ops）：按租户列最近的转换/识别 job。
#[derive(Deserialize)]
struct ParseLogsQuery {
    limit: Option<i64>,
}

async fn admin_list_parse_logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ParseLogsQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();
    let items = crate::docparse::logs::list_by_tenant(
        &deps.pool,
        &tenant,
        q.limit.unwrap_or(50).clamp(1, 500),
    )
    .await?;
    Ok(ApiResponse::success(json!({ "items": items })))
}

// ── docparse 引擎管理 ───────────────────────────────────────────────

async fn admin_list_docparse_engines(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();
    // Registry is the source of truth for which engines exist (and their
    // category, from the trait); per-tenant table rows are admin overrides
    // (enabled / pricing) [tenant-scoped pricing, aligned with llm_models].
    // Merging both ways also surfaces table-only engines (configured rows
    // whose engine isn't registered in this build).
    let sql = format!(
        "SELECT engine_name, enabled, price_per_page, price_per_call, cost_per_page, \
         COALESCE(category, 'document'), updated_at FROM docparse_engines WHERE tenant_id = {}",
        Driver::ph(1)
    );
    let rows: Vec<(String, bool, i64, i64, i64, String, String)> =
        sqlx::query_as(crate::db::safe_sql(&sql))
            .bind(&tenant)
            .fetch_all(&deps.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    let mut overrides: std::collections::HashMap<String, (bool, i64, i64, i64, String, String)> =
        rows.into_iter()
            .map(|(name, enabled, ppp, ppc, cost, cat, updated)| {
                (name, (enabled, ppp, ppc, cost, cat, updated))
            })
            .collect();

    let mut items: Vec<Value> = Vec::new();
    for name in deps.parsers.names() {
        let category = deps
            .parsers
            .get(name)
            .map(|e| e.category())
            .unwrap_or("document");
        let (enabled, ppp, ppc, cost, updated) = match overrides.remove(name) {
            Some((enabled, ppp, ppc, cost, _cat, updated)) => {
                (enabled, ppp, ppc, cost, Some(updated))
            }
            None => (true, 0, 0, 0, None),
        };
        items.push(json!({
            "engine_name": name, "enabled": enabled,
            "price_per_page": ppp, "price_per_call": ppc,
            "cost_per_page": cost, "registered": true,
            "category": category, "updated_at": updated,
        }));
    }
    let mut leftovers: Vec<_> = overrides.into_iter().collect();
    leftovers.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, (enabled, ppp, ppc, cost, category, updated)) in leftovers {
        items.push(json!({
            "engine_name": name, "enabled": enabled,
            "price_per_page": ppp, "price_per_call": ppc,
            "cost_per_page": cost, "registered": false,
            "category": category, "updated_at": updated,
        }));
    }
    Ok(ApiResponse::success(json!({ "items": items })))
}

#[derive(Deserialize)]
struct UpdateEngineBody {
    enabled: Option<bool>,
    price_per_page: Option<i64>,
    price_per_call: Option<i64>,
    cost_per_page: Option<i64>,
}

async fn admin_update_docparse_engine(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(body): Json<UpdateEngineBody>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();
    let sets: Vec<String> = [
        body.enabled
            .map(|v| format!("enabled = {}", if v { "TRUE" } else { "FALSE" })),
        body.price_per_page.map(|v| format!("price_per_page = {v}")),
        body.price_per_call.map(|v| format!("price_per_call = {v}")),
        body.cost_per_page.map(|v| format!("cost_per_page = {v}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if sets.is_empty() {
        return Err(AppError::BadRequest("no fields to update".into()));
    }
    let sql = format!(
        "UPDATE docparse_engines SET {}, updated_at = {} WHERE engine_name = {} AND tenant_id = {}",
        sets.join(", "),
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(crate::utils::tz::now_utc())
        .bind(&name)
        .bind(&tenant)
        .execute(&deps.pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    // First touch of a registry-known engine: the per-tenant override row
    // doesn't exist yet → INSERT it (category from the trait). MySQL has no
    // RETURNING; rows_affected decides update-vs-insert [AGENTS rule #10].
    if result.rows_affected() == 0 {
        let category = deps
            .parsers
            .get(&name)
            .map(|e| e.category())
            .unwrap_or("document");
        let insert = format!(
            "INSERT INTO docparse_engines \
             (id, tenant_id, engine_name, enabled, price_per_page, price_per_call, cost_per_page, category, updated_at) \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
            Driver::ph(1),
            Driver::ph(2),
            Driver::ph(3),
            Driver::ph(4),
            Driver::ph(5),
            Driver::ph(6),
            Driver::ph(7),
            Driver::ph(8),
            Driver::ph(9)
        );
        sqlx::query(crate::db::safe_sql(&insert))
            .bind(crate::utils::id::new_id())
            .bind(&tenant)
            .bind(&name)
            .bind(body.enabled.unwrap_or(true))
            .bind(body.price_per_page.unwrap_or(0))
            .bind(body.price_per_call.unwrap_or(0))
            .bind(body.cost_per_page.unwrap_or(0))
            .bind(category)
            .bind(crate::utils::tz::now_utc())
            .execute(&deps.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    }
    Ok(ApiResponse::success(json!({ "updated": true })))
}

// ── docparse token 管理 ─────────────────────────────────────────────

async fn admin_create_docparse_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();
    let name = q.get("name").cloned().unwrap_or_else(|| "default".into());
    let quota = q
        .get("daily_page_quota")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    // Owner: explicit user_id (admin acting on behalf, ID_ENCODING-aware —
    // UserPicker submits encoded ids) or the caller [照抄 llm
    // admin_create_token 的 parse_id + owner 存在性校验]。
    let user_id = match q.get("user_id") {
        Some(raw) if !raw.trim().is_empty() => {
            let id = parse_snowflake(raw)?;
            crate::models::user::find_by_id(&state.pool, id, Some(&tenant))
                .await?
                .ok_or_else(|| AppError::BadRequest(format!("user not found: {raw}")))?
                .id
        }
        _ => auth.ensure_snowflake_user_id()?,
    };
    let (raw, id) =
        crate::docparse::tokens::create(&deps.pool, &tenant, user_id, &name, quota).await?;
    Ok(ApiResponse::success(json!({
        "id": id, "user_id": user_id, "token": raw, "name": name,
        "daily_page_quota": quota, "status": "active",
    })))
}

async fn admin_list_docparse_tokens(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = auth.tenant_id().unwrap_or("default").to_string();
    let rows = crate::docparse::tokens::list_by_tenant(&deps.pool, &tenant).await?;
    // Resolve owner usernames (admin list view) [照抄 llm token list]。
    let ids: Vec<SnowflakeId> = rows.iter().map(|r| r.user_id).collect();
    let names = crate::models::user::find_usernames_by_ids(&deps.pool, &ids).await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|t| {
            json!({
                "id": t.id,
                "user_id": t.user_id,
                "username": names.get(&t.user_id.0),
                "name": t.name,
                "token_prefix": t.token_prefix,
                // Reversible copy for the admin UI; NULL when token_enc was
                // never stored (legacy rows) or decryption fails.
                "token": t.token_enc.as_deref().and_then(crate::llm::crypto::decrypt),
                "status": t.status,
                "daily_page_quota": t.daily_page_quota,
                "created_at": t.created_at,
                "last_used_at": t.last_used_at,
            })
        })
        .collect();
    Ok(ApiResponse::success(json!({ "items": items })))
}

async fn admin_disable_docparse_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    crate::docparse::tokens::set_status(&state.pool, id, "disabled").await?;
    Ok(ApiResponse::success(json!({ "disabled": true })))
}

async fn admin_enable_docparse_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    crate::docparse::tokens::set_status(&state.pool, id, "active").await?;
    Ok(ApiResponse::success(json!({ "enabled": true })))
}

async fn admin_delete_docparse_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    crate::docparse::tokens::delete(&state.pool, id).await?;
    Ok(ApiResponse::success(json!({ "deleted": true })))
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
    let kb_id: SnowflakeId = parse_snowflake(
        &field
            .text()
            .await
            .map_err(|e| AppError::BadRequest(format!("kb_id read failed: {e}")))?,
    )?;

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
    // field 3 (optional): parser_engine override.
    let parser_engine: Option<String> = match multipart.next_field().await {
        Ok(Some(f)) => Some(
            f.text()
                .await
                .map_err(|e| AppError::BadRequest(format!("parser_engine read failed: {e}")))?,
        ),
        _ => None,
    };

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
    if let Some(engine) = parser_engine.as_deref().filter(|e| !e.is_empty()) {
        crate::kb::models::document::set_document_parser_engine(
            &deps.pool,
            doc.id,
            Some(engine),
            &tenant_of(&auth),
        )
        .await?;
    }
    Ok(ApiResponse::success(
        json!({ "id": doc.id, "status": doc.status, "title": doc.title }),
    ))
}

#[derive(Deserialize)]
struct OnlineDocRequest {
    /// Optional per-document parser engine override (unknown names warn +
    /// fall back at route time; kb-parser-engines-design §2 D2 ⓪).
    #[serde(default)]
    parser_engine: Option<String>,

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
    if let Some(engine) = req.parser_engine.as_deref().filter(|e| !e.is_empty()) {
        crate::kb::models::document::set_document_parser_engine(
            &deps.pool,
            doc.id,
            Some(engine),
            &tenant_of(&auth),
        )
        .await?;
    }
    Ok(ApiResponse::success(
        json!({ "id": doc.id, "status": doc.status, "title": doc.title }),
    ))
}

/// Reader view (质量验证): page-organized leaf chunks with recognized
/// images interleaved — inspect what the RAG actually ingested.
#[derive(Deserialize)]
struct ReaderQuery {
    page: Option<i64>,
}

async fn admin_document_reader(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<ReaderQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    let doc = crate::kb::models::document::find_document_by_id(&state.pool, id, &tenant)
        .await?
        .ok_or_else(|| AppError::NotFound("kb_document".into()))?;
    // 页参数：?page=N 显式翻页；缺省 = 第一页（最小页码，无锚点为 NULL 组）。
    let page = match q.page {
        Some(p) => Some(p),
        None => crate::kb::models::chunk::reader_first_page(&state.pool, id)
            .await?
            .or(Some(1)),
    };
    let view = crate::kb::models::chunk::reader_page(&state.pool, id, page).await?;
    Ok(ApiResponse::success(json!({
        "doc": {
            "id": doc.id, "title": doc.title, "pages": doc.pages,
            "parse_degraded": doc.parse_degraded, "parser_engine": doc.parser_engine,
        },
        "page": view.page, "total_pages": view.total_pages,
        "blocks": view.blocks, "index": view.index,
    })))
}

#[derive(Deserialize)]
struct ListImagesQuery {
    kb_id: Option<SnowflakeId>,
    doc_id: Option<SnowflakeId>,
    status: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

/// Admin image listing (kb/doc/status filters + pagination, names joined).
async fn admin_list_images(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListImagesQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(20);
    let (items, total) = crate::kb::models::image::list_images(
        &state.pool,
        &tenant_of(&auth),
        q.kb_id,
        q.doc_id,
        q.status.as_deref(),
        page,
        page_size,
    )
    .await?;
    Ok(ApiResponse::success(
        json!({ "items": items, "total": total, "page": page, "page_size": page_size }),
    ))
}

/// Inline image preview: embedded bytes from storage; external URLs
/// redirect (same shape as media `serve_file`).
async fn admin_image_preview(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<axum::response::Response> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let img = crate::kb::models::image::find_image_by_id(&state.pool, id, &tenant_of(&auth))
        .await?
        .ok_or_else(|| AppError::NotFound("kb_image".into()))?;
    if let Some(url) = img
        .original_url
        .as_deref()
        .filter(|u| u.starts_with("http"))
    {
        return Ok(axum::response::Redirect::temporary(url).into_response());
    }
    let key = img
        .storage_key
        .as_deref()
        .ok_or_else(|| AppError::NotFound("kb_image bytes".into()))?;
    let bytes = state.storage.get(key).await?;
    axum::http::Response::builder()
        .header("Content-Type", img.mime_type)
        .header("Content-Length", bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("image preview: {e}")))
}

#[derive(Deserialize)]
struct ListDocumentsQuery {
    kb_id: Option<SnowflakeId>,
    status: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

async fn admin_list_documents(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListDocumentsQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(20);
    let (docs, total) = document::list_documents(
        &state.pool,
        q.kb_id,
        q.status.as_deref(),
        page,
        page_size,
        &tenant_of(&auth),
    )
    .await?;
    // Per-doc image counts in one grouped query (list display: recognition
    // progress visibility).
    let doc_ids: Vec<i64> = docs.iter().map(|d| i64::from(d.id)).collect();
    let mut image_counts: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    if !doc_ids.is_empty() {
        let placeholders: Vec<String> = (1..=doc_ids.len()).map(crate::db::Driver::ph).collect();
        let sql = format!(
            "SELECT doc_id, {} FROM kb_images WHERE doc_id IN ({}) GROUP BY doc_id",
            crate::db::Driver::cast_int("COUNT(*)"),
            placeholders.join(", ")
        );
        let mut query = sqlx::query_as::<_, (i64, i64)>(crate::db::safe_sql(&sql));
        for id in &doc_ids {
            query = query.bind(id);
        }
        for (doc_id, n) in query
            .fetch_all(&state.pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?
        {
            image_counts.insert(doc_id, n);
        }
    }
    let items: Vec<Value> = docs
        .into_iter()
        .map(|d| {
            let images = image_counts.get(&i64::from(d.id)).copied().unwrap_or(0);
            let mut v = serde_json::to_value(&d).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert("images".into(), json!(images));
            }
            v
        })
        .collect();
    Ok(ApiResponse::success(
        json!({ "items": items, "total": total, "page": page, "page_size": page_size }),
    ))
}

/// Processing history for one document: its `kb_process_document` job runs
/// (attempts, errors, timestamps) — admin observability. Payload doc_id may
/// be plain or encoded depending on ID_ENCODING at write time, so match both.
async fn admin_document_jobs(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let raw = i64::from(id);
    let plain = format!("%\"doc_id\":\"{raw}\"%");
    let encoded = format!(
        "%\"doc_id\":\"{}\"%",
        crate::types::snowflake_id::encode_id(raw)
    );
    let rows = sqlx::query(crate::db::safe_sql(&format!(
        "SELECT id, status, attempts, max_attempts, error, created_at, updated_at \
         FROM jobs WHERE job_type = 'kb_process_document' \
         AND (payload LIKE {} OR payload LIKE {}) \
         ORDER BY created_at DESC LIMIT 20",
        Driver::ph(1),
        Driver::ph(2)
    )))
    .bind(&plain)
    .bind(&encoded)
    .fetch_all(&state.pool)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("job history query failed: {e}")))?;
    let items: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": r.try_get::<i64, _>("id").unwrap_or_default(),
                "status": r.try_get::<String, _>("status").unwrap_or_default(),
                "attempts": r.try_get::<i64, _>("attempts").unwrap_or_default(),
                "max_attempts": r.try_get::<i64, _>("max_attempts").unwrap_or_default(),
                "error": r.try_get::<Option<String>, _>("error").ok().flatten(),
                "created_at": r.try_get::<crate::utils::tz::Timestamp, _>("created_at").ok(),
                "updated_at": r.try_get::<crate::utils::tz::Timestamp, _>("updated_at").ok(),
            })
        })
        .collect();
    Ok(ApiResponse::success(
        json!({ "items": items, "total": items.len() }),
    ))
}

// ── run records (kb-observability-design T1: 运行记录) ─────────────

#[derive(Deserialize)]
struct RunsQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    kb_id: Option<String>,
    #[serde(default)]
    doc_id: Option<String>,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    page_size: Option<i64>,
}

/// Optional id query param → i64 (ID_ENCODING-aware; garbage → 400).
fn parse_id_query(raw: &Option<String>) -> AppResult<Option<i64>> {
    match raw.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None | Some("") => Ok(None),
        Some(s) => Ok(Some(i64::from(parse_snowflake(s)?))),
    }
}

async fn admin_list_runs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<RunsQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let tenant = tenant_of(&auth);
    let filter = kb_run::RunFilter {
        kind: q.kind.filter(|s| !s.is_empty()),
        kb_id: parse_id_query(&q.kb_id)?,
        doc_id: parse_id_query(&q.doc_id)?,
        agent_id: parse_id_query(&q.agent_id)?,
        session_id: parse_id_query(&q.session_id)?,
        status: q.status.filter(|s| !s.is_empty()),
    };
    let (runs, total) = kb_run::list_runs(
        &state.pool,
        &tenant,
        &filter,
        q.page.unwrap_or(1),
        q.page_size.unwrap_or(20),
    )
    .await?;
    Ok(ApiResponse::success(json!({
        "items": runs,
        "total": total,
        "page": q.page.unwrap_or(1),
        "page_size": q.page_size.unwrap_or(20),
    })))
}

async fn admin_get_run(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let run = kb_run::find_run_by_id(&state.pool, id, &tenant_of(&auth))
        .await?
        .ok_or_else(|| AppError::NotFound("kb_run".into()))?;
    Ok(ApiResponse::success(json!({ "run": run })))
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
    let tenant = tenant_of(&auth);
    // W4 fix [kb-observability-design §1.1]: the sync path used to `?` the
    // error away, leaving the doc in an intermediate status with no error.
    // Now it mirrors the job path — failed status + error + a traced run.
    let mode = crate::kb::trace::TraceMode::parse(&deps.config.kb.trace_mode);
    let prior = crate::kb::models::kb_run::count_runs(
        &state.pool,
        crate::kb::models::kb_run::KIND_INGEST_DOC,
        Some(id),
        None,
    )
    .await
    .unwrap_or(0);
    let mut trace = crate::kb::trace::RunRecorder::create(
        mode,
        crate::kb::trace::RunSpec {
            kind: crate::kb::models::kb_run::KIND_INGEST_DOC,
            trigger_src: "admin",
            tenant_id: tenant.clone(),
            kb_id: None,
            doc_id: Some(id),
            agent_id: None,
            session_id: None,
            job_id: None,
            attempt: prior + 1,
            long_running: true,
        },
    );
    let result = service::process_document_traced(&deps, id, &tenant, &mut trace).await;
    if let Err(e) = &result {
        let _ = document::set_document_status(
            &state.pool,
            id,
            "failed",
            Some(&e.to_string()),
            None,
            &tenant,
        )
        .await;
    }
    result?;
    let doc = document::find_document_by_id(&state.pool, id, &tenant)
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
    // ID_ENCODING-aware: accepts both plain and encoded ids
    // [抄RF:handlers/cart.rs parse_id 用法——AGENTS.md SnowflakeId 纪律].
    crate::types::snowflake_id::parse_id(raw)
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
    kb_ids: Option<Vec<SnowflakeId>>,
    /// Document scope (playground); empty = whole KB scope.
    #[serde(default)]
    doc_ids: Option<Vec<SnowflakeId>>,
    #[serde(default)]
    stream: bool,
    /// Admin-only: include the pipeline trace (stages) in the response
    /// [kb-observability-design §5] — ignored for non-admin callers.
    #[serde(default)]
    trace: Option<bool>,
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
        tenant_id: tenant_of(&auth),
        kb_ids: req.kb_ids.unwrap_or_default().iter().map(|i| i.0).collect(),
        doc_ids: req
            .doc_ids
            .unwrap_or_default()
            .iter()
            .map(|i| i.0)
            .collect(),
        question: req.question,
    };
    let user_id = auth.user_id().map(crate::types::snowflake_id::SnowflakeId);
    // Trace exposure is admin-only (§5: avoid leaking retrieval internals).
    let want_trace = req.trace.unwrap_or(false) && auth.ensure_admin().is_ok();
    let started = std::time::Instant::now();

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
            let latency = started.elapsed().as_millis() as i64;
            let trace_payload = if want_trace {
                outcome.trace.stages_snapshot()
            } else {
                serde_json::Value::Null
            };
            let run_id = match &result {
                Ok(()) => {
                    outcome
                        .trace
                        .finish(&deps.pool, kb_run::STATUS_OK, None)
                        .await
                }
                Err(e) => {
                    outcome
                        .trace
                        .finish(&deps.pool, kb_run::STATUS_FAILED, Some(&e.to_string()))
                        .await
                }
            };
            let log_id = log_ask(&deps, &ask, &outcome, user_id, latency, run_id).await;
            let payload = match result {
                Ok(()) => serde_json::json!({
                    "status": outcome.status,
                    "references": outcome.references.iter().cloned().map(ReferenceDto::from).collect::<Vec<_>>(),
                    "log_id": log_id.map(i64::from),
                    "run_id": run_id.map(i64::from),
                    "trace": trace_payload,
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
        let result = crate::kb::pipeline::finish_answer(&deps, &mut outcome).await;
        let latency = started.elapsed().as_millis() as i64;
        let trace_payload = if want_trace {
            outcome.trace.stages_snapshot()
        } else {
            serde_json::Value::Null
        };
        let run_id = match &result {
            Ok(()) => {
                outcome
                    .trace
                    .finish(&deps.pool, kb_run::STATUS_OK, None)
                    .await
            }
            Err(e) => {
                outcome
                    .trace
                    .finish(&deps.pool, kb_run::STATUS_FAILED, Some(&e.to_string()))
                    .await
            }
        };
        let log_id = log_ask(&deps, &ask, &outcome, user_id, latency, run_id).await;
        result?;
        let mut body = serde_json::json!({
            "status": outcome.status,
            "answer": outcome.answer,
            "references": outcome.references.iter().cloned().map(ReferenceDto::from).collect::<Vec<_>>(),
            "log_id": log_id.map(i64::from),
        });
        if want_trace {
            body["run_id"] = serde_json::json!(run_id.map(i64::from));
            body["trace"] = trace_payload;
        }
        Ok(ApiResponse::success(body).into_response())
    }
}

/// Persist the query log row (§9 feedback data plane + observability §3.4:
/// rewritten question, full kb_ids, latency, run backlink — source='ask').
#[allow(clippy::too_many_arguments)]
async fn log_ask(
    deps: &crate::kb::service::KbDeps,
    ask: &crate::kb::pipeline::AskRequest,
    outcome: &crate::kb::pipeline::AskOutcome,
    user_id: Option<crate::types::snowflake_id::SnowflakeId>,
    latency_ms: i64,
    run_id: Option<SnowflakeId>,
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
    let kb_ids_json = serde_json::json!(ask.kb_ids);
    crate::kb::models::query_log::insert_entry(
        &deps.pool,
        &crate::kb::models::query_log::LogEntry {
            kb_id,
            question: &ask.question,
            answer: Some(&outcome.answer),
            cited_units: Some(&cited),
            status: outcome.status,
            top_score: Some(f64::from(outcome.top_score)),
            user_id,
            rewritten_question: Some(&outcome.question),
            kb_ids: Some(&kb_ids_json),
            latency_ms: Some(latency_ms),
            run_id,
            error: None,
            source: "ask",
        },
    )
    .await
    .ok()
}

#[derive(Deserialize)]
struct SearchRequestDto {
    query: String,
    #[serde(default)]
    kb_ids: Option<Vec<SnowflakeId>>,
    #[serde(default)]
    top_k: Option<u32>,
}

/// Unit-level search without generation — the retrieval seam agents consume
/// (§10; [抄WK:agent/tools/knowledge_search.go 模式]). Goes through
/// `search_units` (no S1 rewrite) with its own traced run + query log
/// (source='search'). Q7 fix: the response `status` is the REAL coverage
/// verdict (empty recall or below `fallback_threshold` → `uncovered`),
/// not the hardcoded `"answered"` `prepare_answer` used to leak.
async fn public_search(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SearchRequestDto>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    auth.ensure_authenticated()?;
    let deps = state.kb_deps()?;
    let tenant = tenant_of(&auth);
    let kb_ids: Vec<i64> = req.kb_ids.unwrap_or_default().iter().map(|i| i.0).collect();
    let started = std::time::Instant::now();

    let mut trace = crate::kb::trace::RunRecorder::create(
        crate::kb::trace::TraceMode::parse(&deps.config.kb.trace_mode),
        crate::kb::trace::RunSpec {
            kind: kb_run::KIND_SEARCH,
            trigger_src: "public",
            tenant_id: tenant.clone(),
            kb_id: kb_ids.first().copied().map(SnowflakeId),
            doc_id: None,
            agent_id: None,
            session_id: None,
            job_id: None,
            attempt: 1,
            long_running: false,
        },
    );
    trace.begin(&deps.pool, &deps.config).await;

    let result =
        crate::kb::pipeline::search_units(&deps, &tenant, &kb_ids, &req.query, &mut trace).await;
    let latency = started.elapsed().as_millis() as i64;
    match result {
        Ok((top_score, mut units)) => {
            if let Some(k) = req.top_k {
                units.truncate(k as usize);
            }
            let status = crate::kb::pipeline::coverage_status(
                units.len(),
                top_score,
                deps.config.kb.fallback_threshold,
            );
            let run_id = trace.finish(&deps.pool, kb_run::STATUS_OK, None).await;
            let cited = serde_json::json!(units.iter().map(|u| u.unit_id).collect::<Vec<i64>>());
            let kb_ids_json = serde_json::json!(kb_ids);
            let _ = crate::kb::models::query_log::insert_entry(
                &deps.pool,
                &crate::kb::models::query_log::LogEntry {
                    kb_id: kb_ids.first().copied().map(SnowflakeId),
                    question: &req.query,
                    answer: None,
                    cited_units: Some(&cited),
                    status,
                    top_score: Some(f64::from(top_score)),
                    user_id: auth.user_id().map(SnowflakeId),
                    rewritten_question: None,
                    kb_ids: Some(&kb_ids_json),
                    latency_ms: Some(latency),
                    run_id,
                    error: None,
                    source: "search",
                },
            )
            .await;
            let items: Vec<serde_json::Value> = units
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
                "status": status,
                "top_score": top_score,
            })))
        }
        Err(e) => {
            trace
                .finish(&deps.pool, kb_run::STATUS_FAILED, Some(&e.to_string()))
                .await;
            let kb_ids_json = serde_json::json!(kb_ids);
            let _ = crate::kb::models::query_log::insert_entry(
                &deps.pool,
                &crate::kb::models::query_log::LogEntry {
                    kb_id: kb_ids.first().copied().map(SnowflakeId),
                    question: &req.query,
                    answer: None,
                    cited_units: None,
                    status: "error",
                    top_score: None,
                    user_id: auth.user_id().map(SnowflakeId),
                    rewritten_question: None,
                    kb_ids: Some(&kb_ids_json),
                    latency_ms: Some(latency),
                    run_id: None,
                    error: Some(&e.to_string()),
                    source: "search",
                },
            )
            .await;
            Err(e)
        }
    }
}

// ── wiki governance (M4) ───────────────────────────────────────────

#[derive(Deserialize)]
struct DistillRequest {
    kb_id: SnowflakeId,
    doc_ids: Vec<SnowflakeId>,
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
        doc_ids: req.doc_ids.iter().map(|i| i.0).collect(),
        tenant_id: tenant_of(&auth),
    });
    new_job.priority = -5; // slow LLM work must not starve online jobs (§7)
    let queue = crate::worker::DefaultJobQueue::new(state.pool.clone());
    queue.enqueue(new_job).await?;
    Ok(ApiResponse::success(json!({ "queued": true })))
}

#[derive(Deserialize)]
struct ListWikiPagesQuery {
    kb_id: Option<SnowflakeId>,
    status: Option<String>,
    page: Option<i64>,
    page_size: Option<i64>,
}

async fn admin_list_wiki_pages(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListWikiPagesQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(20);
    let (pages, total) = knowledge_base_list(
        &state.pool,
        q.kb_id,
        q.status.as_deref(),
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
    kb_id: Option<SnowflakeId>,
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
                "id": p.id, "kb_id": p.kb_id, "title": p.title, "slug": p.slug,
                "status": p.status, "summary": p.summary,
                "current_revision": p.current_revision,
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

/// Chunk detail (full content) — reference "view full" popover input.
async fn admin_get_chunk(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let id = parse_snowflake(&id)?;
    let Some(c) = crate::kb::models::chunk::find_chunk_by_id(&state.pool, id).await? else {
        return Err(AppError::NotFound("kb_chunk".into()));
    };
    let kb_names = knowledge_base::find_kb_names_by_ids(&state.pool, &[i64::from(c.kb_id)]).await?;
    let doc_ids: Vec<i64> = c.doc_id.map(i64::from).into_iter().collect();
    let doc_titles = document::find_doc_titles_by_ids(&state.pool, &doc_ids).await?;
    Ok(ApiResponse::success(json!({
        "id": c.id, "kb_id": c.kb_id,
        "kb_name": kb_names.get(&i64::from(c.kb_id)),
        "doc_id": c.doc_id,
        "doc_title": c.doc_id.and_then(|d| doc_titles.get(&i64::from(d))),
        "kind": c.kind, "parent_id": c.parent_id, "seq": c.seq,
        "breadcrumb": c.breadcrumb, "content": c.content,
        "has_embedding": c.embedding.is_some(),
    })))
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
    let Some(chunk) = crate::kb::models::chunk::find_chunk_by_id(&state.pool, id).await? else {
        return Err(AppError::NotFound("kb_chunk".into()));
    };
    // Traced five-step body lives in the service layer (thin handler).
    service::edit_chunk_traced(&deps, &chunk, auth.user_id().map(SnowflakeId), &req.content)
        .await?;
    Ok(ApiResponse::success(json!({ "id": id, "reindexed": true })))
}

/// Delete one chunk (cascading to its children): SQL rows, vector points
/// and FTS units are all removed; the source facet is rebuilt from the
/// remaining chunks. Mirrors the edit handler's re-index shape.
async fn admin_delete_chunk(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    let Some(chunk) = crate::kb::models::chunk::find_chunk_by_id(&state.pool, id).await? else {
        return Err(AppError::NotFound("kb_chunk".into()));
    };
    // Children of a parent chunk are part of its content — cascade.
    let children = crate::kb::models::chunk::find_children_by_parent(&state.pool, id).await?;
    let mut gone_ids: Vec<i64> = children.iter().map(|c| i64::from(c.id)).collect();
    gone_ids.push(i64::from(id));
    deps.vector
        .delete(i64::from(chunk.kb_id), &gone_ids)
        .await?;
    crate::kb::models::chunk::delete_chunks_by_ids(&state.pool, &gone_ids).await?;
    // FTS: rebuild the remaining units of the source facet per kind
    // (document → doc_id facet, faq → faq_id facet, wiki → self-keyed).
    if let Some(doc_id) = chunk.doc_id {
        let remaining = crate::kb::models::chunk::find_chunks_by_doc(&state.pool, doc_id).await?;
        rebuild_fts_facet(
            &deps,
            i64::from(doc_id),
            &remaining
                .iter()
                .map(|c| {
                    (i64::from(c.id), i64::from(c.kb_id), c.kind.clone(), {
                        match &c.breadcrumb {
                            Some(b) => format!("{b}\n{}", c.content),
                            None => c.content.clone(),
                        }
                    })
                })
                .collect::<Vec<_>>(),
        )
        .await?;
        document::set_chunk_count(&state.pool, doc_id, remaining.len() as i64, &tenant).await?;
        // Incremental invalidation (§7): pages citing this doc go stale.
        let stale = crate::kb::models::wiki_page::mark_stale_by_doc(&state.pool, doc_id).await?;
        if !stale.is_empty() {
            tracing::info!(
                "[kb] doc {doc_id} chunks edited: {} wiki page(s) marked stale",
                stale.len()
            );
        }
    } else if let Some(faq_id) = chunk.faq_id {
        let remaining = crate::kb::models::chunk::find_chunks_by_faq(&state.pool, faq_id).await?;
        rebuild_fts_facet(
            &deps,
            i64::from(faq_id),
            &remaining
                .iter()
                .map(|c| {
                    (
                        i64::from(c.id),
                        i64::from(c.kb_id),
                        c.kind.clone(),
                        c.content.clone(),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .await?;
    } else {
        // Wiki (or lone) units key on themselves — drop per unit.
        for gid in &gone_ids {
            deps.kbsearch.delete_document(*gid).await?;
        }
    }
    Ok(ApiResponse::success(
        json!({ "deleted": true, "removed": gone_ids.len() }),
    ))
}

/// Rebuild one FTS facet from its remaining chunks; an empty facet is
/// removed outright.
async fn rebuild_fts_facet(
    deps: &KbDeps,
    facet_id: i64,
    remaining: &[(i64, i64, String, String)],
) -> AppResult<()> {
    if remaining.is_empty() {
        deps.kbsearch.delete_document(facet_id).await?;
        return Ok(());
    }
    let units: Vec<crate::kb::kbsearch::KbIndexUnit> = remaining
        .iter()
        .map(
            |(unit_id, kb_id, kind, text)| crate::kb::kbsearch::KbIndexUnit {
                unit_id: *unit_id,
                kb_id: *kb_id,
                doc_id: facet_id,
                kind: kind.clone(),
                text: text.clone(),
            },
        )
        .collect();
    deps.kbsearch.reindex_document(&units).await?;
    Ok(())
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
    let tenant = auth.tenant_id().unwrap_or("default");
    let messages = vec![
        raisfast_agent::ChatMessage {
            role: raisfast_agent::ChatRole::System,
            content: Some(prompt_file!("src/kb/prompts/faq_draft.md")),
            images: Vec::new(),
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
            images: Vec::new(),
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
    let reply = deps
        .router
        .call(tenant, crate::llm::models::log::LogSource::Kb)
        .chat(None, &request)
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
        .map(|(question, count, log_id)| {
            json!({ "question": question, "count": count, "log_id": log_id })
        })
        .collect();
    Ok(ApiResponse::success(json!({ "items": items })))
}

/// Restore a page to a revision snapshot: current content is snapshotted
/// first (undo is always possible), then the revision content is applied;
/// published pages re-index immediately [抄WK:wiki rollback 语义].
async fn admin_restore_wiki_page(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((id, rev)): Path<(String, String)>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let rev_id = parse_snowflake(&rev)?;
    let tenant = tenant_of(&auth);
    let Some(page) =
        crate::kb::models::wiki_page::find_page_by_id(&state.pool, id, &tenant).await?
    else {
        return Err(AppError::NotFound("kb_wiki_page".into()));
    };
    let revision =
        crate::services::content_revision::get_revision(&state.pool, "kb_wiki_page", id, rev_id)
            .await
            .map_err(|_| AppError::NotFound("content_revision".into()))?;
    let content = revision
        .snapshot
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("revision snapshot missing content")))?;
    // 快照当前内容（restore 本身可撤销）
    let snapshot = json!({ "title": page.title, "slug": page.slug, "content": page.content,
        "summary": page.summary, "status": page.status });
    insert_wiki_snapshot(&state.pool, id, page.current_revision, &snapshot, &tenant).await?;
    let was_published = page.status == "published";
    crate::kb::models::wiki_page::update_page_content(
        &state.pool,
        id,
        content,
        page.summary.as_deref(),
        if was_published { "published" } else { "draft" },
        &tenant,
    )
    .await?;
    if was_published {
        crate::kb::distill::publish_page(
            &deps,
            id,
            SnowflakeId(auth.user_id().unwrap_or_default()),
            &tenant,
        )
        .await?;
    }
    Ok(ApiResponse::success(
        json!({ "id": id, "restored_from": rev_id, "reindexed": was_published }),
    ))
}

#[derive(Deserialize)]
struct ListFaqsQuery {
    kb_id: SnowflakeId,
    page: Option<i64>,
    page_size: Option<i64>,
}

async fn admin_list_faqs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListFaqsQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(20);
    let (faqs, total) =
        crate::kb::models::faq::list_faqs(&state.pool, q.kb_id, page, page_size, &tenant_of(&auth))
            .await?;
    Ok(ApiResponse::success(
        json!({ "items": faqs, "total": total, "page": page, "page_size": page_size }),
    ))
}

#[derive(Deserialize)]
struct UpdateFaqRequest {
    standard_question: String,
    #[serde(default)]
    similar_questions: Vec<String>,
    answers: Vec<String>,
}

async fn admin_update_faq(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateFaqRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let id = parse_snowflake(&id)?;
    let tenant = tenant_of(&auth);
    crate::kb::models::faq::update_faq(
        &state.pool,
        id,
        &crate::kb::models::faq::UpdateFaqCmd {
            standard_question: req.standard_question,
            similar_questions: req.similar_questions,
            answers: req.answers,
        },
        &tenant,
    )
    .await?;
    // enabled FAQs re-index immediately (content changed).
    if let Some(faq) = crate::kb::models::faq::find_faqs_by_ids(&state.pool, &[i64::from(id)])
        .await?
        .into_iter()
        .next()
        && faq.enabled
    {
        crate::kb::service::index_faq(&deps, &faq).await?;
    }
    Ok(ApiResponse::success(json!({ "id": id, "reindexed": true })))
}

/// Chunk list: document drill-down (`doc_id` set → whole document, unpaged)
/// or standalone browsing (paged across KBs, optional `kb_id` filter).
/// Browsing paginates by parent group; each page carries the children of its
/// parents so a page cut never splits a family. `total` counts groups.
/// Every item carries its KB name and document title for list columns.
#[derive(Deserialize)]
struct ListChunksQuery {
    #[serde(default)]
    doc_id: Option<SnowflakeId>,
    #[serde(default)]
    kb_id: Option<SnowflakeId>,
    #[serde(default)]
    page: Option<i64>,
    #[serde(default)]
    page_size: Option<i64>,
}

async fn admin_list_chunks(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<ListChunksQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let (chunks, total) = if let Some(doc_id) = q.doc_id {
        let all = crate::kb::models::chunk::find_chunks_by_doc(&state.pool, doc_id).await?;
        let n = all.len() as i64;
        (all, n)
    } else {
        let page = q.page.unwrap_or(1).max(1);
        let page_size = q.page_size.unwrap_or(50).clamp(1, 200);
        let (parents, total) =
            crate::kb::models::chunk::list_chunks_paged(&state.pool, q.kb_id, page, page_size)
                .await?;
        // Reunite each page's parents with their children, then restore the
        // flat display order (kb → doc → seq keeps every family contiguous).
        let parent_ids: Vec<i64> = parents.iter().map(|p| i64::from(p.id)).collect();
        let children =
            crate::kb::models::chunk::find_children_by_parents(&state.pool, &parent_ids).await?;
        let mut rows: Vec<_> = parents.into_iter().chain(children).collect();
        rows.sort_by_key(|c| {
            (
                i64::from(c.kb_id),
                c.doc_id.map(i64::from).unwrap_or_default(),
                c.seq,
            )
        });
        (rows, total)
    };
    // Resolve kb_name / doc_title columns in two batch lookups.
    let kb_ids: Vec<i64> = {
        let mut ids: Vec<i64> = chunks.iter().map(|c| i64::from(c.kb_id)).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let doc_ids: Vec<i64> = {
        let mut ids: Vec<i64> = chunks
            .iter()
            .filter_map(|c| c.doc_id.map(i64::from))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let kb_names = knowledge_base::find_kb_names_by_ids(&state.pool, &kb_ids).await?;
    let doc_titles = document::find_doc_titles_by_ids(&state.pool, &doc_ids).await?;
    let items: Vec<Value> = chunks
        .iter()
        .map(|c| {
            json!({
                "id": c.id, "kind": c.kind, "parent_id": c.parent_id, "seq": c.seq,
                "page": c.page,
                "breadcrumb": c.breadcrumb, "bytes": c.content.len(),
                "has_embedding": c.embedding.is_some(),
                "preview": c.content.chars().take(100).collect::<String>(),
                "content": c.content,
                "kb_id": c.kb_id,
                "kb_name": kb_names.get(&i64::from(c.kb_id)),
                "doc_id": c.doc_id,
                "doc_title": c.doc_id.and_then(|d| doc_titles.get(&i64::from(d))),
            })
        })
        .collect();
    Ok(ApiResponse::success(
        json!({ "items": items, "total": total }),
    ))
}

/// Ops card: per-KB counters + vector backend identity.
async fn admin_kb_stats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let tenant = tenant_of(&auth);
    let (kbs, _) =
        crate::kb::models::knowledge_base::list_kbs(&state.pool, 1, 100, &tenant).await?;
    let mut items = Vec::new();
    for kb in &kbs {
        let docs: i64 = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM kb_documents WHERE kb_id = {}",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        )))
        .bind(i64::from(kb.id))
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        let chunks: i64 = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM kb_chunks WHERE kb_id = {}",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        )))
        .bind(i64::from(kb.id))
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        let faqs: i64 = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM kb_faqs WHERE kb_id = {}",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        )))
        .bind(i64::from(kb.id))
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        let wiki: i64 = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM kb_wiki_pages WHERE kb_id = {}",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        )))
        .bind(i64::from(kb.id))
        .fetch_one(&state.pool)
        .await
        .unwrap_or(0);
        items.push(json!({
            "kb_id": kb.id, "name": kb.name, "kind": kb.kind,
            "documents": docs, "chunks": chunks, "faqs": faqs, "wiki_pages": wiki,
        }));
    }
    let vector_backend = state
        .kb_runtime
        .as_ref()
        .map(|rt| rt.vector.backend_name().to_string());
    Ok(ApiResponse::success(json!({
        "enabled": state.kb_runtime.is_some(),
        "vector_backend": vector_backend,
        "default_embedding_model": serde_json::Value::Null,
        "default_embedding_dim": serde_json::Value::Null,
        "items": items,
    })))
}
