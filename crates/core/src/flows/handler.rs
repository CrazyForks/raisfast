//! Flow engine HTTP handlers (P1.10 minimal admin surface).
//!
//! Endpoints:
//! - POST /admin/flows                    create flow + first version (definition)
//! - GET  /admin/flows                    list flows (default tenant)
//! - GET  /admin/flows/{id}               flow detail (incl. latest version definition)
//! - PUT  /admin/flows/{id}               update metadata + publish new version
//! - DELETE /admin/flows/{id}             cascade-delete flow + versions/instances
//! - POST /admin/flows/{id}/run           create + execute an instance (inputs)
//! - GET  /admin/flows/instances          paginated instance list (filters)
//! - GET  /admin/flows/instances/{id}     instance status/outputs
//! - GET  /admin/flows/instances/{id}/node-runs  per-node run history
//! - POST /admin/flows/instances/{id}/stop  mark canceled
//!
//! cron/event triggers + draft/publish (explicit) land in P2.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::payment::crypto::{aes256gcm_decrypt, aes256gcm_encrypt};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

use super::exec::FlowsExec;
use super::graph;
use super::model::{self, Flow, FlowVersion};

fn now() -> Timestamp {
    crate::utils::tz::now_utc()
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize)]
pub struct CreateFlowReq {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Value,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize)]
pub struct RunFlowReq {
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub inputs: Option<Value>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize)]
pub struct UpdateFlowReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Immediate publish of this definition (legacy path).
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Option<Value>,
    /// Save the working draft (no new version).
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub draft: Option<Value>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize, Default)]
pub struct PublishFlowReq {
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Option<Value>,
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Serialize)]
pub struct FlowDetail {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    /// Working draft if one is saved (falls back to the latest published).
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version_number: Option<i64>,
    pub has_draft: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows",
        post,
        create_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows",
        get,
        list_flows,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/page",
        get,
        list_flows_page,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}",
        get,
        get_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}",
        put,
        update_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}",
        delete,
        delete_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/run",
        post,
        run_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/publish",
        post,
        publish_flow,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/apis/logs",
        get,
        list_public_api_logs,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/apis",
        get,
        list_flow_apis,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api",
        get,
        flow_api_status,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api/enable",
        post,
        enable_flow_api,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api/rotate",
        post,
        rotate_flow_api,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api/slug",
        post,
        rotate_slug_api,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api/disable",
        post,
        disable_flow_api,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/api/delete",
        post,
        delete_flow_api,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/flows/{id}/run",
        post,
        run_public_api,
        "flows",
        "public-flows",
        "public"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/versions",
        get,
        list_versions,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/{id}/versions/{version}/rollback",
        post,
        rollback_version,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/instances",
        get,
        list_instances,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/instances/{id}",
        get,
        get_instance,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/instances/{id}/node-runs",
        get,
        list_node_runs,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/instances/{id}/stop",
        post,
        stop_instance,
        "flows",
        "admin/flows"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/flows/instances/{id}/resume",
        post,
        resume_instance,
        "flows",
        "admin/flows"
    );
    let _ = restful;
    r
}

#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize)]
pub struct ResumeReq {
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub payload: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListInstancesQuery {
    #[serde(default = "default_page")]
    pub page: i64,
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

fn default_page() -> i64 {
    1
}

fn default_page_size() -> i64 {
    20
}

async fn resume_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<ResumeReq>,
) -> AppResult<ApiResponse<Value>> {
    let instance_id = parse_id(id)?;
    super::run::resume_instance(&state.pool, instance_id, req.payload).await?;
    let done = model::find_instance_by_id(&state.pool, instance_id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(&done).unwrap_or_default(),
    ))
}

/// Create a flow with its first published version (definition validated).
async fn create_flow(
    State(state): State<AppState>,
    Json(req): Json<CreateFlowReq>,
) -> AppResult<ApiResponse<Value>> {
    // Validate graph structure + node configs up front.
    graph::load_definition(&req.definition)?;

    let flow_id = crate::utils::id::new_snowflake_id();
    let now = now();
    let flow = Flow {
        id: flow_id,
        tenant_id: crate::constants::DEFAULT_TENANT.to_string(),
        name: req.name,
        description: req.description,
        enabled: true,
        current_version: None,
        extra: None,
        created_at: now,
        updated_at: now,
    };
    model::insert_flow(&state.pool, &flow).await?;

    let version_id = crate::utils::id::new_snowflake_id();
    let version = FlowVersion {
        id: version_id,
        flow_id,
        version_number: 1,
        definition: req.definition,
        created_by: None,
        created_at: now,
    };
    model::insert_flow_version(&state.pool, &version).await?;
    model::set_flow_current_version(&state.pool, flow_id, version_id).await?;

    Ok(ApiResponse::success(json!({
        "flow_id": flow_id,
        "flow_version_id": version_id
    })))
}

/// Enrich one flow row with `version_number` / `has_draft` / `has_api`.
async fn flow_list_row(pool: &crate::db::Pool, flow: &model::Flow) -> AppResult<Value> {
    let mut v = serde_json::to_value(flow)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize flow: {e}")))?;
    let version_number = model::current_version_number(pool, flow.id).await?;
    let has_draft = model::flow_draft(flow).is_some();
    let has_api = model::find_api_key_by_flow(pool, flow.id).await?.is_some();
    if let Some(obj) = v.as_object_mut() {
        obj.insert(
            "version_number".to_string(),
            version_number.map(Into::into).unwrap_or(Value::Null),
        );
        obj.insert("has_draft".to_string(), json!(has_draft));
        obj.insert("has_api".to_string(), json!(has_api));
    }
    Ok(v)
}

async fn list_flows(State(state): State<AppState>) -> AppResult<ApiResponse<Value>> {
    let flows = model::find_flows_by_tenant(&state.pool, crate::constants::DEFAULT_TENANT).await?;
    let mut items = Vec::with_capacity(flows.len());
    for flow in flows {
        items.push(flow_list_row(&state.pool, &flow).await?);
    }
    Ok(ApiResponse::success(Value::Array(items)))
}

/// Paged flows list (server-side pagination) for the admin table.
async fn list_flows_page(
    State(state): State<AppState>,
    Query(q): Query<ListInstancesQuery>,
) -> AppResult<ApiResponse<Value>> {
    let (rows, total) = model::find_flows_page(
        &state.pool,
        crate::constants::DEFAULT_TENANT,
        q.page.max(1),
        q.page_size.clamp(1, 100),
    )
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for flow in rows {
        items.push(flow_list_row(&state.pool, &flow).await?);
    }
    Ok(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": q.page.max(1),
        "page_size": q.page_size.clamp(1, 100),
    })))
}

/// Flow detail value: draft (if any) shadows the latest published definition.
async fn build_detail(pool: &crate::db::Pool, flow_id: SnowflakeId) -> AppResult<Value> {
    let flow = model::find_flow_by_id(pool, flow_id).await?;
    let latest = model::latest_version(pool, flow_id).await?;
    let draft = model::flow_draft(&flow);
    let has_draft = draft.is_some();
    let definition = draft.or_else(|| latest.as_ref().map(|v| v.definition.clone()));
    let detail = FlowDetail {
        id: flow.id,
        tenant_id: flow.tenant_id,
        name: flow.name,
        description: flow.description,
        enabled: flow.enabled,
        definition,
        version_number: latest.map(|v| v.version_number),
        has_draft,
        created_at: flow.created_at,
        updated_at: flow.updated_at,
    };
    serde_json::to_value(&detail)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize flow detail: {e}")))
}

/// Flow detail incl. the latest published version's definition (for the editor).
async fn get_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(
        build_detail(&state.pool, flow_id).await?,
    ))
}

/// Update metadata and/or save a draft; `definition` (legacy) publishes now.
async fn update_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateFlowReq>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    let flow = model::find_flow_by_id(&state.pool, flow_id).await?;

    let name = req.name.unwrap_or(flow.name);
    let description = req.description.or(flow.description);
    model::update_flow_meta(&state.pool, flow_id, &name, description.as_deref()).await?;

    if let Some(definition) = req.definition {
        graph::load_definition(&definition)?;
        let next = model::latest_version(&state.pool, flow_id)
            .await?
            .map(|v| v.version_number + 1)
            .unwrap_or(1);
        let version = FlowVersion {
            id: crate::utils::id::new_snowflake_id(),
            flow_id,
            version_number: next,
            definition,
            created_by: None,
            created_at: now(),
        };
        model::insert_flow_version(&state.pool, &version).await?;
        model::set_flow_current_version(&state.pool, flow_id, version.id).await?;
        model::set_flow_draft(&state.pool, flow_id, None).await?;
    } else if let Some(draft) = req.draft {
        graph::load_definition(&draft)?;
        model::set_flow_draft(&state.pool, flow_id, Some(draft)).await?;
    }

    Ok(ApiResponse::success(
        build_detail(&state.pool, flow_id).await?,
    ))
}

/// Publish the current working draft as a new version (explicit publish).
async fn publish_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PublishFlowReq>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    let flow = model::find_flow_by_id(&state.pool, flow_id).await?;
    let draft = req
        .definition
        .or_else(|| model::flow_draft(&flow))
        .ok_or_else(|| AppError::BadRequest("没有可发布的草稿".into()))?;
    graph::load_definition(&draft)?;
    let next = model::latest_version(&state.pool, flow_id)
        .await?
        .map(|v| v.version_number + 1)
        .unwrap_or(1);
    let version = FlowVersion {
        id: crate::utils::id::new_snowflake_id(),
        flow_id,
        version_number: next,
        definition: draft,
        created_by: None,
        created_at: now(),
    };
    model::insert_flow_version(&state.pool, &version).await?;
    model::set_flow_current_version(&state.pool, flow_id, version.id).await?;
    model::set_flow_draft(&state.pool, flow_id, None).await?;
    Ok(ApiResponse::success(
        build_detail(&state.pool, flow_id).await?,
    ))
}

/// Cascade-delete a flow and all its versions/instances.
async fn delete_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::delete_flow(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(json!({"deleted": true})))
}

/// All published versions of a flow, oldest first.
async fn list_versions(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    let versions = model::list_versions(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(versions)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize versions: {e}")))?,
    ))
}

/// Roll back to an older version: the selected snapshot is re-published as a
/// new `version_number` (immutable append model) and becomes the current one.
async fn rollback_version(
    State(state): State<AppState>,
    Path((flow_id_raw, version_raw)): Path<(String, String)>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(flow_id_raw)?;
    let version_id = parse_id(version_raw)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    let old = model::find_version_by_id(&state.pool, version_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    if old.flow_id != flow_id {
        return Err(AppError::not_found("flow_version"));
    }
    let next = model::latest_version(&state.pool, flow_id)
        .await?
        .map(|v| v.version_number + 1)
        .unwrap_or(1);
    let new_version = FlowVersion {
        id: crate::utils::id::new_snowflake_id(),
        flow_id,
        version_number: next,
        definition: old.definition,
        created_by: None,
        created_at: now(),
    };
    model::insert_flow_version(&state.pool, &new_version).await?;
    model::set_flow_current_version(&state.pool, flow_id, new_version.id).await?;
    // Rolling back replaces the working draft with the restored published state.
    model::set_flow_draft(&state.pool, flow_id, None).await?;
    Ok(ApiResponse::success(
        build_detail(&state.pool, flow_id).await?,
    ))
}

fn api_endpoint(slug: &str) -> String {
    format!("/flows/{slug}/run")
}

fn token_hash(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    hex::encode(h.finalize())
}

fn enc_key(cfg: &AppConfig) -> AppResult<[u8; 32]> {
    let raw = cfg
        .app_key
        .as_deref()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("api key: APP_KEY 未配置")))?;
    let bytes = BASE64
        .decode(raw)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("api key: bad app_key b64: {e}")))?;
    bytes
        .try_into()
        .map_err(|_| AppError::Internal(anyhow::anyhow!("api key: app_key must be 32 bytes")))
}

fn encrypt_token(cfg: &AppConfig, token: &str) -> AppResult<String> {
    aes256gcm_encrypt(token, &enc_key(cfg)?)
}

fn decrypt_token(cfg: &AppConfig, enc: &str) -> AppResult<String> {
    aes256gcm_decrypt(enc, &enc_key(cfg)?)
}

/// Current public-API state for a flow (token + endpoint + enabled).
async fn flow_api_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    let row = model::find_api_key_by_flow(&state.pool, flow_id).await?;
    let token = match &row {
        Some(r) => Some(decrypt_token(&state.config, &r.token_enc)?),
        None => None,
    };
    let enabled = row.as_ref().map(|r| r.enabled).unwrap_or(false);
    let require_auth = row.as_ref().map(|r| r.require_auth).unwrap_or(true);
    let slug = row.as_ref().map(|r| r.slug.clone()).unwrap_or_default();
    Ok(ApiResponse::success(json!({
        "enabled": enabled,
        "disabled": row.is_some() && !enabled,
        "require_auth": require_auth,
        "slug": slug,
        "token": token,
        "endpoint": api_endpoint(&slug),
    })))
}

/// Paginated log of EXTERNAL API calls only (trigger = `api_public`).
async fn list_public_api_logs(
    State(state): State<AppState>,
    Query(q): Query<ListInstancesQuery>,
) -> AppResult<ApiResponse<Value>> {
    let flows = model::find_flows_by_tenant(&state.pool, crate::constants::DEFAULT_TENANT).await?;
    let name_of: std::collections::HashMap<String, String> = flows
        .into_iter()
        .map(|f| (f.id.to_string(), f.name))
        .collect();
    let flow_filter = match q.flow_id {
        Some(raw) if !raw.is_empty() => Some(parse_id(raw)?),
        _ => None,
    };
    let (rows, total) = model::find_instances_by_trigger_page(
        &state.pool,
        "api_public",
        flow_filter,
        q.page.max(1),
        q.page_size.clamp(1, 100),
    )
    .await?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|i| {
            json!({
                "instance_id": i.id,
                "flow_id": i.flow_id,
                "flow_name": name_of.get(&i.flow_id.to_string()).cloned().unwrap_or_default(),
                "status": i.status,
                "inputs": i.trigger_payload,
                "error": i.error,
                "outputs": i.outputs,
                "started_at": i.started_at,
                "finished_at": i.finished_at,
                "created_at": i.created_at,
            })
        })
        .collect();
    Ok(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": q.page.max(1),
        "page_size": q.page_size.clamp(1, 100),
    })))
}

/// All flows that hold an API key (enabled or paused) — management list.
async fn list_flow_apis(
    State(state): State<AppState>,
    Query(q): Query<ListInstancesQuery>,
) -> AppResult<ApiResponse<Value>> {
    let flows = model::find_flows_by_tenant(&state.pool, crate::constants::DEFAULT_TENANT).await?;
    let mut all: Vec<Value> = Vec::new();
    for f in flows {
        let Some(row) = model::find_api_key_by_flow(&state.pool, f.id).await? else {
            continue;
        };
        let token = decrypt_token(&state.config, &row.token_enc)?;
        all.push(json!({
            "flow_id": f.id,
            "name": f.name,
            "token": token,
            "slug": row.slug,
            "endpoint": api_endpoint(&row.slug),
            "enabled": row.enabled,
            "require_auth": row.require_auth,
            "updated_at": f.updated_at,
        }));
    }
    let total = all.len() as i64;
    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 100);
    let offset = ((page - 1) * page_size) as usize;
    let items: Vec<Value> = all
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .collect();
    Ok(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size,
    })))
}

/// Enable / re-enable a flow's public API. `require_auth=false` allows calls
/// without a bearer token (internal networks). Keys are AES-GCM encrypted.
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Deserialize, Default)]
pub struct EnableApiReq {
    #[serde(default)]
    pub require_auth: Option<bool>,
}

async fn enable_flow_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<EnableApiReq>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    model::latest_version(&state.pool, flow_id)
        .await?
        .ok_or_else(|| AppError::BadRequest("先发布一个版本再开放 API".into()))?;

    let require_auth = req.require_auth.unwrap_or(true);
    let token = match model::find_api_key_by_flow(&state.pool, flow_id).await? {
        Some(row) => {
            model::set_api_key_enabled(&state.pool, flow_id, true).await?;
            model::set_api_key_require_auth(&state.pool, flow_id, require_auth).await?;
            decrypt_token(&state.config, &row.token_enc)?
        }
        None => {
            let t = crate::utils::id::random_hex(24);
            let slug = crate::utils::id::random_hex(10);
            let enc = encrypt_token(&state.config, &t)?;
            model::create_api_key(
                &state.pool,
                flow_id,
                &token_hash(&t),
                &enc,
                &slug,
                require_auth,
            )
            .await?;
            t
        }
    };
    let slug = model::find_api_key_by_flow(&state.pool, flow_id)
        .await?
        .map(|r| r.slug)
        .unwrap_or_default();
    Ok(ApiResponse::success(json!({
        "enabled": true,
        "require_auth": require_auth,
        "token": token,
        "slug": slug,
        "endpoint": api_endpoint(&slug),
    })))
}

/// Rotate the token of an existing public API (keeps slug / enabled / auth).
async fn rotate_flow_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    if model::find_api_key_by_flow(&state.pool, flow_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found("flow_api"));
    }
    let t = crate::utils::id::random_hex(24);
    let enc = encrypt_token(&state.config, &t)?;
    model::update_api_key_token(&state.pool, flow_id, &token_hash(&t), &enc).await?;
    let fresh = model::find_api_key_by_flow(&state.pool, flow_id).await?;
    let require_auth = fresh.as_ref().map(|r| r.require_auth).unwrap_or(true);
    let slug = fresh.map(|r| r.slug).unwrap_or_default();
    Ok(ApiResponse::success(json!({
        "enabled": true,
        "require_auth": require_auth,
        "token": t,
        "slug": slug,
        "endpoint": api_endpoint(&slug),
    })))
}

/// Rotate the public path slug (keeps token / enabled / auth).
async fn rotate_slug_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    if model::find_api_key_by_flow(&state.pool, flow_id)
        .await?
        .is_none()
    {
        return Err(AppError::not_found("flow_api"));
    }
    let slug = crate::utils::id::random_hex(10);
    model::update_api_key_slug(&state.pool, flow_id, &slug).await?;
    let fresh = model::find_api_key_by_flow(&state.pool, flow_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_api"))?;
    let token = decrypt_token(&state.config, &fresh.token_enc)?;
    Ok(ApiResponse::success(json!({
        "enabled": fresh.enabled,
        "require_auth": fresh.require_auth,
        "token": token,
        "slug": fresh.slug,
        "endpoint": api_endpoint(&fresh.slug),
    })))
}

/// Pause a flow's public API — keeps the key so it can be resumed later.
async fn disable_flow_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    if model::find_api_key_by_flow(&state.pool, flow_id)
        .await?
        .is_some()
    {
        model::set_api_key_enabled(&state.pool, flow_id, false).await?;
    }
    Ok(ApiResponse::success(json!({"enabled": false})))
}

/// Permanently revoke a flow's public API (key deleted, cannot resume).
async fn delete_flow_api(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    model::delete_api_key_by_flow(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(json!({"deleted": true})))
}

/// Public invocation on `/flows/{id}/run`: locate the flow by id first, then
/// authenticate. Flows with `require_auth=false` accept calls with no token
/// (internal networks); otherwise `Authorization: Bearer <token>` is required.
/// Only the latest published version is run; recorded as `trigger = api_public`.
async fn run_public_api(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<RunFlowReq>,
) -> AppResult<Json<Value>> {
    let key = model::find_api_key_by_slug(&state.pool, &slug)
        .await?
        .ok_or_else(|| AppError::not_found("flow_api"))?;
    if !key.enabled {
        return Err(AppError::not_found("flow_api"));
    }
    let flow_id = key.flow_id;
    model::find_flow_by_id(&state.pool, flow_id).await?;
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|s| !s.is_empty());
    if key.require_auth {
        let token = bearer
            .ok_or_else(|| AppError::BadRequest("缺少 Authorization: Bearer <token>".into()))?;
        let row = model::find_api_key_by_hash(&state.pool, &token_hash(token))
            .await?
            .ok_or_else(|| AppError::not_found("flow_api"))?;
        if !row.enabled || row.flow_id != flow_id {
            return Err(AppError::not_found("flow_api"));
        }
        model::touch_api_key(&state.pool, row.id).await?;
    }
    // Authless flows accept calls with no token; a valid token is still honored.
    let bearer_row = if let Some(token) = bearer {
        model::find_api_key_by_hash(&state.pool, &token_hash(token)).await?
    } else {
        None
    };
    if let Some(row) = bearer_row
        && row.enabled
        && row.flow_id == flow_id
    {
        model::touch_api_key(&state.pool, row.id).await?;
    }
    let instance = run_latest(&state, flow_id, req.inputs, "api_public").await?;
    match instance.status.as_str() {
        "success" => Ok(Json(json!({
            "status": "succeeded",
            "instance_id": instance.id,
            "outputs": instance.outputs,
        }))),
        "waiting" => Ok(Json(json!({
            "status": "waiting",
            "instance_id": instance.id,
        }))),
        _ => Ok(Json(json!({
            "status": "failed",
            "instance_id": instance.id,
            "error": instance.error,
        }))),
    }
}

/// Run the latest published version of a flow and return its finished instance.
async fn run_latest(
    state: &AppState,
    flow_id: SnowflakeId,
    inputs: Option<Value>,
    trigger: &str,
) -> AppResult<model::FlowInstance> {
    let flow = model::find_flow_by_id(&state.pool, flow_id).await?;
    let version = model::latest_version(&state.pool, flow_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    graph::load_definition(&version.definition)?;

    let instance_id = crate::utils::id::new_snowflake_id();
    let now = now();
    let instance = model::FlowInstance {
        id: instance_id,
        tenant_id: flow.tenant_id.clone(),
        flow_id,
        flow_version_id: version.id,
        status: "running".into(),
        has_exceptions: false,
        trigger_kind: trigger.to_string(),
        trigger_payload: inputs,
        inputs_summary: None,
        outputs: None,
        error: None,
        started_by: None,
        started_at: Some(now),
        finished_at: None,
        waiting_kind: None,
        waiting_needed: None,
        waiting_received: 0,
        resume_until: None,
        created_at: now,
    };
    model::insert_flow_instance(&state.pool, &instance).await?;

    let exec = FlowsExec {
        plane: state.integration.clone(),
        plugins: Some(state.plugins.clone()),
    };
    super::run::execute_instance(&state.pool, instance_id, &exec).await?;
    model::find_instance_by_id(&state.pool, instance_id).await
}

/// Create an instance from the flow's current version and execute it.
async fn run_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RunFlowReq>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = crate::types::snowflake_id::parse_id(&id)?;
    let done = run_latest(&state, flow_id, req.inputs, "api").await?;
    Ok(ApiResponse::success(
        serde_json::to_value(&done).unwrap_or_default(),
    ))
}

async fn get_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let instance_id = parse_id(id)?;
    let instance = model::find_instance_by_id(&state.pool, instance_id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(&instance).unwrap_or_default(),
    ))
}

/// Paginated instance list with optional `flow_id` / `status` filters.
async fn list_instances(
    State(state): State<AppState>,
    Query(q): Query<ListInstancesQuery>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = match q.flow_id {
        Some(raw) if !raw.is_empty() => Some(parse_id(raw)?),
        _ => None,
    };
    let (items, total) = model::find_instances_page(
        &state.pool,
        flow_id,
        q.status.as_deref(),
        q.page.max(1),
        q.page_size.clamp(1, 100),
    )
    .await?;
    Ok(ApiResponse::success(json!({
        "items": serde_json::to_value(items).map_err(|e| AppError::Internal(anyhow::anyhow!("serialize instances: {e}")))?,
        "total": total,
        "page": q.page.max(1),
        "page_size": q.page_size.clamp(1, 100),
    })))
}

/// Per-node run history for one instance (oldest first).
async fn list_node_runs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let instance_id = parse_id(id)?;
    // 404 when the instance itself doesn't exist.
    model::find_instance_by_id(&state.pool, instance_id).await?;
    let runs = model::find_node_runs(&state.pool, instance_id).await?;
    Ok(ApiResponse::success(serde_json::to_value(runs).map_err(
        |e| AppError::Internal(anyhow::anyhow!("serialize runs: {e}")),
    )?))
}

async fn stop_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let instance_id = parse_id(id)?;
    model::finalize_instance(&state.pool, instance_id, "canceled", false, None, None).await?;
    Ok(ApiResponse::success(json!({"ok": true})))
}

fn parse_id(s: String) -> AppResult<SnowflakeId> {
    crate::types::snowflake_id::parse_id(&s)
}

#[allow(dead_code)]
async fn _noop(_: Pool) {}
