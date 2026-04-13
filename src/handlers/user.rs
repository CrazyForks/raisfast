//! 用户相关处理器
//!
//! 处理当前用户资料查看/修改、密码变更、公开用户查询、用户列表等请求。

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::{AdminUser, AuthUser};
use crate::models::user::{UpdatePasswordRequest, UpdateUserRequest, UserResponse};
use crate::services::auth;
use crate::utils::pagination::PaginationParams;

/// 获取当前登录用户资料
///
/// - **方法/路径：** `GET /api/users/me`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 返回当前认证用户的公开信息。
/// - **返回：** `ApiResponse<UserResponse>`
pub async fn get_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_me(&state.pool, &auth_user.user_id).await?;
    Ok(ApiResponse::success(user))
}

/// 更新当前用户资料
///
/// - **方法/路径：** `PUT /api/users/me`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 更新当前用户的昵称、简介、网站、头像等信息。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<UserResponse>`
pub async fn update_me(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;
    let user = auth::update_me(&state.pool, &auth_user.user_id, req).await?;
    Ok(ApiResponse::success(user))
}

/// 修改当前用户密码
///
/// - **方法/路径：** `PUT /api/users/me/password`
/// - **认证：** 需要登录（`AuthUser`）
/// - **说明：** 验证旧密码后更新为新密码，同时清除所有刷新令牌以强制重新登录。
/// - **验证：** 通过 `validation::validate()` 校验请求体，验证错误消息通过 i18n 翻译。
/// - **返回：** `ApiResponse<()>`
pub async fn change_password(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    auth::change_password(&state.pool, &auth_user.user_id, req).await?;
    Ok(ApiResponse::success(()))
}

/// 获取指定用户的公开资料
///
/// - **方法/路径：** `GET /api/users/:id`
/// - **认证：** 无需认证
/// - **说明：** 根据用户 ID 返回该用户的公开信息。
/// - **返回：** `ApiResponse<UserResponse>`
pub async fn get_user(
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    let user = auth::get_public_user(&state.pool, &id).await?;
    Ok(ApiResponse::success(user))
}

/// 获取用户列表（管理员）
///
/// - **方法/路径：** `GET /api/admin/users`
/// - **认证：** 需要管理员权限（`AdminUser`）
/// - **说明：** 分页查询所有用户，支持自定义 `page` 和 `page_size` 参数。
/// - **返回：** `ApiResponse<PaginatedData<UserResponse>>`
pub async fn list_users(
    State(state): State<crate::AppState>,
    _admin: AdminUser,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    params.sanitize();
    let (users, total) = auth::list_users(&state.pool, params.page, params.page_size).await?;
    Ok(ApiResponse::success(PaginatedData {
        items: users,
        total,
        page: params.page,
        page_size: params.page_size,
    }))
}

/// 管理员更新用户角色
///
/// - **方法/路径：** `PUT /api/users/:id/role`
/// - **认证：** 需要管理员权限
/// - **请求体：** `{ "role": "reader" | "author" | "admin" }`
pub async fn update_role(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> AppResult<ApiResponse<UserResponse>> {
    let role = body["role"]
        .as_str()
        .ok_or_else(|| AppError::BadRequest("role is required".into()))?;

    if !["reader", "author", "admin"].contains(&role) {
        return Err(AppError::BadRequest(
            "role must be reader, author, or admin".into(),
        ));
    }

    let user = crate::models::user::update_role(&state.pool, &id, role).await?;
    Ok(ApiResponse::success(user.into()))
}
