//! Cron 调度管理 API handler
//!
//! 提供定时任务的 CRUD、启停、执行历史查询端点。
//! 所有端点需要管理员权限。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AdminUser;
use crate::utils::pagination::PaginationParams;
use crate::worker::{
    CronSchedule, cleanup_execution_logs, create_schedule, delete_schedule, find_by_id,
    list_execution_logs, list_schedules, recent_execution_logs, toggle_schedule, update_schedule,
};

/// 创建调度请求体
#[derive(Debug, Deserialize, validator::Validate)]
pub struct CreateCronRequest {
    #[validate(length(min = 1, message = "label is required"))]
    pub label: String,
    #[validate(length(min = 1, message = "job_type is required"))]
    pub job_type: String,
    pub payload: Option<String>,
    #[validate(length(min = 1, message = "cron_expr is required"))]
    pub cron_expr: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 更新调度请求体
#[derive(Debug, Deserialize, validator::Validate)]
pub struct UpdateCronRequest {
    #[validate(length(min = 1, message = "label is required"))]
    pub label: Option<String>,
    pub job_type: Option<String>,
    pub payload: Option<Option<String>>,
    #[validate(length(min = 1, message = "cron_expr is required"))]
    pub cron_expr: Option<String>,
    pub enabled: Option<bool>,
}

/// 执行日志查询参数
#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    pub schedule_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

/// GET /api/v1/admin/crons — 列出所有调度（分页）
pub async fn list(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<CronSchedule>>> {
    params.sanitize();
    let all = list_schedules(&state.pool).await?;
    let total = all.len() as i64;
    let offset = params.offset() as usize;
    let items: Vec<_> = all
        .into_iter()
        .skip(offset)
        .take(params.page_size as usize)
        .collect();
    Ok(params.paginate(items, total))
}

/// GET /api/v1/admin/crons/{id} — 调度详情
pub async fn get(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<CronSchedule>> {
    let schedule = find_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found("cron_schedule"))?;
    Ok(ApiResponse::success(schedule))
}

/// POST /api/v1/admin/crons — 创建调度
pub async fn create(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    crate::errors::validation::validate(&req)?;
    let schedule = create_schedule(
        &state.pool,
        &req.label,
        &req.job_type,
        req.payload.as_deref(),
        &req.cron_expr,
        req.enabled,
    )
    .await?;
    Ok(ApiResponse::success(schedule))
}

/// PUT /api/v1/admin/crons/{id} — 更新调度
pub async fn update(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    crate::errors::validation::validate(&req)?;
    let updated = update_schedule(
        &state.pool,
        &id,
        req.label,
        req.job_type,
        req.payload,
        req.cron_expr,
        req.enabled,
    )
    .await?;
    Ok(ApiResponse::success(updated))
}

/// POST /api/v1/admin/crons/{id}/toggle — 启停切换
pub async fn toggle(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> AppResult<ApiResponse<()>> {
    toggle_schedule(&state.pool, &id, body.enabled).await?;
    Ok(ApiResponse::success(()))
}

/// DELETE /api/v1/admin/crons/{id} — 删除调度
pub async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    delete_schedule(&state.pool, &id).await?;
    Ok(ApiResponse::success(()))
}

/// GET /api/v1/admin/crons/logs — 查询执行日志
///
/// 支持两种模式：
/// - `?schedule_id=xxx` — 查某个调度的历史
/// - 不传 — 查所有调度的最近记录
pub async fn logs(
    _admin: AdminUser,
    State(state): State<AppState>,
    Query(params): Query<LogQueryParams>,
) -> AppResult<ApiResponse<Vec<crate::worker::CronExecutionLog>>> {
    let limit = params.limit.clamp(1, 100);
    let logs = if let Some(ref schedule_id) = params.schedule_id {
        list_execution_logs(&state.pool, schedule_id, limit).await?
    } else {
        recent_execution_logs(&state.pool, limit).await?
    };
    Ok(ApiResponse::success(logs))
}

/// POST /api/v1/admin/crons/logs/cleanup — 清理过期日志
pub async fn cleanup_logs(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<u64>> {
    let days = state.config.cron_log_retention_days;
    let count = cleanup_execution_logs(&state.pool, days).await?;
    Ok(ApiResponse::success(count))
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}
