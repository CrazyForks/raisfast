//! 用户模型与数据库查询
//!
//! 定义用户相关的数据结构（完整行模型、API 响应模型、请求验证结构体）
//! 以及对 `users` 表的增删改查操作。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

pub type SocialLinks = HashMap<String, String>;
pub type UserMetadata = serde_json::Value;

define_enum!(
    UserRole {
        Admin = "admin",
        Editor = "editor",
        Author = "author",
        Reader = "reader",
    }
);

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

/// 用户完整数据库行模型
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct User {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub username: String,
    pub role: UserRole,
    pub status: UserStatus,
    pub registered_via: RegisteredVia,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub display_name: Option<String>,
    pub slug: Option<String>,
    pub locale: Option<String>,
    pub social_links: Option<String>,
    pub metadata: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(User {
    required { id, document_id, username, role, status, registered_via, created_at, updated_at }
    optional { avatar, bio, website, display_name, slug, locale, social_links, metadata }
});

pub fn parse_social_links(raw: &Option<String>) -> Option<SocialLinks> {
    raw.as_ref().and_then(|s| serde_json::from_str(s).ok())
}

pub fn encode_social_links(links: &Option<SocialLinks>) -> Option<String> {
    links
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
}

pub fn parse_metadata(raw: &Option<String>) -> Option<UserMetadata> {
    raw.as_ref().and_then(|s| serde_json::from_str(s).ok())
}

pub fn encode_metadata(meta: &Option<UserMetadata>) -> Option<String> {
    meta.as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
}

/// 根据用户名查找用户
pub async fn find_by_username(pool: &crate::db::Pool, username: &str) -> AppResult<Option<User>> {
    let sql = format!("SELECT * FROM users WHERE username = {}", ph(1));
    let user = sqlx::query_as::<_, User>(&sql)
        .bind(username)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 根据 document_id 查找用户（外部接口）
pub async fn find_by_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM users WHERE document_id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, User>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let user = q.fetch_optional(pool).await?;
    Ok(user)
}

/// 根据整数主键查找用户（内部 FK 查询）
pub async fn find_by_pk(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM users WHERE id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, User>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let user = q.fetch_optional(pool).await?;
    Ok(user)
}

/// 创建新用户
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateUserCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();

    match tenant_id {
        Some(tid) => {
            let vals = (1..=5).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO users (document_id, tenant_id, username, created_at, updated_at, role, status, registered_via) VALUES ({vals}, {}, {}, {})",
                ph(6),
                ph(7),
                ph(8)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(&cmd.username)
                .bind(now)
                .bind(now)
                .bind(UserRole::Reader)
                .bind(UserStatus::Active)
                .bind(cmd.registered_via)
                .execute(pool)
                .await?;
        }
        None => {
            let vals = (1..=4).map(ph).collect::<Vec<_>>().join(", ");
            let sql = format!(
                "INSERT INTO users (document_id, username, created_at, updated_at, role, status, registered_via) VALUES ({vals}, {}, {}, {})",
                ph(5),
                ph(6),
                ph(7)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(&cmd.username)
                .bind(now)
                .bind(now)
                .bind(UserRole::Reader)
                .bind(UserStatus::Active)
                .bind(cmd.registered_via)
                .execute(pool)
                .await?;
        }
    }

    let filter = tenant_filter_ph(tenant_id, 2);
    let sql = format!("SELECT * FROM users WHERE document_id = {}{filter}", ph(1));
    let mut q = sqlx::query_as::<_, User>(&sql).bind(&document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let user = q
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch newly created user")))?;
    Ok(user)
}

/// 更新用户资料
pub async fn update_profile(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateProfileCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let user = find_by_pk(pool, cmd.id, tenant_id)
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
        .map(|m| serde_json::to_string(m).unwrap_or_default())
        .or_else(|| user.social_links.clone());
    let metadata = cmd
        .metadata
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .or_else(|| user.metadata.clone());
    let now = crate::utils::tz::now_utc();
    let filter = tenant_filter_ph(tenant_id, 9);
    let sql = format!(
        "UPDATE users SET username = {}, bio = {}, website = {}, avatar = {}, social_links = {}, metadata = {}, updated_at = {} WHERE id = {}{filter}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8)
    );
    let mut q = sqlx::query(&sql)
        .bind(username)
        .bind(bio)
        .bind(website)
        .bind(avatar)
        .bind(social_links)
        .bind(metadata)
        .bind(now)
        .bind(user.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    find_by_pk(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch updated user")))
}

/// 分页查询所有用户
pub async fn find_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<User>, i64)> {
    let offset = (page - 1) * page_size;
    let filter = tenant_filter_ph(tenant_id, 1);
    let base = usize::from(tenant_id.is_some());
    let sql_q = format!(
        "SELECT * FROM users WHERE 1=1{filter} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(base + 1),
        ph(base + 2)
    );
    let mut q = sqlx::query_as::<_, User>(&sql_q);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let users = q.bind(page_size).bind(offset).fetch_all(pool).await?;
    let count_q = format!("SELECT COUNT(*) FROM users WHERE 1=1{filter}");
    let mut q2 = sqlx::query_as::<_, (i64,)>(&count_q);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    let total = q2.fetch_one(pool).await?;
    Ok((users, total.0))
}

/// 管理员更新用户角色
pub async fn update_role(
    pool: &crate::db::Pool,
    document_id: &str,
    role: UserRole,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let now = crate::utils::tz::now_utc();
    let filter = tenant_filter_ph(tenant_id, 3);
    let sql = format!(
        "UPDATE users SET role = {}, updated_at = {} WHERE document_id = {}{filter}",
        ph(1),
        ph(2),
        ph(3)
    );
    let mut q = sqlx::query(&sql).bind(role).bind(now).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "user")?;
    find_by_id(pool, document_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))
}

pub async fn delete_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM users WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
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
        let user = create(&pool, &new_cmd("iduser"), None).await.unwrap();
        let found = super::find_by_id(&pool, &user.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, user.id);
    }
    #[tokio::test]
    async fn find_by_pk() {
        let pool = setup_pool().await;
        let user = create(&pool, &new_cmd("pkuser"), None).await.unwrap();
        let found = super::find_by_pk(&pool, user.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.document_id, user.document_id);
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
        for i in 0..5 {
            create(&pool, &new_cmd(&format!("user{i}")), None)
                .await
                .unwrap();
        }
        let (users, total) = find_all(&pool, 1, 3, None).await.unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(total, 5);
    }
    #[tokio::test]
    async fn update_role() {
        let pool = setup_pool().await;
        let user = create(&pool, &new_cmd("roleuser"), None).await.unwrap();
        assert_eq!(user.role, UserRole::Reader);
        let updated = super::update_role(&pool, &user.document_id, UserRole::Author, None)
            .await
            .unwrap();
        assert_eq!(updated.role, UserRole::Author);
    }
}
