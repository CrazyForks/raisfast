//! Tenant management API handler

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
    let r = reg_route!(
        r,
        registry,
        "/admin/tenants",
        get(list_tenants).post(create_tenant),
        "system admin",
        "admin/tenants",
        ["GET", "POST"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/tenants/{id}",
        get(get_tenant).put(update_tenant).delete(delete_tenant),
        "system admin",
        "admin/tenants",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/tenants/batch",
        http_post(admin_batch),
        "system admin",
        "admin/tenants",
        ["POST"]
    )
}

/// GET /admin/tenants — List all tenants (paginated)
#[utoipa::path(get, path = "/admin/tenants", tag = "tenants",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Tenant list"))
)]
pub async fn list_tenants(
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<Tenant>>> {
    params.sanitize();
    let all = state.tenant.list().await?;
    Ok(params.paginate_in_memory(all))
}

/// GET /admin/tenants/:id — Get tenant details
#[utoipa::path(get, path = "/admin/tenants/{id}", tag = "tenants",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Tenant ID")),
    responses((status = 200, description = "Tenant details"))
)]
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

/// POST /admin/tenants — Create a tenant
#[utoipa::path(post, path = "/admin/tenants", tag = "tenants",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Tenant created"))
)]
pub async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> AppResult<ApiResponse<Tenant>> {
    let tenant = state.tenant.create(&req).await?;
    Ok(ApiResponse::success(tenant))
}

/// PUT /admin/tenants/:id — Update a tenant
#[utoipa::path(put, path = "/admin/tenants/{id}", tag = "tenants",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Tenant ID")),
    responses((status = 200, description = "Tenant updated"))
)]
pub async fn update_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> AppResult<ApiResponse<Tenant>> {
    let tenant = state.tenant.update(&id, &req).await?;
    Ok(ApiResponse::success(tenant))
}

/// DELETE /admin/tenants/:id — Delete a tenant
#[utoipa::path(delete, path = "/admin/tenants/{id}", tag = "tenants",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Tenant ID")),
    responses((status = 200, description = "Tenant deleted"))
)]
pub async fn delete_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.tenant.delete(&id).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "deleted": true,
    })))
}

#[utoipa::path(post, path = "/admin/tenants/batch", tag = "tenants",
    security(("bearer_auth" = [])),
    request_body = BatchRequest,
    responses((status = 200, description = "Batch operation completed"))
)]
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
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
