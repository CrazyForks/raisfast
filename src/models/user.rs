//! 用户模型与数据库查询
//!
//! 定义用户相关的数据结构（完整行模型、API 响应模型、请求验证结构体）
//! 以及对 `users` 表的增删改查操作。所有密码字段使用 bcrypt 哈希存储，
//! API 响应中不会泄露 `password_hash`。

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::tenant::{resolve_tenant, tenant_filter};
use crate::errors::app_error::{AppError, AppResult};

/// 用户完整数据库行模型
///
/// 直接映射 `users` 表的所有字段，包含 `password_hash`。
/// 该结构体仅在内部使用，不应直接返回给前端。
#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub struct User {
    pub id: String,
    pub tenant_id: String,
    pub email: String,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub phone: Option<String>,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub email_verified: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// 根据邮箱查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
/// `tenant_id` 为 `None` 时（超管）不过滤租户。
pub async fn find_by_email(
    pool: &crate::db::Pool,
    email: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    let sql_str = format!(
        "SELECT * FROM users WHERE email = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, User>(&sql).bind(email);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let user = q.fetch_optional(pool).await?;
    Ok(user)
}

/// 根据用户名查找用户
pub async fn find_by_username(pool: &crate::db::Pool, username: &str) -> AppResult<Option<User>> {
    let sql = crate::db::dialect::translate("SELECT * FROM users WHERE username = ?");
    let user = sqlx::query_as::<_, User>(&sql)
        .bind(username)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 根据手机号查找用户
pub async fn find_by_phone(pool: &crate::db::Pool, phone: &str) -> AppResult<Option<User>> {
    let sql = crate::db::dialect::translate("SELECT * FROM users WHERE phone = ?");
    let user = sqlx::query_as::<_, User>(&sql)
        .bind(phone)
        .fetch_optional(pool)
        .await?;
    Ok(user)
}

/// 根据用户 ID 查找用户
///
/// 返回 `Ok(Some(user))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<User>> {
    let sql_str = format!(
        "SELECT * FROM users WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql_str);
    let mut q = sqlx::query_as::<_, User>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let user = q.fetch_optional(pool).await?;
    Ok(user)
}

/// 创建新用户
///
/// 自动生成 UUID v7 作为主键，默认角色为 `reader`。
/// 创建完成后重新查询并返回完整用户记录。
pub async fn create(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateUserCmd,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();
    let tid = resolve_tenant(tenant_id);

    let sql = crate::db::dialect::translate(
        "INSERT INTO users (id, tenant_id, email, username, password_hash, role, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'reader', ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&id)
        .bind(tid)
        .bind(&cmd.email)
        .bind(&cmd.username)
        .bind(&cmd.password_hash)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

    let user = find_by_id(pool, &id, tenant_id)
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
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let now = Utc::now().to_rfc3339();
    let user = find_by_id(pool, &cmd.id, tenant_id)
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

    let sql = format!(
        "UPDATE users SET username = ?, bio = ?, website = ?, avatar = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql)
        .bind(username)
        .bind(bio)
        .bind(website)
        .bind(avatar)
        .bind(now)
        .bind(&cmd.id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;

    find_by_id(pool, &cmd.id, tenant_id)
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
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(new_password_hash).bind(now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

/// 绑定手机号
pub async fn update_phone(
    pool: &crate::db::Pool,
    id: &str,
    phone: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE users SET phone = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(phone).bind(now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

/// 分页查询所有用户
///
/// 按 `created_at` 降序排列。返回用户列表和总记录数。
pub async fn find_all(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: Option<&str>,
) -> AppResult<(Vec<User>, i64)> {
    let offset = (page - 1) * page_size;
    let filter = tenant_filter(tenant_id);

    let sql_q =
        format!("SELECT * FROM users WHERE 1=1{filter} ORDER BY created_at DESC LIMIT ? OFFSET ?");
    let sql = crate::db::dialect::translate(&sql_q);
    let mut q = sqlx::query_as::<_, User>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let users = q.bind(page_size).bind(offset).fetch_all(pool).await?;

    let count_q = format!("SELECT COUNT(*) FROM users WHERE 1=1{filter}");
    let sql = crate::db::dialect::translate(&count_q);
    let mut q = sqlx::query_as::<_, (i64,)>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let total = q.fetch_one(pool).await?;

    Ok((users, total.0))
}

/// 管理员更新用户角色
pub async fn update_role(
    pool: &crate::db::Pool,
    id: &str,
    role: &str,
    tenant_id: Option<&str>,
) -> AppResult<User> {
    let now = Utc::now().to_rfc3339();
    let sql = format!(
        "UPDATE users SET role = ?, updated_at = ? WHERE id = ?{}",
        tenant_filter(tenant_id)
    );
    let sql = crate::db::dialect::translate(&sql);
    let mut q = sqlx::query(&sql).bind(role).bind(now).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;

    AppError::expect_affected(&result, "user")?;

    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("user"))
}
