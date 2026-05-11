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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::UpdateUserRequest;
    use crate::repositories::sqlx_user::SqlxUserRepository;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn auth(doc_id: &str) -> AuthUser {
        AuthUser::from_parts(Some(doc_id.to_string()), Some(1), "admin".to_string(), None)
    }

    async fn insert_user(pool: &crate::db::Pool, username: &str) -> crate::models::user::User {
        let repo = SqlxUserRepository::new(pool.clone());
        repo.create(
            crate::commands::CreateUserCmd {
                username: username.to_string(),
                registered_via: "email".to_string(),
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn get_me_returns_user() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "meuser").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let a = AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            "admin".to_string(),
            None,
        );
        let resp = super::get_me(&repo, &a).await.unwrap();
        assert_eq!(resp.username, "meuser");
    }

    #[tokio::test]
    async fn get_me_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxUserRepository::new(pool.clone());
        let a = auth("ghost");
        assert!(super::get_me(&repo, &a).await.is_err());
    }

    #[tokio::test]
    async fn update_me_changes_bio() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "upduser").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let a = AuthUser::from_parts(
            Some(user.document_id.clone()),
            Some(user.id),
            "admin".to_string(),
            None,
        );
        let resp = super::update_me(
            &repo,
            &a,
            UpdateUserRequest {
                username: None,
                bio: Some("new bio".into()),
                website: None,
                avatar: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.bio, Some("new bio".to_string()));
    }

    #[tokio::test]
    async fn get_public_user_found() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "pubuser").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let resp = super::get_public_user(&repo, &user.document_id, None)
            .await
            .unwrap();
        assert_eq!(resp.username, "pubuser");
    }

    #[tokio::test]
    async fn get_public_user_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxUserRepository::new(pool.clone());
        assert!(
            super::get_public_user(&repo, "missing", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_users_paginated() {
        let pool = setup_pool().await;
        insert_user(&pool, "user_a").await;
        insert_user(&pool, "user_b").await;
        let repo = SqlxUserRepository::new(pool.clone());
        let (users, total) = super::list_users(&repo, 1, 10, None).await.unwrap();
        assert_eq!(total, 2);
        assert_eq!(users.len(), 2);
    }
}
