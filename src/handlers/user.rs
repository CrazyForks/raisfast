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
use crate::services::auth;
use crate::utils::pagination::PaginationParams;

/// 获取当前登录用户资料
pub async fn get_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_me(state.user_repo.as_ref(), &auth_user.user_id).await?;
    Ok(ApiResponse::success(user))
}

/// 更新当前用户资料
pub async fn update_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;
    let user = auth::update_me(state.user_repo.as_ref(), &auth_user.user_id, req).await?;
    Ok(ApiResponse::success(user))
}

/// 修改当前用户密码
pub async fn change_password(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    auth::change_password(
        state.user_repo.as_ref(),
        &state.pool,
        &auth_user.user_id,
        req,
    )
    .await?;
    Ok(ApiResponse::success(()))
}

/// 获取指定用户的公开资料
pub async fn get_user(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_public_user(state.user_repo.as_ref(), &id).await?;
    Ok(ApiResponse::success(user))
}

/// 获取用户列表（管理员）
pub async fn list_users(
    State(state): State<crate::AppState>,
    _admin: AdminUser,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    params.sanitize();
    let (users, total) =
        auth::list_users(state.user_repo.as_ref(), params.page, params.page_size).await?;
    Ok(ApiResponse::success(PaginatedData {
        items: users,
        total,
        page: params.page,
        page_size: params.page_size,
    }))
}

/// 管理员更新用户角色
pub async fn update_role(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;

    let user = state.user_repo.update_role(&id, &req.role).await?;
    Ok(ApiResponse::success(user.into()))
}
