//! Background job queue inspection API handler
//!
//! Exposes the worker `jobs` table to the admin UI: status-filtered listing,
//! queue stats, retry of dead jobs, row removal, and old-finished-job cleanup.
//! All endpoints require admin privileges.

use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::pagination::PaginationParams;
use crate::utils::tz::Timestamp;
use crate::worker::{DefaultJobQueue, JobFilter, JobQueue, JobRow, JobStats, JobStatus};

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
        "/admin/jobs",
        get,
        self::list,
        "system",
        "admin/jobs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/jobs/stats",
        get,
        self::stats,
        "system",
        "admin/jobs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/jobs/types",
        get,
        self::types,
        "system",
        "admin/jobs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/jobs/{id}/retry",
        post,
        self::retry,
        "system",
        "admin/jobs",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/jobs/{id}",
        delete,
        self::remove,
        "system",
        "admin/jobs",
        "admin"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/jobs/cleanup",
        post,
        self::cleanup,
        "system",
        "admin/jobs",
        "admin"
    )
}

/// Query params for GET /admin/jobs
#[derive(Debug, Deserialize)]
pub struct JobQueryParams {
    /// Optional status filter: pending|running|completed|failed|dead
    pub status: Option<String>,
    /// Optional exact job_type filter (values from GET /admin/jobs/types)
    pub job_type: Option<String>,
    #[serde(default = "crate::utils::pagination::default_page")]
    pub page: i64,
    #[serde(default = "crate::utils::pagination::default_page_size")]
    pub page_size: i64,
}

/// Admin-facing job row. Same shape as [`JobRow`] but with a wire-encoded id.
#[derive(Debug, serde::Serialize)]
pub struct JobDto {
    pub id: SnowflakeId,
    pub job_type: String,
    pub payload: serde_json::Value,
    pub status: JobStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub run_after: Option<Timestamp>,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl From<JobRow> for JobDto {
    fn from(row: JobRow) -> Self {
        Self {
            id: SnowflakeId(row.id.parse::<i64>().unwrap_or(0)),
            job_type: row.job_type,
            payload: row.payload,
            status: row.status,
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            run_after: row.run_after,
            error: row.error,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// GET /api/v1/admin/jobs — List queued jobs (status/job_type filter + pagination)
#[utoipa::path(get, path = "/admin/jobs", tag = "jobs",
    security(("bearer_auth" = [])),
    params(
        ("status" = Option<String>, Query, description = "Filter by status: pending|running|completed|failed|dead"),
        ("job_type" = Option<String>, Query, description = "Filter by exact job_type"),
        ("page" = Option<i64>, Query, description = "Page number (default 1)"),
        ("page_size" = Option<i64>, Query, description = "Items per page (default 20, max 100)"),
    ),
    responses((status = 200, description = "Paginated job list"))
)]
pub async fn list(
    _auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<JobQueryParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<JobDto>>> {
    let filter = JobFilter {
        status: match params.status.as_deref() {
            None | Some("") => None,
            Some(s) => Some(s.parse::<JobStatus>().map_err(AppError::BadRequest)?),
        },
        job_type: params
            .job_type
            .as_deref()
            .filter(|t| !t.is_empty())
            .map(str::to_string),
    };
    let pg = PaginationParams::from_options(Some(params.page), Some(params.page_size));

    let queue = DefaultJobQueue::new(state.pool.clone());
    let (items, total) = queue.list(filter, pg.page, pg.page_size).await?;
    Ok(pg.paginate(items.into_iter().map(JobDto::from).collect(), total))
}

/// GET /api/v1/admin/jobs/types — Distinct job_type values (for filter dropdowns)
#[utoipa::path(get, path = "/admin/jobs/types", tag = "jobs",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Distinct job types in the queue"))
)]
pub async fn types(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<String>>> {
    let queue = DefaultJobQueue::new(state.pool.clone());
    Ok(ApiResponse::success(queue.list_job_types().await?))
}

/// GET /api/v1/admin/jobs/stats — Queue depth per status
#[utoipa::path(get, path = "/admin/jobs/stats", tag = "jobs",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Counts per job status"))
)]
pub async fn stats(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<JobStats>> {
    let queue = DefaultJobQueue::new(state.pool.clone());
    Ok(ApiResponse::success(queue.stats().await?))
}

/// POST /api/v1/admin/jobs/{id}/retry — Requeue a dead job
///
/// Resets the job to `pending` with `attempts = 0`. Only jobs currently in
/// `dead` status are eligible; anything else returns 404.
#[utoipa::path(post, path = "/admin/jobs/{id}/retry", tag = "jobs",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Job ID")),
    responses((status = 200, description = "Job requeued as pending"))
)]
pub async fn retry(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let sid = crate::types::snowflake_id::parse_id(&id)?;
    let queue = DefaultJobQueue::new(state.pool.clone());
    queue.retry(&sid.0.to_string()).await?;
    Ok(ApiResponse::success(()))
}

/// DELETE /api/v1/admin/jobs/{id} — Remove a job row
#[utoipa::path(delete, path = "/admin/jobs/{id}", tag = "jobs",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Job ID")),
    responses((status = 200, description = "Job removed"))
)]
pub async fn remove(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    let sid = crate::types::snowflake_id::parse_id(&id)?;
    let queue = DefaultJobQueue::new(state.pool.clone());
    queue.remove(&sid.0.to_string()).await?;
    Ok(ApiResponse::success(()))
}

/// POST /api/v1/admin/jobs/cleanup — Purge completed/dead jobs older than 7 days
#[utoipa::path(post, path = "/admin/jobs/cleanup", tag = "jobs",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Number of purged jobs"))
)]
pub async fn cleanup(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<u64>> {
    let queue = DefaultJobQueue::new(state.pool.clone());
    let count = queue.cleanup().await?;
    Ok(ApiResponse::success(count))
}
