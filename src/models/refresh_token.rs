//! 刷新令牌模型与数据库查询
//!
//! 定义刷新令牌（RefreshToken）的数据结构以及对 `refresh_tokens` 表的
//! 创建、查找、删除操作。刷新令牌用于在访问令牌过期后获取新的令牌对，
//! 存储在数据库中支持主动吊销。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

/// 刷新令牌完整数据库行模型
///
/// 直接映射 `refresh_tokens` 表的所有字段。
/// `expires_at` 为 ISO 8601 格式的过期时间。
#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct RefreshToken {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub token: String,
    pub expires_at: Timestamp,
    pub created_at: Timestamp,
}

/// 创建新的刷新令牌记录
///
/// 自动生成 UUID v7 作为 document_id。
pub async fn create_token(
    pool: &crate::db::Pool,
    user_id: i64,
    token: &str,
    expires_at: &str,
) -> AppResult<()> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    sqlx::query(&format!(
        "INSERT INTO refresh_tokens (document_id, user_id, token, expires_at, created_at) VALUES ({}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
    ))
    .bind(document_id)
    .bind(user_id)
    .bind(token)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 根据令牌字符串查找刷新令牌
///
/// 返回 `Ok(Some(token))` 或 `Ok(None)`（未找到时）。
pub async fn find_by_token(pool: &crate::db::Pool, token: &str) -> AppResult<Option<RefreshToken>> {
    let sql = format!("SELECT * FROM refresh_tokens WHERE token = {}", ph(1));
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
    sqlx::query(&format!(
        "DELETE FROM refresh_tokens WHERE token = {}",
        ph(1),
    ))
    .bind(token)
    .execute(pool)
    .await?;
    Ok(())
}

/// 删除指定用户的所有刷新令牌
///
/// 用于登出所有设备或修改密码后强制重新登录。
pub async fn delete_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<()> {
    sqlx::query(&format!(
        "DELETE FROM refresh_tokens WHERE user_id = {}",
        ph(1),
    ))
    .bind(user_id)
    .execute(pool)
    .await?;
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

    async fn insert_user(pool: &crate::db::Pool) -> i64 {
        let cmd = crate::commands::user::CreateUserCmd {
            username: crate::utils::id::new_document_id(),
            registered_via: "test".to_string(),
        };
        let user = crate::models::user::create(pool, &cmd, None).await.unwrap();
        user.id
    }

    #[tokio::test]
    async fn create_and_find_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token = crate::utils::id::new_document_id();
        create_token(&pool, user_id, &token, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        let found = find_by_token(&pool, &token).await.unwrap().unwrap();
        assert_eq!(found.token, token);
        assert_eq!(found.user_id, user_id);
        assert_eq!(
            found.expires_at,
            "2099-12-31T00:00:00Z".parse::<Timestamp>().unwrap()
        );
    }

    #[tokio::test]
    async fn delete_by_token() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token = crate::utils::id::new_document_id();
        create_token(&pool, user_id, &token, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        super::delete_by_token(&pool, &token).await.unwrap();
        assert!(find_by_token(&pool, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_by_user() {
        let pool = setup_pool().await;
        let user_id = insert_user(&pool).await;
        let token1 = crate::utils::id::new_document_id();
        let token2 = crate::utils::id::new_document_id();
        create_token(&pool, user_id, &token1, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        create_token(&pool, user_id, &token2, "2099-12-31T00:00:00Z")
            .await
            .unwrap();
        super::delete_by_user(&pool, user_id).await.unwrap();
        assert!(find_by_token(&pool, &token1).await.unwrap().is_none());
        assert!(find_by_token(&pool, &token2).await.unwrap().is_none());
    }
}
