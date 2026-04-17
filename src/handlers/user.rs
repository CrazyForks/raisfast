//! 用户相关处理器
//!
//! 处理当前用户资料查看/修改、密码变更、公开用户查询、用户列表等请求。

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::handlers::dto::{
    UpdatePasswordRequest, UpdateRoleRequest, UpdateUserRequest, UserResponse,
};
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::middleware::tenant::ResolvedTenant;
use crate::services::auth;
use crate::utils::pagination::PaginationParams;

/// 获取当前登录用户资料
#[utoipa::path(get, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户资料"))
)]
pub async fn get_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_me(
        state.user_repo.as_ref(),
        &auth_user.user_id,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(user))
}

/// 更新当前用户资料
#[utoipa::path(put, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdateUserRequest,
    responses((status = 200, description = "用户资料已更新"))
)]
pub async fn update_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;
    let user = auth::update_me(
        state.user_repo.as_ref(),
        &auth_user.user_id,
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(user))
}

/// 修改当前用户密码
#[utoipa::path(put, path = "/users/me/password", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdatePasswordRequest,
    responses((status = 200, description = "密码已修改"))
)]
pub async fn change_password(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    auth::change_password(
        state.user_repo.as_ref(),
        &state.pool,
        &auth_user.user_id,
        req,
        tenant.as_str(),
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 获取指定用户的公开资料
#[utoipa::path(get, path = "/users/{id}", tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    responses((status = 200, description = "用户公开资料"))
)]
pub async fn get_user(
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_public_user(state.user_repo.as_ref(), &id, tenant.as_str()).await?;
    Ok(ApiResponse::success(user))
}

/// 获取用户列表（管理员）
#[utoipa::path(get, path = "/users", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "用户列表"))
)]
pub async fn list_users(
    State(state): State<crate::AppState>,
    _admin: AdminUser,
    tenant: ResolvedTenant,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    params.sanitize();
    let (users, total) = auth::list_users(
        state.user_repo.as_ref(),
        params.page,
        params.page_size,
        tenant.as_str(),
    )
    .await?;
    Ok(params.paginate(users, total))
}

/// 管理员更新用户角色
#[utoipa::path(put, path = "/users/{id}/role", tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    request_body = UpdateRoleRequest,
    responses((status = 200, description = "用户角色已更新"))
)]
pub async fn update_role(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    tenant: ResolvedTenant,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;

    let user = state
        .user_repo
        .update_role(&id, &req.role, tenant.as_str())
        .await?;
    Ok(ApiResponse::success(user.into()))
}
