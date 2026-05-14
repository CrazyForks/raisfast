//! Cron schedule management API handler
//!
//! Provides scheduled task CRUD, start/stop, and execution history query endpoints.
//! All endpoints require admin privileges.

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::utils::pagination::PaginationParams;
use crate::worker::{
    CronSchedule, cleanup_execution_logs, create_schedule, delete_schedule, find_by_id,
    list_execution_logs, list_schedules, recent_execution_logs, toggle_schedule, update_schedule,
};

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/admin/crons",
        get(self::list).post(create),
        "system admin",
        "admin/crons",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/crons/{id}",
        get(self::get).put(update).delete(self::delete),
        "system admin",
        "admin/crons",
        ["GET", "PUT", "DELETE"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/crons/{id}/toggle",
        http_post(toggle),
        "system admin",
        "admin/crons",
        ["POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/crons/logs",
        get(logs),
        "system admin",
        "admin/crons",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/crons/logs/cleanup",
        http_post(cleanup_logs),
        "system admin",
        "admin/crons",
        ["POST"]
    );
    reg_route!(
        r,
        registry,
        "/admin/crons/batch",
        http_post(admin_batch),
        "system admin",
        "admin/crons",
        ["POST"]
    )
}

/// Create schedule request body
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

/// Update schedule request body
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

/// Execution log query parameters
#[derive(Debug, Deserialize)]
pub struct LogQueryParams {
    pub schedule_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

/// GET /api/v1/admin/crons — List all schedules (paginated)
#[utoipa::path(get, path = "/admin/crons", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List cron schedules"))
)]
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<CronSchedule>>> {
    auth.ensure_admin()?;
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

/// GET /api/v1/admin/crons/{id} — Schedule details
#[utoipa::path(get, path = "/admin/crons/{id}", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    responses((status = 200, description = "Schedule detail"))
)]
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<CronSchedule>> {
    auth.ensure_admin()?;
    let schedule = find_by_id(&state.pool, &id)
        .await?
        .ok_or_else(|| AppError::not_found("cron_schedule"))?;
    Ok(ApiResponse::success(schedule))
}

/// POST /api/v1/admin/crons — Create a schedule
#[utoipa::path(post, path = "/admin/crons", tag = "cron",
    security(("bearer_auth" = [])),
    request_body = serde_json::Value,
    responses((status = 200, description = "Schedule created"))
)]
pub async fn create(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    auth.ensure_admin()?;
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

/// PUT /api/v1/admin/crons/{id} — Update a schedule
#[utoipa::path(put, path = "/admin/crons/{id}", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Schedule updated"))
)]
pub async fn update(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    auth.ensure_admin()?;
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

/// POST /api/v1/admin/crons/{id}/toggle — Toggle enable/disable
#[utoipa::path(post, path = "/admin/crons/{id}/toggle", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    request_body = serde_json::Value,
    responses((status = 200, description = "Schedule toggled"))
)]
pub async fn toggle(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    toggle_schedule(&state.pool, &id, body.enabled).await?;
    Ok(ApiResponse::success(()))
}

/// DELETE /api/v1/admin/crons/{id} — Delete a schedule
#[utoipa::path(delete, path = "/admin/crons/{id}", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    responses((status = 200, description = "Schedule deleted"))
)]
pub async fn delete(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    delete_schedule(&state.pool, &id).await?;
    Ok(ApiResponse::success(()))
}

/// GET /api/v1/admin/crons/logs — Query execution logs
///
/// Supports two modes:
/// - `?schedule_id=xxx` — Query a specific schedule's history
/// - Omit — Query recent records for all schedules
#[utoipa::path(get, path = "/admin/crons/logs", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Execution logs"))
)]
pub async fn logs(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<LogQueryParams>,
) -> AppResult<ApiResponse<Vec<crate::worker::CronExecutionLog>>> {
    auth.ensure_admin()?;
    let limit = params.limit.clamp(1, 100);
    let logs = if let Some(ref schedule_id) = params.schedule_id {
        list_execution_logs(&state.pool, schedule_id, limit).await?
    } else {
        recent_execution_logs(&state.pool, limit).await?
    };
    Ok(ApiResponse::success(logs))
}

/// POST /api/v1/admin/crons/logs/cleanup — Clean up expired logs
#[utoipa::path(post, path = "/admin/crons/logs/cleanup", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Expired logs cleaned up"))
)]
pub async fn cleanup_logs(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<u64>> {
    auth.ensure_admin()?;
    let days = state.config.cron_log_retention_days;
    let count = cleanup_execution_logs(&state.pool, days).await?;
    Ok(ApiResponse::success(count))
}

#[derive(Debug, Deserialize)]
pub struct ToggleBody {
    pub enabled: bool,
}

#[utoipa::path(post, path = "/admin/crons/batch", tag = "cron",
    security(("bearer_auth" = [])),
    request_body = BatchRequest,
    responses((status = 200, description = "Batch operation completed"))
)]
pub async fn admin_batch(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    auth.ensure_admin()?;
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if delete_schedule(&state.pool, id).await.is_ok() {
                    affected += 1;
                }
            }
            "enable" => {
                if toggle_schedule(&state.pool, id, true).await.is_ok() {
                    affected += 1;
                }
            }
            "disable" => {
                if toggle_schedule(&state.pool, id, false).await.is_ok() {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
