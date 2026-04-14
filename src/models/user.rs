//! 用户模型与数据库查询
//!
//! 定义用户相关的数据结构（完整行模型、API 响应模型、请求验证结构体）
//! 以及对 `users` 表的增删改查操作。所有密码字段使用 bcrypt 哈希存储，
//! API 响应中不会泄露 `password_hash`。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::errors::app_error::{AppError, AppResult};

/// 用户完整数据库行模型
///
/// 直接映射 `users` 表的所有字段，包含 `password_hash`。
/// 该结构体仅在内部使用，不应直接返回给前端。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct User {
    pub id: String,
    pub email: String,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 根据邮箱查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_email(pool: &crate::db::Pool, email: &str) -> AppResult<Option<User>> {
    let sql = crate::db::dialect::translate("SELECT * FROM users WHERE email = ?");
    let user = sqlx::query_as::<_, User>(&sql)
        .bind(email)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 根据用户 ID 查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<Option<User>> {
    let sql = crate::db::dialect::translate("SELECT * FROM users WHERE id = ?");
    let user = sqlx::query_as::<_, User>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 创建新用户
///
/// 自动生成 UUID v7 作为主键，默认角色为 `reader`。
/// 创建完成后重新查询并返回完整用户记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateUserCmd,
) -> AppResult<User> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO users (id, email, username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, 'reader', ?, ?)",
        id,
        cmd.email,
        cmd.username,
        cmd.password_hash,
        now,
        now,
    )
    .execute(pool)
    .await?;

    let user = find_by_id(pool, &id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch newly created user")))?;
    Ok(user)
}

/// 更新用户资料
///
/// 仅更新传入的非空字段，其余保留原值。
/// 自动更新 `updated_at` 时间戳。
pub async fn update_profile(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateProfileCmd,
) -> AppResult<User> {
    let now = Utc::now().to_rfc3339();
    let user = find_by_id(pool, &cmd.id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;

    let username = cmd.username.as_deref().unwrap_or(&user.username);
    let bio = cmd.bio.as_deref().map(|s| s.to_string()).or(user.bio);
    let website = cmd
        .website
        .as_deref()
        .map(|s| s.to_string())
        .or(user.website);
    let avatar = cmd.avatar.as_deref().map(|s| s.to_string()).or(user.avatar);

    sqlx::query!(
        "UPDATE users SET username = ?, bio = ?, website = ?, avatar = ?, updated_at = ? WHERE id = ?",
        username,
        bio,
        website,
        avatar,
        now,
        cmd.id,
    )
    .execute(pool)
    .await?;

    find_by_id(pool, &cmd.id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch updated user")))
}

/// 更新用户密码
///
/// 直接用新的哈希值覆盖 `password_hash`，并更新 `updated_at`。
pub async fn update_password(
    pool: &crate::db::Pool,
    id: &str,
    new_password_hash: &str,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query!(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?",
        new_password_hash,
        now,
        id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 分页查询所有用户
///
/// 按 `created_at` 降序排列。返回用户列表和总记录数。
pub async fn find_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<User>, i64)> {
    let offset = (page - 1) * page_size;
    let sql = crate::db::dialect::translate(
        "SELECT * FROM users ORDER BY created_at DESC LIMIT ? OFFSET ?",
    );
    let users = sqlx::query_as::<_, User>(&sql)
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;

    Ok((users, total.0))
}

/// 管理员更新用户角色
pub async fn update_role(pool: &crate::db::Pool, id: &str, role: &str) -> AppResult<User> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query!(
        "UPDATE users SET role = ?, updated_at = ? WHERE id = ?",
        role,
        now,
        id,
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("user".into()));
    }

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))
}
