//! 审计日志 API Handler

use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::utils::pagination::PaginationParams;

/// GET /admin/audit — 查询审计日志（分页）
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
    Query(filter): Query<AuditFilter>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<crate::audit::model::AuditEntry>>>
{
    auth.ensure_admin()?;
    params.sanitize();
    let (items, total) = state
        .audit
        .list(
            auth.tenant_id(),
            filter.action.as_deref(),
            filter.actor_id,
            params.page,
            params.page_size,
        )
        .await?;
    Ok(params.paginate(items, total))
}

/// GET /admin/audit/:id — 获取单条审计日志
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<ApiResponse<crate::audit::model::AuditEntry>> {
    auth.ensure_admin()?;
    let entry = state.audit.get(id).await?;
    Ok(ApiResponse::success(entry))
}

#[derive(Debug, Deserialize)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub actor_id: Option<i64>,
}
