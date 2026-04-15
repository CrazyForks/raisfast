//! 租户管理 API Handler

use axum::Json;
use axum::extract::{Path, State};

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::models::tenant::Tenant;
use crate::services::tenant::{CreateTenantRequest, UpdateTenantRequest};

/// GET /admin/tenants — 列出所有租户
pub async fn list_tenants(State(state): State<AppState>) -> AppResult<ApiResponse<Vec<Tenant>>> {
    let tenants = state.tenant.list().await?;
    Ok(ApiResponse::success(tenants))
}

/// GET /admin/tenants/:id — 获取租户详情
pub async fn get_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<Tenant>> {
    let tenant = state
        .tenant
        .get(&id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("tenant/{id}")))?;
    Ok(ApiResponse::success(tenant))
}

/// POST /admin/tenants — 创建租户
pub async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> AppResult<ApiResponse<Tenant>> {
    let tenant = state.tenant.create(&req).await?;
    Ok(ApiResponse::success(tenant))
}

/// PUT /admin/tenants/:id — 更新租户
pub async fn update_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> AppResult<ApiResponse<Tenant>> {
    let tenant = state.tenant.update(&id, &req).await?;
    Ok(ApiResponse::success(tenant))
}

/// DELETE /admin/tenants/:id — 删除租户
pub async fn delete_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.tenant.delete(&id).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "deleted": true,
    })))
}
