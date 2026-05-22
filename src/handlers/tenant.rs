//! Tenant management API handler

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse, TenantResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::services::tenant::{CreateTenantRequest, UpdateTenantRequest};
use crate::utils::pagination::PaginationParams;

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
        "/admin/tenants",
        get,
        list_tenants,
        "system admin",
        "admin/tenants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tenants",
        create,
        create_tenant,
        "system admin",
        "admin/tenants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tenants/{id}",
        get,
        get_tenant,
        "system admin",
        "admin/tenants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tenants/{id}",
        put,
        update_tenant,
        "system admin",
        "admin/tenants"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/tenants/{id}",
        delete,
        delete_tenant,
        "system admin",
        "admin/tenants"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/tenants/batch",
        post,
        admin_batch,
        "system admin",
        "admin/tenants"
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
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<TenantResponse>>> {
    params.sanitize();
    let all = state.tenant.list().await?;
    let all: Vec<TenantResponse> = all.into_iter().map(TenantResponse::from_tenant).collect();
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
) -> AppResult<ApiResponse<TenantResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let tenant = state
        .tenant
        .get(id)
        .await?
        .ok_or_else(|| AppError::not_found(&format!("tenant/{id}")))?;
    Ok(ApiResponse::success(TenantResponse::from_tenant(tenant)))
}

/// POST /admin/tenants — Create a tenant
#[utoipa::path(post, path = "/admin/tenants", tag = "tenants",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Tenant created"))
)]
pub async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> AppResult<ApiResponse<TenantResponse>> {
    let tenant = state.tenant.create(&req).await?;
    Ok(ApiResponse::success(TenantResponse::from_tenant(tenant)))
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
) -> AppResult<ApiResponse<TenantResponse>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let tenant = state.tenant.update(id, &req).await?;
    Ok(ApiResponse::success(TenantResponse::from_tenant(tenant)))
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
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.tenant.delete(id).await?;
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
    for raw_id in &req.ids {
        let Ok(id) = crate::types::snowflake_id::parse_id(raw_id) else {
            continue;
        };
        match req.action.as_str() {
            "delete" if state.tenant.delete(id).await.is_ok() => {
                affected += 1;
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
