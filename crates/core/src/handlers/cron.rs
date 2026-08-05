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
        "/admin/cron-handlers",
        get,
        self::list_handlers,
        "system",
        "admin/cron-handlers",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons",
        get,
        self::list,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons",
        create,
        self::create,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        get,
        self::get,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        put,
        update,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}",
        delete,
        self::delete,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/{id}/toggle",
        post,
        toggle,
        "system",
        "admin/crons",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/logs",
        get,
        logs,
        "system",
        "admin/cron_execution_log",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/logs/cleanup",
        post,
        cleanup_logs,
        "system",
        "admin/cron_execution_log",
        "admin"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/crons/batch",
        post,
        admin_batch,
        "system",
        "admin/crons",
        "admin"
    )
}

/// GET /api/v1/admin/cron-handlers — List all cron handler metadata (task menu)
#[utoipa::path(get, path = "/admin/cron-handlers", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Cron handler metadata grouped by category"))
)]
pub async fn list_handlers(
    _auth: AuthUser,
    State(_state): State<AppState>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let metas = crate::worker::handlers::cron_handler_metas();

    // Group by category
    use std::collections::BTreeMap;
    let mut categories: BTreeMap<&str, Vec<serde_json::Value>> = BTreeMap::new();
    for m in metas {
        let entry = serde_json::json!({
            "id": m.id,
            "display_name": m.display_name,
            "description": m.description,
            "category": m.category,
            "params_schema": m.params_schema.and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            "has_params": m.params_schema.is_some(),
            "icon": m.icon,
        });
        categories.entry(m.category).or_default().push(entry);
    }

    let result: Vec<serde_json::Value> = categories
        .into_iter()
        .map(|(category, handlers)| {
            serde_json::json!({ "category": category, "handlers": handlers })
        })
        .collect();

    Ok(ApiResponse::success(serde_json::json!({
        "categories": result
    })))
}

/// GET /api/v1/admin/crons — List all schedules (paginated)
#[utoipa::path(get, path = "/admin/crons", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "List cron schedules"))
)]
pub async fn list(
    _auth: AuthUser,
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

/// GET /api/v1/admin/crons/{id} — Schedule details
#[utoipa::path(get, path = "/admin/crons/{id}", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    responses((status = 200, description = "Schedule detail"))
)]
pub async fn get(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<CronSchedule>> {
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<CreateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    crate::errors::validation::validate(&req)?;

    // Serialize params to string if present
    let params_str = req
        .params
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Internal(e.into()))?;

    // Determine which constructor to use
    let schedule = if req.exec_kind == "script" {
        let lang = req.script_lang.as_deref().ok_or_else(|| {
            AppError::BadRequest("script_lang is required for exec_kind=script".into())
        })?;
        let source = req.script_source.as_deref().ok_or_else(|| {
            AppError::BadRequest("script_source is required for exec_kind=script".into())
        })?;
        crate::worker::create_script_schedule(
            &state.pool,
            &req.label,
            &req.cron_expr,
            req.enabled,
            lang,
            source,
            req.script_entry.as_deref(),
        )
        .await?
    } else if req.exec_kind == "builtin" && req.handler_id.is_some() {
        // New path: use handler_id + params
        let handler_id = req.handler_id.as_deref().unwrap();
        crate::worker::create_schedule_v2(
            &state.pool,
            &req.label,
            &req.cron_expr,
            req.enabled,
            "builtin",
            Some(handler_id),
            params_str.as_deref(),
        )
        .await?
    } else {
        // Legacy path: job_type + payload
        let jt = req
            .handler_id
            .as_deref()
            .or(req.job_type.as_deref())
            .unwrap_or("custom");
        create_schedule(
            &state.pool,
            &req.label,
            jt,
            params_str.as_deref().or(req.payload.as_deref()),
            &req.cron_expr,
            req.enabled,
        )
        .await?
    };

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
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronRequest>,
) -> AppResult<ApiResponse<CronSchedule>> {
    crate::errors::validation::validate(&req)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;

    // Serialize params JSON to string
    let params_str = req
        .params
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Internal(e.into()))?;

    let updated = update_schedule(
        &state.pool,
        id,
        req.label,
        req.cron_expr,
        req.enabled,
        req.exec_kind,
        req.handler_id,
        params_str,
        req.script_lang,
        req.script_source,
        req.script_entry,
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ToggleBody>,
) -> AppResult<ApiResponse<()>> {
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<LogQueryParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::worker::CronExecutionLog>>>
{
    let page = params.page.max(1);
    let page_size = params.page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;

    let (items, total) = if let Some(ref schedule_id) = params.schedule_id {
        let sid = crate::types::snowflake_id::parse_id(schedule_id)?;
        list_execution_logs(&state.pool, sid, page_size, offset).await?
    } else {
        // Recent logs across all schedules — use limit = page_size, no offset pagination
        let logs = recent_execution_logs(&state.pool, page_size).await?;
        let count = logs.len() as i64;
        return Ok(ApiResponse::success(
            crate::errors::response::PaginatedData {
                items: logs,
                total: count,
                page,
                page_size,
            },
        ));
    };

    Ok(ApiResponse::success(
        crate::errors::response::PaginatedData {
            items,
            total,
            page,
            page_size,
        },
    ))
}

/// POST /api/v1/admin/crons/logs/cleanup — Clean up expired logs
#[utoipa::path(post, path = "/admin/crons/logs/cleanup", tag = "cron",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Expired logs cleaned up"))
)]
pub async fn cleanup_logs(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<u64>> {
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
    _auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
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
