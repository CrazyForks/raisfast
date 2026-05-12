//! 用户相关处理器
//!
//! 处理当前用户资料查看/修改、密码变更、公开用户查询、用户列表等请求。

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    BatchRequestWithRole, UpdatePasswordRequest, UpdateRoleRequest, UpdateUserRequest, UserResponse,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::services::{auth, user};
use crate::utils::pagination::PaginationParams;

pub fn routes(registry: &mut crate::server::RouteRegistry) -> axum::Router<crate::AppState> {
    use axum::routing::{get, post as http_post, put};

    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        "/users/me",
        get(get_me).put(update_me),
        "system public",
        "users",
        ["GET", "PUT"]
    );
    let r = reg_route!(
        r,
        registry,
        "/users/me/password",
        put(change_password),
        "system public",
        "users",
        ["PUT"]
    );
    let r = reg_route!(
        r,
        registry,
        "/users/{id}",
        get(get_user),
        "system public",
        "users",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/users/{id}/role",
        put(update_role),
        "system public",
        "users",
        ["PUT"]
    );
    let r = reg_route!(
        r,
        registry,
        "/users",
        get(list_users),
        "system public",
        "users",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/users",
        get(admin_list_users),
        "system admin",
        "admin/users",
        ["GET"]
    );
    let r = reg_route!(
        r,
        registry,
        "/admin/users/{id}",
        get(admin_get_user)
            .put(admin_update_user)
            .delete(admin_delete_user),
        "system admin",
        "admin/users",
        ["GET", "PUT", "DELETE"]
    );
    reg_route!(
        r,
        registry,
        "/admin/users/batch",
        http_post(admin_batch_users),
        "system admin",
        "admin/users",
        ["POST"]
    )
}

/// 获取当前登录用户资料
#[utoipa::path(get, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "当前用户资料"))
)]
pub async fn get_me(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<UserResponse>> {
    let u = user::get_me(state.user_repo.as_ref(), &auth).await?;
    Ok(ApiResponse::success(u))
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
    let u = user::update_me(state.user_repo.as_ref(), &auth, req).await?;
    Ok(ApiResponse::success(u))
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
    let u = user::get_public_user(state.user_repo.as_ref(), &id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(u))
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

    let u = state
        .user_repo
        .update_role(&id, req.role, auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(UserResponse::from_user(u)?))
}

// ── Admin handlers ──

pub async fn admin_list_users(
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

pub async fn admin_get_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;
    let u = user::get_public_user(state.user_repo.as_ref(), &id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(u))
}

pub async fn admin_update_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let cmd = crate::commands::UpdateProfileCmd {
        id: {
            let user = state
                .user_repo
                .find_by_id(&id, auth.tenant_id())
                .await?
                .ok_or_else(|| crate::errors::app_error::AppError::not_found("user"))?;
            user.id
        },
        username: req.username,
        bio: req.bio,
        website: req.website,
        avatar: req.avatar,
        social_links: req.social_links,
        metadata: req.metadata,
    };
    let u = state
        .user_repo
        .update_profile(cmd, auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(UserResponse::from_user(u)?))
}

pub async fn admin_delete_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    crate::models::user::delete_by_document_id(&state.pool, &id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(()))
}

pub async fn admin_batch_users(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<BatchRequestWithRole>,
) -> AppResult<ApiResponse<crate::dto::BatchResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let mut affected = 0usize;
    for uid in &req.ids {
        match req.action.as_str() {
            "delete" => {
                if crate::models::user::delete_by_document_id(&state.pool, uid, auth.tenant_id())
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            "disable" => {
                if state
                    .user_repo
                    .update_role(uid, crate::models::user::UserRole::Reader, auth.tenant_id())
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            "enable" => {
                if state
                    .user_repo
                    .update_role(uid, crate::models::user::UserRole::Reader, auth.tenant_id())
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            "change_role" => {
                let Some(role) = req.role else {
                    continue;
                };
                if state
                    .user_repo
                    .update_role(uid, role, auth.tenant_id())
                    .await
                    .is_ok()
                {
                    affected += 1;
                }
            }
            _ => {}
        }
    }
    Ok(ApiResponse::success(crate::dto::BatchResponse::new(
        &req.action,
        affected,
    )))
}
