//! User profile management service.

use crate::types::snowflake_id::SnowflakeId;
use std::sync::Arc;

use async_trait::async_trait;

use crate::commands::UpdateProfileCmd;
use crate::dto::{UpdateUserRequest, UserResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::user::{User, UserRole};

/// Get the current user's profile.
pub async fn get_me(pool: &crate::db::Pool, auth: &AuthUser) -> AppResult<UserResponse> {
    let uid = auth.ensure_snowflake_user_id()?;
    let user = crate::models::user::find_by_id(pool, uid, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    UserResponse::from_user_with_contacts(pool, user).await
}

/// Update the current user's profile (username, bio, website, avatar).
pub async fn update_me(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    req: UpdateUserRequest,
) -> AppResult<UserResponse> {
    let user = crate::models::user::update_profile(
        pool,
        &UpdateProfileCmd {
            id: auth.ensure_snowflake_user_id()?,
            username: req.username,
            bio: req.bio,
            website: req.website,
            avatar: req.avatar,
            social_links: req.social_links,
            metadata: req.metadata,
        },
        auth.tenant_id(),
    )
    .await?;
    UserResponse::from_user_with_contacts(pool, user).await
}

/// Get a specific user's public profile.
pub async fn get_public_user(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<UserResponse> {
    let user = crate::models::user::find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    UserResponse::from_user_with_contacts(pool, user).await
}

/// List users with pagination.
///
/// Returns a list of user responses and the total record count.
pub async fn list_users(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<UserResponse>, i64)> {
    let (users, total) = crate::models::user::find_all(pool, page, page_size, tenant_id).await?;
    let mut responses = Vec::with_capacity(users.len());
    for user in users {
        responses.push(UserResponse::from_user_with_contacts(pool, user).await?);
    }
    Ok((responses, total))
}

#[async_trait]
pub trait UserService: Send + Sync {
    async fn update_role(
        &self,
        user_id: &str,
        roles: Vec<UserRole>,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;

    async fn admin_update_user(
        &self,
        user_id: &str,
        req: &UpdateUserRequest,
        tenant_id: Option<&str>,
    ) -> AppResult<User>;

    async fn delete_user(&self, user_id: &str, tenant_id: Option<&str>) -> AppResult<()>;

    async fn batch_update_roles(
        &self,
        operations: &[(String, UserRole)],
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<bool>>;
}

pub struct UserServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl UserServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserService for UserServiceImpl {
    async fn update_role(
        &self,
        user_id: &str,
        roles: Vec<UserRole>,
        tenant_id: Option<&str>,
    ) -> AppResult<User> {
        let uid = crate::types::snowflake_id::parse_id(user_id)?;
        let user = crate::models::user::find_by_id(&self.pool, uid, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        let tid = tenant_id.unwrap_or(crate::constants::DEFAULT_TENANT);
        let role_ids = crate::models::user_role::resolve_role_ids(&self.pool, &roles).await?;
        crate::models::user_role::set_roles(&self.pool, uid, &role_ids, tid).await?;
        Ok(user)
    }

    async fn admin_update_user(
        &self,
        user_id: &str,
        req: &UpdateUserRequest,
        tenant_id: Option<&str>,
    ) -> AppResult<User> {
        let uid = crate::types::snowflake_id::parse_id(user_id)?;
        let user = crate::models::user::find_by_id(&self.pool, uid, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;

        let cmd = UpdateProfileCmd {
            id: user.id,
            username: req.username.clone(),
            bio: req.bio.clone(),
            website: req.website.clone(),
            avatar: req.avatar.clone(),
            social_links: req.social_links.clone(),
            metadata: req.metadata.clone(),
        };
        crate::models::user::update_profile(&self.pool, &cmd, tenant_id).await?;

        if let Some(ref status) = req.status {
            crate::models::user::update_status(&self.pool, uid, *status, tenant_id).await?;
        }

        if let Some(ref password) = req.password {
            crate::services::auth::ensure_not_demo_user(&self.pool, uid).await?;
            crate::services::auth::validate_password_strength(password)?;
            let hash = crate::services::auth::hash_password(password)?;
            let cred_data = crate::models::user_credential::wrap_password_hash(&hash);
            crate::models::user_credential::update_credential_data(&self.pool, uid, &cred_data)
                .await?;
        }

        let fresh = crate::models::user::find_by_id(&self.pool, uid, tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("user"))?;
        Ok(fresh)
    }

    async fn delete_user(&self, user_id: &str, tenant_id: Option<&str>) -> AppResult<()> {
        let uid = crate::types::snowflake_id::parse_id(user_id)?;
        crate::models::user::delete_by_id(&self.pool, uid, tenant_id).await
    }

    async fn batch_update_roles(
        &self,
        operations: &[(String, UserRole)],
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<bool>> {
        let mut results = Vec::with_capacity(operations.len());
        for (user_id, role) in operations {
            let ok = self
                .update_role(user_id, vec![role.clone()], tenant_id)
                .await
                .is_ok();
            results.push(ok);
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::UpdateUserRequest;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn insert_user(pool: &crate::db::Pool, username: &str) -> crate::models::user::User {
        crate::models::user::create(
            pool,
            &crate::commands::CreateUserCmd {
                username: username.to_string(),
                registered_via: crate::models::user::RegisteredVia::Email,
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
        let a = AuthUser::from_parts(Some(*user.id), crate::models::user::UserRole::Admin, None);
        let resp = super::get_me(&pool, &a).await.unwrap();
        assert_eq!(resp.username, "meuser");
    }

    #[tokio::test]
    async fn get_me_not_found() {
        let pool = setup_pool().await;
        let a = AuthUser::from_parts(
            Some(*crate::utils::id::new_snowflake_id()),
            crate::models::user::UserRole::Admin,
            None,
        );
        assert!(super::get_me(&pool, &a).await.is_err());
    }

    #[tokio::test]
    async fn update_me_changes_bio() {
        let pool = setup_pool().await;
        let user = insert_user(&pool, "upduser").await;
        let a = AuthUser::from_parts(Some(*user.id), crate::models::user::UserRole::Admin, None);
        let resp = super::update_me(
            &pool,
            &a,
            UpdateUserRequest {
                username: None,
                bio: Some("new bio".into()),
                website: None,
                avatar: None,
                social_links: None,
                metadata: None,
                status: None,
                password: None,
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
        let resp = super::get_public_user(&pool, user.id, None).await.unwrap();
        assert_eq!(resp.username, "pubuser");
    }

    #[tokio::test]
    async fn get_public_user_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::get_public_user(&pool, SnowflakeId(999999), None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_users_paginated() {
        let pool = setup_pool().await;
        let tenant = format!("t_{}", crate::utils::id::new_id());
        crate::models::user::create(
            &pool,
            &crate::commands::CreateUserCmd {
                username: format!("user_a_{}", crate::utils::id::new_id()),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            Some(&tenant),
        )
        .await
        .unwrap();
        crate::models::user::create(
            &pool,
            &crate::commands::CreateUserCmd {
                username: format!("user_b_{}", crate::utils::id::new_id()),
                registered_via: crate::models::user::RegisteredVia::Email,
            },
            Some(&tenant),
        )
        .await
        .unwrap();
        let (users, total) = super::list_users(&pool, 1, 10, Some(&tenant))
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(users.len(), 2);
    }
}
