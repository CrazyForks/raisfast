//! 用户资料管理服务。

use crate::commands::UpdateProfileCmd;
use crate::dto::{UpdateUserRequest, UserResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::repositories::UserRepository;

/// 获取当前用户资料。
pub async fn get_me(user_repo: &dyn UserRepository, auth: &AuthUser) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(auth.ensure_authenticated()?, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    Ok(user.into())
}

/// 更新当前用户资料（用户名、简介、网站、头像）。
pub async fn update_me(
    user_repo: &dyn UserRepository,
    auth: &AuthUser,
    req: UpdateUserRequest,
) -> AppResult<UserResponse> {
    let user = user_repo
        .update_profile(
            UpdateProfileCmd {
                id: auth.user_int_id().ok_or(AppError::Unauthorized)?,
                username: req.username,
                bio: req.bio,
                website: req.website,
                avatar: req.avatar,
            },
            auth.tenant_id(),
        )
        .await?;
    Ok(user.into())
}

/// 获取指定用户的公开资料。
pub async fn get_public_user(
    user_repo: &dyn UserRepository,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<UserResponse> {
    let user = user_repo
        .find_by_id(id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    Ok(user.into())
}

/// 分页查询用户列表。
///
/// 返回用户响应列表和总记录数。
pub async fn list_users(
    user_repo: &dyn UserRepository,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<UserResponse>, i64)> {
    let (users, total) = user_repo.find_all(page, page_size, tenant_id).await?;
    let responses = users.into_iter().map(UserResponse::from).collect();
    Ok((responses, total))
}
