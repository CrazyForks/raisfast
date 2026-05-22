//! Cron schedule management API handler
//!
//! Provides scheduled task CRUD, start/stop, and execution history query endpoints.
//! All endpoints require admin privileges.

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{
    BatchRequest, BatchResponse, CreateCronRequest, LogQueryParams, ToggleBody, UpdateCronRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::pagination::PaginationParams;
use crate::worker::{
    CronSchedule, cleanup_execution_logs, create_schedule, delete_schedule, find_by_id,
    list_execution_logs, list_schedules, recent_execution_logs, toggle_schedule, update_schedule,
};

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
        "/admin/crons",
        get,
        self::list,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons",
        create,
        self::create,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        get,
        self::get,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        put,
        update,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        delete,
        self::delete,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}/toggle",
        post,
        toggle,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/logs",
        get,
        logs,
        "system admin",
        "admin/crons"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/logs/cleanup",
        post,
        cleanup_logs,
        "system admin",
        "admin/crons"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/batch",
        post,
        admin_batch,
        "system admin",
        "admin/crons"
    )
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let schedule = find_by_id(&state.pool, id)
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let updated = update_schedule(
        &state.pool,
        id,
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    toggle_schedule(&state.pool, id, body.enabled).await?;
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    delete_schedule(&state.pool, id).await?;
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
        let sid = crate::types::snowflake_id::parse_id(schedule_id)?;
        list_execution_logs(&state.pool, sid, limit).await?
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
    for id_str in &req.ids {
        let id = match id_str.parse::<i64>() {
            Ok(v) => SnowflakeId(v),
            Err(_) => continue,
        };
        match req.action.as_str() {
            "delete" if delete_schedule(&state.pool, id).await.is_ok() => {
                affected += 1;
            }
            "enable" if toggle_schedule(&state.pool, id, true).await.is_ok() => {
                affected += 1;
            }
            "disable" if toggle_schedule(&state.pool, id, false).await.is_ok() => {
                affected += 1;
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
