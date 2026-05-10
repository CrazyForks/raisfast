//! 租户管理 API Handler

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::models::tenant::Tenant;
use crate::services::tenant::{CreateTenantRequest, UpdateTenantRequest};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post};

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/admin/tenants", get(list_tenants).post(create_tenant), "system admin", "admin/tenants", ["GET", "POST"]);
    let r = reg_route!(r, registry, "/admin/tenants/{id}", get(get_tenant).put(update_tenant).delete(delete_tenant), "system admin", "admin/tenants", ["GET", "PUT", "DELETE"]);
    reg_route!(r, registry, "/admin/tenants/batch", http_post(admin_batch), "system admin", "admin/tenants", ["POST"])
}


/// GET /admin/tenants — 列出所有租户（分页）
pub async fn list_tenants(
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<Tenant>>> {
    params.sanitize();
    let all = state.tenant.list().await?;
    Ok(params.paginate_in_memory(all))
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

pub async fn admin_batch(
    State(state): State<AppState>,
    Json(req): Json<BatchRequest>,
) -> AppResult<ApiResponse<BatchResponse>> {
    crate::errors::validation::validate(&req)?;
    let mut affected = 0usize;
    for id in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if state.tenant.delete(id).await.is_ok() {
                    affected += 1;
                }
            }
            "suspend" | "activate" => {
                let status = if req.action == "suspend" {
                    "suspended"
                } else {
                    "active"
                };
                if state
                    .tenant
                    .update(
                        id,
                        &UpdateTenantRequest {
                            name: None,
                            domain: None,
                            config: None,
                            status: Some(status.to_string()),
                        },
                    )
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(&req.action, affected)))
}
