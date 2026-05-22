//! Dashboard statistics API handler
//!
//! Provides three statistics endpoints for the Admin Dashboard:
//! - Overview statistics
//! - Single content type statistics
//! - Trend data

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::TrendsQuery;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::services::stats::StatsService;

pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    config: &crate::config::app::AppConfig,
) -> axum::Router<crate::AppState> {
    let _restful = config.api_restful;
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/stats",
        get,
        overview,
        "system admin",
        "admin/stats"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/stats/content/{table}",
        get,
        content_stats,
        "system admin",
        "admin/stats"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/stats/trends",
        get,
        trends,
        "system admin",
        "admin/stats"
    )
}

/// Overview statistics
///
/// `GET /api/v1/admin/stats`
#[utoipa::path(get, path = "/admin/stats", tag = "stats",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Overview statistics"))
)]
pub async fn overview(State(state): State<AppState>) -> AppResult<ApiResponse<serde_json::Value>> {
    let svc = StatsService::new(state.pool.clone());
    let data = svc.overview(None).await?;
    Ok(ApiResponse::success(data))
}

/// Single content type statistics
///
/// `GET /api/v1/admin/stats/content/:table`
#[utoipa::path(get, path = "/admin/stats/content/{table}", tag = "stats",
    security(("bearer_auth" = [])),
    params(("table" = String, Path, description = "Content table name")),
    responses((status = 200, description = "Content statistics"))
)]
pub async fn content_stats(
    State(state): State<AppState>,
    Path(table): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let svc = StatsService::new(state.pool.clone());
    let data = svc.content_stats(&table, None).await?;
    Ok(ApiResponse::success(data))
}

/// Trend data
///
/// `GET /api/v1/admin/stats/trends?table=posts&days=30`
#[utoipa::path(get, path = "/admin/stats/trends", tag = "stats",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Trend data"))
)]
pub async fn trends(
    State(state): State<AppState>,
    Query(query): Query<TrendsQuery>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let table = query.table.as_deref().unwrap_or("posts");
    let days = query.days.unwrap_or(30);

    let svc = StatsService::new(state.pool.clone());
    let data = svc.trends(table, days, None).await?;
    Ok(ApiResponse::success(data))
}
