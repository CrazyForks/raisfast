//! 用户相关处理器
//!
//! 处理当前用户资料查看/修改、密码变更、公开用户查询、用户列表等请求。

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::dto::{
    UpdatePasswordRequest, UpdateRoleRequest, UpdateUserRequest, UserResponse,
};
use crate::middleware::auth::AuthUser;
use crate::services::{auth, user};
use crate::utils::pagination::PaginationParams;

/// 获取当前登录用户资料
#[utoipa::path(get, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户资料"))
)]
pub async fn get_me(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = user::get_me(state.user_repo.as_ref(), &auth).await?;
    Ok(ApiResponse::success(user))
}

/// 更新当前用户资料
#[utoipa::path(put, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdateUserRequest,
    responses((status = 200, description = "用户资料已更新"))
)]
pub async fn update_me(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;
    let user = user::update_me(state.user_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(user))
}

/// 修改当前用户密码
#[utoipa::path(put, path = "/users/me/password", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdatePasswordRequest,
    responses((status = 200, description = "密码已修改"))
)]
pub async fn change_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    auth::change_password(state.user_repo.as_ref(), &state.pool, &auth, req).await?;
    Ok(ApiResponse::success(()))
}

/// 获取指定用户的公开资料
#[utoipa::path(get, path = "/users/{id}", tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    responses((status = 200, description = "用户公开资料"))
)]
pub async fn get_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = user::get_public_user(state.user_repo.as_ref(), &id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(user))
}

/// 获取用户列表（管理员）
#[utoipa::path(get, path = "/users", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "用户列表"))
)]
pub async fn list_users(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (users, total) = user::list_users(
        state.user_repo.as_ref(),
        params.page,
        params.page_size,
        auth.tenant_id(),
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
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;

    let user = state
        .user_repo
        .update_role(&id, &req.role, auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(user.into()))
}
