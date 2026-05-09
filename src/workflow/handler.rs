//! 工作流管理 API Handler

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use serde_json::json;

use super::model::StepDef;
use crate::AppState;
use crate::db::dialect;
use crate::errors::app_error::{AppError, AppResult};

/// 创建工作流定义请求
#[derive(Debug, Deserialize)]
pub struct CreateWorkflowRequest {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub steps: Vec<StepDef>,
}

/// 启动工作流实例请求
#[derive(Debug, Deserialize)]
pub struct StartWorkflowRequest {
    pub context: serde_json::Value,
    pub triggered_by: Option<String>,
}

/// 执行步骤请求
#[derive(Debug, Deserialize)]
pub struct ExecuteStepRequest {
    pub output: serde_json::Value,
}

/// 实例列表查询参数
#[derive(Debug, Deserialize)]
pub struct InstanceQuery {
    pub definition_id: Option<String>,
    pub status: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// POST /admin/workflows — 创建工作流定义
pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateWorkflowRequest>,
) -> AppResult<impl IntoResponse> {
    let wf = state
        .workflow
        .create_workflow(
            &body.id,
            &body.name,
            body.description.as_deref(),
            &body.steps,
        )
        .await?;
    Ok(Json(json!({"code": 0, "message": "created", "data": wf})))
}

/// GET /admin/workflows — 列出所有工作流定义
pub async fn list(State(state): State<AppState>) -> AppResult<impl IntoResponse> {
    let workflows = state.workflow.list_workflows().await?;
    Ok(Json(json!({"code": 0, "message": "ok", "data": workflows})))
}

/// GET /admin/workflows/{id} — 获取工作流定义
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let wf = state.workflow.get_workflow(&id).await?;
    Ok(Json(json!({"code": 0, "message": "ok", "data": wf})))
}

/// DELETE /admin/workflows/{id} — 删除工作流定义
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    state.workflow.delete_workflow(&id).await?;
    Ok(Json(json!({"code": 0, "message": "deleted"})))
}

/// POST /admin/workflows/{id}/start — 启动工作流实例
pub async fn start(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<StartWorkflowRequest>,
) -> AppResult<impl IntoResponse> {
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
    Ok(Json(
        json!({"code": 0, "message": "started", "data": instance}),
    ))
}

/// GET /admin/workflows/instances — 列出工作流实例
pub async fn list_instances(
    State(state): State<AppState>,
    Query(query): Query<InstanceQuery>,
) -> AppResult<impl IntoResponse> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let (items, total) = state
        .workflow
        .list_instances(
            query.definition_id.as_deref(),
            query.status.as_deref(),
            page,
            page_size,
        )
        .await?;
    Ok(Json(json!({
        "code": 0,
        "message": "ok",
        "data": {"items": items, "total": total, "page": page, "page_size": page_size}
    })))
}

/// GET /admin/workflows/instances/{id} — 获取工作流实例
pub async fn get_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let instance = state
        .workflow
        .get_instance(&id)
        .await?
        .ok_or_else(|| AppError::not_found("workflow instance"))?;
    Ok(Json(json!({"code": 0, "message": "ok", "data": instance})))
}

/// POST /admin/workflows/instances/{id}/execute — 执行当前步骤
pub async fn execute_step(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ExecuteStepRequest>,
) -> AppResult<impl IntoResponse> {
    let instance = state.workflow.execute_step(&id, &body.output).await?;
    Ok(Json(json!({"code": 0, "message": "ok", "data": instance})))
}

/// POST /admin/workflows/instances/{id}/cancel — 取消工作流实例
pub async fn cancel_instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    state.workflow.cancel_instance(&id).await?;
    Ok(Json(json!({"code": 0, "message": "cancelled"})))
}

/// GET /admin/workflows/instances/{id}/logs — 获取步骤执行日志
pub async fn get_step_logs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let logs = state.workflow.get_step_logs(&id).await?;
    Ok(Json(json!({"code": 0, "message": "ok", "data": logs})))
}
