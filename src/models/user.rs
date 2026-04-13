//! 用户模型与数据库查询
//!
//! 定义用户相关的数据结构（完整行模型、API 响应模型、请求验证结构体）
//! 以及对 `users` 表的增删改查操作。所有密码字段使用 bcrypt 哈希存储，
//! API 响应中不会泄露 `password_hash`。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// 用户完整数据库行模型
///
/// 直接映射 `users` 表的所有字段，包含 `password_hash`。
/// 该结构体仅在内部使用，不应直接返回给前端。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
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

/// 用户公开信息响应模型
///
/// 与 [`User`] 相比去除了 `password_hash` 字段，用于 API 响应。
/// 通过 `From<User>` 自动转换。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub username: String,
    pub role: String,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            email: user.email,
            username: user.username,
            role: user.role,
            avatar: user.avatar,
            bio: user.bio,
            website: user.website,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

/// 注册请求体
///
/// - `email` 必须为合法邮箱格式
/// - `username` 长度 2–50 个字符
/// - `password` 最少 8 个字符
#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128))]
    pub password: String,
}

/// 登录请求体
///
/// - `email` 必须为合法邮箱格式
/// - `password` 不能为空
#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1, max = 128))]
    pub password: String,
}

/// 刷新令牌请求体
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// 登录成功响应体
///
/// 包含访问令牌、刷新令牌、过期时间以及用户公开信息。
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
    pub user: UserResponse,
}

/// 更新用户资料请求体
///
/// 所有字段均为可选，仅更新提供的字段。
/// - `username` 如果提供，长度须在 2–50 个字符之间
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateUserRequest {
    #[validate(length(min = 2, max = 50))]
    pub username: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub avatar: Option<String>,
}

/// 修改密码请求体
///
/// - `old_password` 不能为空
/// - `new_password` 最少 8 个字符
#[derive(Debug, Deserialize, Validate)]
pub struct UpdatePasswordRequest {
    #[validate(length(min = 1, max = 128))]
    pub old_password: String,
    #[validate(length(min = 8, max = 128))]
    pub new_password: String,
}

use crate::errors::app_error::AppError;
use crate::errors::app_error::AppResult;
use validator::Validate;

/// 根据邮箱查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_email(pool: &sqlx::SqlitePool, email: &str) -> AppResult<Option<User>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = ?")
        .bind(email)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 根据用户 ID 查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(pool: &sqlx::SqlitePool, id: &str) -> AppResult<Option<User>> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
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
    pool: &sqlx::SqlitePool,
    email: &str,
    username: &str,
    password_hash: &str,
) -> AppResult<User> {
    let id = Uuid::now_v7().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, email, username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, 'reader', ?, ?)",
    )
    .bind(&id)
    .bind(email)
    .bind(username)
    .bind(password_hash)
    .bind(&now)
    .bind(&now)
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
    pool: &sqlx::SqlitePool,
    id: &str,
    username: Option<&str>,
    bio: Option<&str>,
    website: Option<&str>,
    avatar: Option<&str>,
) -> AppResult<User> {
    let now = Utc::now().to_rfc3339();
    let user = find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::NotFound("user".into()))?;

    let username = username.unwrap_or(&user.username);
    let bio = bio.map(|s| s.to_string()).or(user.bio);
    let website = website.map(|s| s.to_string()).or(user.website);
    let avatar = avatar.map(|s| s.to_string()).or(user.avatar);

    sqlx::query(
        "UPDATE users SET username = ?, bio = ?, website = ?, avatar = ?, updated_at = ? WHERE id = ?",
    )
    .bind(username)
    .bind(&bio)
    .bind(&website)
    .bind(&avatar)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("failed to fetch updated user")))
}

/// 更新用户密码
///
/// 直接用新的哈希值覆盖 `password_hash`，并更新 `updated_at`。
pub async fn update_password(
    pool: &sqlx::SqlitePool,
    id: &str,
    new_password_hash: &str,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
        .bind(new_password_hash)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 分页查询所有用户
///
/// 按 `created_at` 降序排列。返回用户列表和总记录数。
pub async fn find_all(
    pool: &sqlx::SqlitePool,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<User>, i64)> {
    let offset = (page - 1) * page_size;
    let users =
        sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY created_at DESC LIMIT ? OFFSET ?")
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
pub async fn update_role(pool: &sqlx::SqlitePool, id: &str, role: &str) -> AppResult<User> {
    let result =
        sqlx::query("UPDATE users SET role = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(role)
            .bind(id)
            .execute(pool)
            .await?;

    if result.rows_affected() == 0 {
        return Err(crate::errors::app_error::AppError::NotFound("user".into()));
    }

    find_by_id(pool, id)
        .await?
        .ok_or_else(|| crate::errors::app_error::AppError::NotFound("user".into()))
}
