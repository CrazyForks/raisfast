//! RBAC management API handler

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::dto::{BatchRequest, BatchResponse};
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::models::rbac::Role;
use crate::services::rbac::{
    CreateRoleRequest, PermissionView, SetPermissionsRequest, UpdateRoleRequest,
};
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
        "/admin/rbac/roles",
        get,
        list_roles,
        "system admin",
        "admin/rbac"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles",
        create,
        create_role,
        "system admin",
        "admin/rbac"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles/{id}",
        put,
        update_role,
        "system admin",
        "admin/rbac"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles/{id}",
        delete,
        delete_role,
        "system admin",
        "admin/rbac"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles/{id}/permissions",
        get,
        get_permissions,
        "system admin",
        "admin/rbac"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles/{id}/permissions",
        put,
        set_permissions,
        "system admin",
        "admin/rbac"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/rbac/roles/batch",
        post,
        admin_batch,
        "system admin",
        "admin/rbac"
    )
}

/// GET /admin/rbac/roles — List all roles (paginated)
#[utoipa::path(get, path = "/admin/rbac/roles", tag = "rbac",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Role list"))
)]
pub async fn list_roles(
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<Role>>> {
    params.sanitize();
    let all = state.rbac.list_roles().await?;
    Ok(params.paginate_in_memory(all))
}

/// POST /admin/rbac/roles — Create a role
#[utoipa::path(post, path = "/admin/rbac/roles", tag = "rbac",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Role created"))
)]
pub async fn create_role(
    State(state): State<AppState>,
    Json(req): Json<CreateRoleRequest>,
) -> AppResult<ApiResponse<Role>> {
    let role = state.rbac.create_role(&req).await?;
    Ok(ApiResponse::success(role))
}

/// PUT /admin/rbac/roles/:id — Update a role
#[utoipa::path(put, path = "/admin/rbac/roles/{id}", tag = "rbac",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Role ID")),
    responses((status = 200, description = "Role updated"))
)]
pub async fn update_role(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<Role>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let role = state.rbac.update_role(id, &req).await?;
    Ok(ApiResponse::success(role))
}

/// DELETE /admin/rbac/roles/:id — Delete a role
#[utoipa::path(delete, path = "/admin/rbac/roles/{id}", tag = "rbac",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Role ID")),
    responses((status = 200, description = "Role deleted"))
)]
pub async fn delete_role(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let id = crate::types::snowflake_id::parse_id(&id)?;
    state.rbac.delete_role(id).await?;
    Ok(ApiResponse::success(serde_json::json!({"deleted": true})))
}

/// GET /admin/rbac/roles/:id/permissions — Get role permissions
#[utoipa::path(get, path = "/admin/rbac/roles/{id}/permissions", tag = "rbac",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Role ID")),
    responses((status = 200, description = "Role permissions"))
)]
pub async fn get_permissions(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> AppResult<ApiResponse<Vec<PermissionView>>> {
    let perms = state.rbac.get_permissions(&role_id).await?;
    Ok(ApiResponse::success(perms))
}

/// PUT /admin/rbac/roles/:id/permissions — Set role permissions (replace all)
#[utoipa::path(put, path = "/admin/rbac/roles/{id}/permissions", tag = "rbac",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "Role ID")),
    responses((status = 200, description = "Permissions updated"))
)]
pub async fn set_permissions(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
    Json(req): Json<SetPermissionsRequest>,
) -> AppResult<ApiResponse<Vec<PermissionView>>> {
    let perms = state
        .rbac
        .set_permissions(&role_id, &req.permissions)
        .await?;
    Ok(ApiResponse::success(perms))
}

#[utoipa::path(post, path = "/admin/rbac/roles/batch", tag = "rbac",
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
    if req.action == "delete" {
        for raw_id in &req.ids {
            let Ok(id) = crate::types::snowflake_id::parse_id(raw_id) else {
                continue;
            };
            if state.rbac.delete_role(id).await.is_ok() {
                affected += 1;
            }
        }
    }
    Ok(ApiResponse::success(BatchResponse::new(
        &req.action,
        affected,
    )))
}
