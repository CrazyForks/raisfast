//! User-related handlers
//!
//! Handles current user profile viewing/editing, password changes, public user queries, user listing, etc.

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::dto::{
    AdminCreateUserRequest, BatchRequestWithRole, UpdatePasswordRequest, UpdateRoleRequest,
    UpdateUserRequest, UserResponse,
};
use crate::errors::app_error::AppResult;
use crate::errors::response::{ApiResponse, PaginatedData};
use crate::errors::validation;
use crate::middleware::auth::AuthUser;
use crate::models::user::UserRole;
use crate::services::{auth, user};
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
        "/users/me",
        get,
        get_me,
        "system authed",
        "users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users/me",
        put,
        update_me,
        "system authed",
        "users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users/me/password",
        put,
        change_password,
        "system authed",
        "users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users/{id}",
        get,
        get_user,
        "system authed",
        "users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users/{id}/role",
        put,
        update_role,
        "system admin",
        "admin/users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/users",
        get,
        list_users,
        "system admin",
        "users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/users",
        post,
        admin_create_user,
        "system admin",
        "admin/users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/users",
        get,
        admin_list_users,
        "system admin",
        "admin/users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/users/{id}",
        get,
        admin_get_user,
        "system admin",
        "admin/users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/users/{id}",
        put,
        admin_update_user,
        "system admin",
        "admin/users"
    );
    let r = reg_route!(
        r,
        registry,
        restful,
        "/admin/users/{id}",
        delete,
        admin_delete_user,
        "system admin",
        "admin/users"
    );
    reg_route!(
        r,
        registry,
        restful,
        "/admin/users/batch",
        post,
        admin_batch_users,
        "system admin",
        "admin/users"
    )
}

/// Get the current logged-in user's profile
#[utoipa::path(get, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "Current user profile"))
)]
pub async fn get_me(
    auth: AuthUser,
    State(state): State<crate::AppState>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_authenticated()?;
    let u = user::get_me(&state.pool, &auth).await?;
    Ok(ApiResponse::success(u))
}

/// Update the current user's profile
#[utoipa::path(put, path = "/users/me", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdateUserRequest,
    responses((status = 200, description = "User profile updated"))
)]
pub async fn update_me(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<UpdateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    validation::validate(&req)?;
    auth.ensure_authenticated()?;
    let u = user::update_me(&state.pool, &auth, req).await?;
    Ok(ApiResponse::success(u))
}

/// Change the current user's password
#[utoipa::path(put, path = "/users/me/password", tag = "users",
    security(("bearer_auth" = [])),
    request_body = UpdatePasswordRequest,
    responses((status = 200, description = "Password changed"))
)]
pub async fn change_password(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<UpdatePasswordRequest>,
) -> AppResult<ApiResponse<()>> {
    validation::validate(&req)?;
    auth.ensure_authenticated()?;
    auth::change_password(&state.pool, &auth, req).await?;
    Ok(ApiResponse::success(()))
}

/// Get a specific user's public profile
#[utoipa::path(get, path = "/users/{id}", tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    responses((status = 200, description = "User public profile"))
)]
pub async fn get_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_authenticated()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let u = user::get_public_user(&state.pool, id, auth.tenant_id()).await?;
    Ok(ApiResponse::success(u))
}

/// Get user list (admin)
#[utoipa::path(get, path = "/users", tag = "users",
    security(("bearer_auth" = [])),
    responses((status = 200, description = "User list"))
)]
pub async fn list_users(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (users, total) =
        user::list_users(&state.pool, params.page, params.page_size, auth.tenant_id()).await?;
    Ok(params.paginate(users, total))
}

/// Admin updates user role
#[utoipa::path(put, path = "/users/{id}/role", tag = "users",
    security(("bearer_auth" = [])),
    params(("id" = String, Path, description = "User ID")),
    request_body = UpdateRoleRequest,
    responses((status = 200, description = "User role updated"))
)]
pub async fn update_role(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;

    let u = state
        .user_service
        .update_role(&id, req.role, auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(
        UserResponse::from_user_with_contacts(&state.pool, u).await?,
    ))
}

// ── Admin handlers ──

pub async fn admin_create_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Json(req): Json<AdminCreateUserRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;
    validation::validate(&req)?;
    let user = crate::services::auth::admin_create_user(
        &state.aspect_engine,
        req,
        auth.tenant_id(),
        &state.pool,
    )
    .await?;
    Ok(ApiResponse::success(user))
}

pub async fn admin_list_users(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Query(mut params): Query<PaginationParams>,
) -> AppResult<ApiResponse<PaginatedData<UserResponse>>> {
    auth.ensure_admin()?;
    params.sanitize();
    let (users, total) =
        user::list_users(&state.pool, params.page, params.page_size, auth.tenant_id()).await?;
    Ok(params.paginate(users, total))
}

pub async fn admin_get_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<UserResponse>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let u = user::get_public_user(&state.pool, id, auth.tenant_id()).await?;
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
    let u = state
        .user_service
        .admin_update_user(&id, &req, auth.tenant_id())
        .await?;
    Ok(ApiResponse::success(
        UserResponse::from_user_with_contacts(&state.pool, u).await?,
    ))
}

pub async fn admin_delete_user(
    auth: AuthUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<()>> {
    auth.ensure_admin()?;
    state
        .user_service
        .delete_user(&id, auth.tenant_id())
        .await?;
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
            "delete"
                if state
                    .user_service
                    .delete_user(uid, auth.tenant_id())
                    .await
                    .is_ok() =>
            {
                affected += 1;
            }
            "disable" | "enable"
                if state
                    .user_service
                    .update_role(uid, UserRole::Reader, auth.tenant_id())
                    .await
                    .is_ok() =>
            {
                affected += 1;
            }
            "change_role" => {
                let Some(role) = req.role else {
                    continue;
                };
                if state
                    .user_service
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
