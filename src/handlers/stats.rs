//! 仪表盘统计 API handler
//!
//! 提供 Admin Dashboard 的三个统计端点：
//! - 总览统计
//! - 单个内容类型统计
//! - 趋势数据

use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::services::stats::StatsService;

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub table: Option<String>,
    pub days: Option<i64>,
}

/// 总览统计
///
/// `GET /api/v1/admin/stats`
pub async fn overview(State(state): State<AppState>) -> AppResult<ApiResponse<serde_json::Value>> {
    let svc = StatsService::new(state.pool.clone());
    let data = svc.overview(None).await?;
    Ok(ApiResponse::success(data))
}

/// 单个内容类型统计
///
/// `GET /api/v1/admin/stats/content/:table`
pub async fn content_stats(
    State(state): State<AppState>,
    Path(table): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let svc = StatsService::new(state.pool.clone());
    let data = svc.content_stats(&table, None).await?;
    Ok(ApiResponse::success(data))
}

/// 趋势数据
///
/// `GET /api/v1/admin/stats/trends?table=posts&days=30`
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
