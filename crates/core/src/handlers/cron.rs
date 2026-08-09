//! Cron schedule management API handler
//!
//! Provides scheduled task CRUD, start/stop, and execution history query endpoints.
//! All endpoints require admin privileges.

use std::sync::Arc;
use std::time::Duration;

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
    CronExecStatus, CronSchedule, CronScheduler, DefaultJobQueue, JobQueue, cleanup_execution_logs,
    create_execution_log, create_schedule, delete_schedule, find_by_id, list_execution_logs,
    list_schedules, recent_execution_logs, toggle_schedule, update_schedule,
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
        "/admin/crons/{id}/run",
        post,
        self::run_now,
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
    State(state): State<AppState>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let mut metas = state.handler_registry.list_meta();
    // Sort by category then id for deterministic ordering
    metas.sort_by(|a, b| a.category.cmp(b.category).then_with(|| a.id.cmp(b.id)));

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

    // Validate exec_kind value
    match req.exec_kind.as_str() {
        "builtin" | "script" | "plugin" => {}
        "system" => {
            #[cfg(not(feature = "cron-system"))]
            {
                return Err(AppError::BadRequest(
                    "system exec_kind requires the 'cron-system' cargo feature".into(),
                ));
            }
            #[cfg(feature = "cron-system")]
            {
                if !state.config.cron_allow_system_scripts {
                    return Err(AppError::BadRequest(
                        "system scripts are disabled (cron_allow_system_scripts=false)".into(),
                    ));
                }
            }
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "invalid exec_kind: '{other}' (expected: builtin, script, system, plugin)"
            )));
        }
    }

    // Validate script fields when exec_kind=script
    if req.exec_kind == "script" {
        let lang = req.script_lang.as_deref().ok_or_else(|| {
            AppError::BadRequest("script_lang is required for exec_kind=script".into())
        })?;
        match lang {
            #[cfg(feature = "plugin-js")]
            "js" => {}
            #[cfg(feature = "plugin-lua")]
            "lua" => {}
            #[cfg(feature = "plugin-rhai")]
            "rhai" => {}
            other => {
                return Err(AppError::BadRequest(format!(
                    "unsupported script_lang: '{other}' (available: js, lua, rhai)"
                )));
            }
        }
        if req.script_source.as_deref().unwrap_or("").trim().is_empty() {
            return Err(AppError::BadRequest(
                "script_source is required for exec_kind=script".into(),
            ));
        }
    }

    // Serialize params to string if present
    let params_str = req
        .params
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| AppError::Internal(e.into()))?;

    // Determine which constructor to use
    let schedule = if req.exec_kind == "script" {
        let lang = req.script_lang.as_deref().unwrap();
        let source = req.script_source.as_deref().unwrap();
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
    } else if req.exec_kind == "system" {
        let command = req.script_source.as_deref().unwrap_or("");
        crate::worker::create_system_schedule(
            &state.pool,
            &req.label,
            &req.cron_expr,
            req.enabled,
            command,
            req.use_shell.unwrap_or(true),
            req.timeout_secs,
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

/// POST /api/v1/admin/crons/{id}/run — Trigger immediate execution
///
/// Enqueues the schedule's job immediately regardless of its `next_run_at` or `enabled` status.
/// Does NOT advance `next_run_at` — the regular tick will still fire at the scheduled time.
#[utoipa::path(post, path = "/admin/crons/{id}/run", tag = "cron",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Schedule ID")),
    responses((status = 200, description = "Job enqueued for immediate execution"))
)]
pub async fn run_now(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let sid = crate::types::snowflake_id::parse_id(&id)?;
    let schedule = find_by_id(&state.pool, sid)
        .await?
        .ok_or_else(|| AppError::not_found("cron_schedule"))?;

    // Build the job using the same logic as CronScheduler::dispatch
    let scheduler = CronScheduler::new(
        state.pool.clone(),
        Arc::new(DefaultJobQueue::new(state.pool.clone())),
        Duration::from_secs(60),
    );
    let job = scheduler.build_job(&schedule)?;
    let mut new_job = crate::worker::NewJob::from(job);
    new_job.cron_schedule_id = Some(schedule.id);

    // Create execution log row + enqueue
    let log_id = create_execution_log(
        &state.pool,
        schedule.id,
        &schedule.job_type,
        &schedule.label,
    )
    .await
    .ok();
    new_job.cron_log_id = log_id.map(crate::types::snowflake_id::SnowflakeId);

    let queue = DefaultJobQueue::new(state.pool.clone());
    queue.enqueue(new_job).await?;

    // Mark log as dispatched
    if let Some(lid) = log_id {
        raisfast_derive::crud_update!(&state.pool, "cron_execution_log",
            bind: ["status" => CronExecStatus::Dispatched],
            where: ("id", lid)
        )?;
    }

    tracing::info!("manual run triggered for schedule '{}'", schedule.label);
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
        recent_execution_logs(&state.pool, page_size, offset).await?
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
