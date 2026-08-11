//! User model and database queries
//!
//! Defines user-related data structures (full row model, API response model, request validation structs)
//! and CRUD operations on the `users` table.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::driver::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

pub type SocialLinks = HashMap<String, String>;
pub type UserMetadata = serde_json::Value;

/// User role.
///
/// Four built-in roles plus an open [`UserRole::Custom`] variant so that
/// user-defined RBAC roles (created via `/admin/rbac/roles`) can be assigned
/// to users and carried through the auth pipeline without being dropped.
/// Serializes as a flat string (`"admin"`, `"moderator"`, ...), never as a
/// tagged enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash, utoipa::ToSchema)]
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[cfg_attr(feature = "export-types", ts(type = "string"))]
pub enum UserRole {
    Admin,
    Editor,
    Author,
    Reader,
    /// User-defined RBAC role; the inner string is the role name from the
    /// `roles` table. A name equal to one of the built-ins always parses to
    /// the corresponding fixed variant, so there is no collision.
    Custom(String),
}

impl UserRole {
    /// The four built-in role names.
    pub const fn all_values() -> &'static [&'static str] {
        &["admin", "editor", "author", "reader"]
    }

    /// String form used for serialization, JWTs and SQL binds.
    pub fn as_str(&self) -> &str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Editor => "editor",
            UserRole::Author => "author",
            UserRole::Reader => "reader",
            UserRole::Custom(name) => name,
        }
    }
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Always succeeds: unknown strings become [`UserRole::Custom`].
impl std::str::FromStr for UserRole {
    type Err = std::convert::Infallible;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "admin" => UserRole::Admin,
            "editor" => UserRole::Editor,
            "author" => UserRole::Author,
            "reader" => UserRole::Reader,
            other => UserRole::Custom(other.to_string()),
        })
    }
}

/// Flat string serialization (not a tagged enum).
impl serde::Serialize for UserRole {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Flat string deserialization; unknown values become [`UserRole::Custom`].
impl<'de> serde::Deserialize<'de> for UserRole {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(String::deserialize(deserializer)?
            .parse()
            .unwrap_or(UserRole::Reader))
    }
}

impl From<UserRole> for String {
    fn from(val: UserRole) -> String {
        val.as_str().to_string()
    }
}

define_enum!(
    UserStatus {
        Active = "active",
        Suspended = "suspended",
        Banned = "banned",
    }
);

define_enum!(
    RegisteredVia {
        Email = "email",
        Phone = "phone",
        Oauth = "oauth",
    }
);

/// Full database row model for users
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
#[non_exhaustive]
pub struct User {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub username: String,
    pub status: UserStatus,
    pub registered_via: RegisteredVia,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub locale: Option<String>,
    pub social_links: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub fn parse_social_links(raw: &Option<serde_json::Value>) -> Option<SocialLinks> {
    raw.as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok())
}

pub fn encode_social_links(links: &Option<SocialLinks>) -> Option<serde_json::Value> {
    links.as_ref().and_then(|m| serde_json::to_value(m).ok())
}

pub fn parse_metadata(raw: &Option<serde_json::Value>) -> Option<UserMetadata> {
    raw.clone()
}

pub fn encode_metadata(meta: &Option<UserMetadata>) -> Option<serde_json::Value> {
    meta.clone()
}

/// Find user by username
pub async fn find_by_username(pool: &crate::db::Pool, username: &str) -> AppResult<Option<User>> {
    Ok(raisfast_derive::crud_find!(pool, "users", User, where: ("username", username))?)
}

/// Find user by integer primary key (internal FK lookup)
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    Ok(raisfast_derive::crud_find!(pool, "users", User, where: ("id", id), tenant: tenant_id)?)
}

/// Create a new user
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateUserCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );

    raisfast_derive::crud_insert!(
        pool,
        "users",
        [
            "id" => id,
            "username" => &cmd.username,
            "created_at" => now,
            "updated_at" => now,
            "status" => UserStatus::Active.as_str(),
            "registered_via" => cmd.registered_via.as_str()
        ],
        tenant: tenant_id
    )?;

    let user: Option<User> =
        raisfast_derive::crud_find!(pool, "users", User, where: ("id", id), tenant: tenant_id)?;
    let user = user
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch newly created user")))?;
    Ok(user)
}

/// Update user profile
pub async fn update_profile(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateProfileCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let user = find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))?;
    let username = cmd.username.as_deref().unwrap_or(&user.username);
    let bio = cmd
        .bio
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(user.bio);
    let website = cmd
        .website
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(user.website);
    let avatar = cmd
        .avatar
        .as_deref()
        .map(std::string::ToString::to_string)
        .or(user.avatar);
    let social_links = cmd
        .social_links
        .as_ref()
        .map(|m| serde_json::to_value(m).unwrap_or_default())
        .or_else(|| user.social_links.clone());
    let metadata = cmd.metadata.clone().or_else(|| user.metadata.clone());
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "users",
        bind: ["username" => username, "bio" => bio, "website" => website, "avatar" => avatar, "social_links" => social_links, "metadata" => metadata, "updated_at" => &now],
        where: ("id", user.id),
        tenant: tenant_id
    )?;
    find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch updated user")))
}

/// Paginated query for all users
pub async fn find_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<User>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, User,
        table: "users",
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: UserStatus,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(pool, "users",
        bind: ["status" => status, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "user")?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "users", where: ("id", id), tenant: tenant_id)?;
    Ok(())
}

pub async fn tx_create(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::user::CreateUserCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    let registered_via = cmd.registered_via;
    if let Some(tid) = tenant_id {
        raisfast_derive::crud_insert!(&mut *tx, "users", [
            "id" => id,
            "tenant_id" => tid,
            "username" => &cmd.username,
            "created_at" => now,
            "updated_at" => now,
            "status" => UserStatus::Active.as_str(),
            "registered_via" => registered_via.as_str()
        ])?;
    } else {
        raisfast_derive::crud_insert!(&mut *tx, "users", [
            "id" => id,
            "username" => &cmd.username,
            "created_at" => now,
            "updated_at" => now,
            "status" => UserStatus::Active.as_str(),
            "registered_via" => registered_via.as_str()
        ])?;
    }
    Ok(tx_find_by_id(tx, id, tenant_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("user not found after insert"))?)
}

pub async fn tx_find_by_id(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    let filter = crate::db::tenant::tenant_filter_ph(tenant_id, 2);
    let sql = format!(
        "SELECT * FROM users WHERE id = {}{filter}",
        crate::db::Driver::ph(1)
    );
    let mut q = sqlx::query_as::<_, User>(crate::db::safe_sql(&sql)).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(&mut *tx).await?)
}

pub async fn update_avatar(pool: &crate::db::Pool, id: SnowflakeId, avatar: &str) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "users",
        bind: ["avatar" => avatar, "updated_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }
    fn new_cmd(username: &str) -> crate::commands::user::CreateUserCmd {
        crate::commands::user::CreateUserCmd {
            username: username.to_string(),
            registered_via: RegisteredVia::Email,
        }
    }
    #[tokio::test]
    async fn find_by_id() {
        let pool = setup_pool().await;
        let user = create(&pool, &new_cmd("pkuser"), None).await.unwrap();
        let found = super::find_by_id(&pool, user.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, user.id);
    }
    #[tokio::test]
    async fn update_profile() {
        let pool = setup_pool().await;
        let user = create(&pool, &new_cmd("profuser"), None).await.unwrap();
        let updated = super::update_profile(
            &pool,
            &crate::commands::user::UpdateProfileCmd {
                id: user.id,
                username: Some("newname".to_string()),
                bio: Some("hello world".to_string()),
                website: None,
                avatar: None,
                social_links: None,
                metadata: None,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(updated.username, "newname");
    }
    #[tokio::test]
    async fn find_all_paginated() {
        let pool = setup_pool().await;
        let tenant = format!("t_{}", crate::utils::id::new_id());
        for i in 0..5 {
            create(
                &pool,
                &new_cmd(&format!("user{i}_{}", crate::utils::id::new_id())),
                Some(&tenant),
            )
            .await
            .unwrap();
        }
        let (users, total) = find_all(&pool, 1, 3, Some(&tenant)).await.unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(total, 5);
    }
}
