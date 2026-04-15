//! 刷新令牌模型与数据库查询
//!
//! 定义刷新令牌（RefreshToken）的数据结构以及对 `refresh_tokens` 表的
//! 创建、查找、删除操作。刷新令牌用于在访问令牌过期后获取新的令牌对，
//! 存储在数据库中支持主动吊销。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;

/// 刷新令牌完整数据库行模型
///
/// 直接映射 `refresh_tokens` 表的所有字段。
/// `expires_at` 为 ISO 8601 格式的过期时间。
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: String,
    pub user_id: String,
    pub token: String,
    pub expires_at: String,
    pub created_at: String,
}

/// 创建新的刷新令牌记录
///
/// 自动生成 UUID v7 作为主键。
pub async fn create_token(
    pool: &crate::db::Pool,
    user_id: &str,
    token: &str,
    expires_at: &str,
) -> AppResult<()> {
    let (id, now) = crate::utils::id::new_id_and_timestamp();
    sqlx::query!(
        "INSERT INTO refresh_tokens (id, user_id, token, expires_at, created_at) VALUES (?, ?, ?, ?, ?)",
        id,
        user_id,
        token,
        expires_at,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// 根据令牌字符串查找刷新令牌
///
/// 返回 `Ok(Some(token))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_token(pool: &crate::db::Pool, token: &str) -> AppResult<Option<RefreshToken>> {
    let sql = crate::db::dialect::translate("SELECT * FROM refresh_tokens WHERE token = ?");
    let row = sqlx::query_as::<_, RefreshToken>(&sql)
        .bind(token)
        .fetch_optional(pool)
        .await?;
    Ok(row)
}

/// 根据令牌字符串删除刷新令牌
///
/// 用于登出时吊销指定的刷新令牌。
pub async fn delete_by_token(pool: &crate::db::Pool, token: &str) -> AppResult<()> {
    sqlx::query!("DELETE FROM refresh_tokens WHERE token = ?", token)
        .execute(pool)
        .await?;
    Ok(())
}

/// 删除指定用户的所有刷新令牌
///
/// 用于登出所有设备或修改密码后强制重新登录。
pub async fn delete_by_user(pool: &crate::db::Pool, user_id: &str) -> AppResult<()> {
    sqlx::query!("DELETE FROM refresh_tokens WHERE user_id = ?", user_id)
        .execute(pool)
        .await?;
    Ok(())
}
