//! Flow engine HTTP handlers (P1.10 minimal admin surface).
//!
//! Endpoints:
//! - POST /admin/flows                    create flow + first version (definition)
//! - GET  /admin/flows                    list flows (default tenant)
//! - GET  /admin/flows/{id}               flow detail (incl. latest version definition)
//! - PUT  /admin/flows/{id}               update metadata + publish new version
//! - DELETE /admin/flows/{id}             cascade-delete flow + versions/instances
//! - POST /admin/flows/{id}/run           create + execute an instance (inputs)
//! - GET  /admin/flows/instances/{id}     instance status/outputs
//! - POST /admin/flows/instances/{id}/stop  mark canceled
//!
//! cron/event triggers + draft/publish (explicit) land in P2.

use axum::Json;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::AppState;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

use super::exec::FlowsExec;
use super::graph;
use super::model::{self, Flow, FlowInstance, FlowVersion};

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
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub definition: Option<Value>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub version_number: Option<i64>,
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

async fn list_flows(State(state): State<AppState>) -> AppResult<ApiResponse<Value>> {
    let flows = model::find_flows_by_tenant(&state.pool, crate::constants::DEFAULT_TENANT).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(flows).unwrap_or_default(),
    ))
}

/// Flow detail incl. the latest published version's definition (for the editor).
async fn get_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = parse_id(id)?;
    let flow = model::find_flow_by_id(&state.pool, flow_id).await?;
    let latest = model::latest_version(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(&FlowDetail {
            id: flow.id,
            tenant_id: flow.tenant_id,
            name: flow.name,
            description: flow.description,
            enabled: flow.enabled,
            definition: latest.as_ref().map(|v| v.definition.clone()),
            version_number: latest.map(|v| v.version_number),
            created_at: flow.created_at,
            updated_at: flow.updated_at,
        })
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize flow detail: {e}")))?,
    ))
}

/// Update metadata (name/description) and/or publish a new version (definition).
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
    }

    let fresh = model::find_flow_by_id(&state.pool, flow_id).await?;
    let latest = model::latest_version(&state.pool, flow_id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(&FlowDetail {
            id: fresh.id,
            tenant_id: fresh.tenant_id,
            name: fresh.name,
            description: fresh.description,
            enabled: fresh.enabled,
            definition: latest.as_ref().map(|v| v.definition.clone()),
            version_number: latest.map(|v| v.version_number),
            created_at: fresh.created_at,
            updated_at: fresh.updated_at,
        })
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize flow detail: {e}")))?,
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

/// Create an instance from the flow's current version and execute it.
async fn run_flow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RunFlowReq>,
) -> AppResult<ApiResponse<Value>> {
    let flow_id = crate::types::snowflake_id::parse_id(&id)?;
    let flow = model::find_flow_by_id(&state.pool, flow_id).await?;
    let version = model::latest_version(&state.pool, flow_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    // Definitions validated at create; validate this snapshot too.
    graph::load_definition(&version.definition)?;

    let instance_id = crate::utils::id::new_snowflake_id();
    let now = now();
    let instance = FlowInstance {
        id: instance_id,
        tenant_id: flow.tenant_id.clone(),
        flow_id,
        flow_version_id: version.id,
        status: "running".into(),
        has_exceptions: false,
        trigger_kind: "api".into(),
        trigger_payload: req.inputs,
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

    let done = model::find_instance_by_id(&state.pool, instance_id).await?;
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
