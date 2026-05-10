//! RBAC 管理 API Handler

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::AppState;
use crate::errors::app_error::AppResult;
use crate::errors::response::ApiResponse;
use crate::models::rbac::Role;
use crate::services::rbac::{
    CreateRoleRequest, PermissionView, SetPermissionsRequest, UpdateRoleRequest,
};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, put};

    let r = axum::Router::new();
    let r = reg_route!(r, registry, "/admin/rbac/roles", get(list_roles).post(create_role), "system", "admin/rbac", ["GET", "POST"]);
    let r = reg_route!(r, registry, "/admin/rbac/roles/{id}", put(update_role).delete(delete_role), "system", "admin/rbac", ["PUT", "DELETE"]);
    reg_route!(r, registry, "/admin/rbac/roles/{id}/permissions", get(get_permissions).put(set_permissions), "system", "admin/rbac", ["GET", "PUT"])
}


/// GET /admin/rbac/roles — 列出所有角色（分页）
pub async fn list_roles(
    State(state): State<AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<crate::errors::response::PaginatedData<Role>>> {
    params.sanitize();
    let all = state.rbac.list_roles().await?;
    Ok(params.paginate_in_memory(all))
}

/// POST /admin/rbac/roles — 创建角色
pub async fn create_role(
    State(state): State<AppState>,
    Json(req): Json<CreateRoleRequest>,
) -> AppResult<ApiResponse<Role>> {
    let role = state.rbac.create_role(&req).await?;
    Ok(ApiResponse::success(role))
}

/// PUT /admin/rbac/roles/:id — 更新角色
pub async fn update_role(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<Role>> {
    let role = state.rbac.update_role(&id, &req).await?;
    Ok(ApiResponse::success(role))
}

/// DELETE /admin/rbac/roles/:id — 删除角色
pub async fn delete_role(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    state.rbac.delete_role(&id).await?;
    Ok(ApiResponse::success(serde_json::json!({"deleted": true})))
}

/// GET /admin/rbac/roles/:id/permissions — 获取角色权限
pub async fn get_permissions(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> AppResult<ApiResponse<Vec<PermissionView>>> {
    let perms = state.rbac.get_permissions(&role_id).await?;
    Ok(ApiResponse::success(perms))
}

/// PUT /admin/rbac/roles/:id/permissions — 设置角色权限（替换所有）
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
