//! Workflow management API handlers

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use serde_json::json;
#[cfg(feature = "export-types")]
use ts_rs::TS;

use super::model::{StepDef, WorkflowInstanceStatus};
use crate::AppState;
use crate::db::dialect;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;

pub fn routes(registry: &mut crate::server::RouteRegistry, config: &crate::config::app::AppConfig) -> axum::Router<crate::AppState> {
    let restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(r, registry, restful, "/admin/workflows", get, list, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows", create, create, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/{id}", get, self::get, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/{id}", delete, self::delete, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/{id}/start", post, start, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/instances", get, list_instances, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/instances/{id}", get, get_instance, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/instances/{id}/execute", post, execute_step, "system admin", "admin/workflows");
    let r = reg_route!(r, registry, restful, "/admin/workflows/instances/{id}/cancel", post, cancel_instance, "system admin", "admin/workflows");
    reg_route!(r, registry, restful, "/admin/workflows/instances/{id}/logs", get, get_step_logs, "system admin", "admin/workflows")
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct CreateWorkflowRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct StartWorkflowRequest {
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub context: serde_json::Value,
    pub triggered_by: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct ExecuteStepRequest {
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub output: serde_json::Value,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Deserialize)]
pub struct InstanceQuery {
    pub definition_id: Option<String>,
    pub status: Option<WorkflowInstanceStatus>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkflowRequest>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let wf = state
        .workflow
        .create_workflow(
            &body.id,
            &body.name,
            body.description.as_deref(),
            &body.steps,
        )
        .await?;
    Ok(ApiResponse::success(
        serde_json::to_value(wf).unwrap_or_default(),
    ))
}

pub async fn list(State(state): State<AppState>) -> AppResult<ApiResponse<serde_json::Value>> {
    let workflows = state.workflow.list_workflows().await?;
    Ok(ApiResponse::success(
        serde_json::to_value(workflows).unwrap_or_default(),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let wf = state.workflow.get_workflow(&id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(wf).unwrap_or_default(),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.workflow.delete_workflow(&id).await?;
    Ok(ApiResponse::success(()))
}

pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<StartWorkflowRequest>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let triggered_by_int: Option<i64> = match &body.triggered_by {
        Some(doc_id) if !doc_id.is_empty() => {
            let sql = format!(
                "SELECT id FROM users WHERE document_id = {}",
                dialect::ph(1)
            );
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(doc_id)
                .fetch_optional(&state.pool)
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?
        }
        _ => None,
    };

    let instance = state
        .workflow
        .start_workflow(&id, &body.context, triggered_by_int)
        .await?;
    Ok(ApiResponse::success(
        serde_json::to_value(instance).unwrap_or_default(),
    ))
}

pub async fn list_instances(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let (items, total) = state
        .workflow
        .list_instances(
            query.definition_id.as_deref(),
            query.status,
            page,
            page_size,
        )
        .await?;
    Ok(ApiResponse::success(json!({
        "items": items,
        "total": total,
        "page": page,
        "page_size": page_size
    })))
}

pub async fn get_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let instance = state
        .workflow
        .get_instance(&id)
        .await?
        .ok_or_else(|| AppError::not_found("workflow instance"))?;
    Ok(ApiResponse::success(
        serde_json::to_value(instance).unwrap_or_default(),
    ))
}

pub async fn execute_step(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExecuteStepRequest>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let instance = state.workflow.execute_step(&id, &body.output).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(instance).unwrap_or_default(),
    ))
}

pub async fn cancel_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    state.workflow.cancel_instance(&id).await?;
    Ok(ApiResponse::success(()))
}

pub async fn get_step_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let logs = state.workflow.get_step_logs(&id).await?;
    Ok(ApiResponse::success(
        serde_json::to_value(logs).unwrap_or_default(),
    ))
}
