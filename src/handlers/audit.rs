//! Audit log API handler

use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::AuditFilter;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::pagination::PaginationParams;

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
        "/admin/audit",
        get,
        list,
        "system admin",
        "admin/audit"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/audit/{id}",
        get,
        self::get,
        "system admin",
        "admin/audit"
    )
}

/// GET /admin/audit — query audit logs (paginated)
pub async fn list(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
    Query(filter): Query<AuditFilter>,
) -> AppResult<
    ApiResponse<crate::errors::response::PaginatedData<crate::models::audit_log::AuditEntry>>,
> {
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

/// GET /admin/audit/:id — get a single audit log entry
pub async fn get(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<ApiResponse<crate::models::audit_log::AuditEntry>> {
    auth.ensure_admin()?;
    let entry = state.audit.get(SnowflakeId(id)).await?;
    Ok(ApiResponse::success(entry))
}
