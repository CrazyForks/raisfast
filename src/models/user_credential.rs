//! 用户凭证模型与数据库查询
//!
//! 定义用户认证凭证相关的数据结构以及对 `user_credentials` 表的增删改查操作。

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

define_enum!(
    AuthType {
        Email = "email",
        Phone = "phone",
        Oauth = "oauth",
    }
);

pub fn wrap_password_hash(hash: &str) -> String {
    serde_json::json!({"password_hash": hash}).to_string()
}

pub fn extract_password_hash(credential_data: &str) -> AppResult<String> {
    if credential_data.starts_with('{') {
        let val: serde_json::Value = serde_json::from_str(credential_data).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("invalid credential_data JSON: {e}"))
        })?;
        val.get("password_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("missing password_hash in credential_data"))
            })
    } else {
        Ok(credential_data.to_string())
    }
}

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct UserCredential {
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
    pub auth_type: AuthType,
    pub identifier: String,
    pub credential_data: String,
    pub verified: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_auth_type_and_identifier(
    pool: &crate::db::Pool,
    auth_type: AuthType,
    identifier: &str,
) -> AppResult<Option<UserCredential>> {
    let sql = format!(
        "SELECT * FROM user_credentials WHERE auth_type = {} AND identifier = {}",
        ph(1),
        ph(2)
    );
    let cred = sqlx::query_as::<_, UserCredential>(&sql)
        .bind(auth_type)
        .bind(identifier)
        .fetch_optional(pool)
        .await?;
    Ok(cred)
}

pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: i64,
) -> AppResult<Vec<UserCredential>> {
    let sql = format!("SELECT * FROM user_credentials WHERE user_id = {}", ph(1));
    sqlx::query_as::<_, UserCredential>(&sql)
        .bind(user_id)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn count_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<i64> {
    let sql = format!(
        "SELECT COUNT(*) as count FROM user_credentials WHERE user_id = {}",
        ph(1)
    );
    let row: (i64,) = sqlx::query_as(&sql).bind(user_id).fetch_one(pool).await?;
    Ok(row.0)
}

pub async fn create(
    pool: &crate::db::Pool,
    user_id: i64,
    auth_type: AuthType,
    identifier: &str,
    credential_data: &str,
    verified: bool,
) -> AppResult<UserCredential> {
    let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
    let sql = format!(
        "INSERT INTO user_credentials (document_id, user_id, auth_type, identifier, credential_data, verified, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8)
    );
    sqlx::query(&sql)
        .bind(&document_id)
        .bind(user_id)
        .bind(auth_type)
        .bind(identifier)
        .bind(credential_data)
        .bind(if verified { 1 } else { 0 })
        .bind(now)
        .bind(now)
        .execute(pool)
        .await?;

    let sql = format!(
        "SELECT * FROM user_credentials WHERE document_id = {}",
        ph(1)
    );
    let cred = sqlx::query_as::<_, UserCredential>(&sql)
        .bind(&document_id)
        .fetch_one(pool)
        .await?;
    Ok(cred)
}

pub async fn update_credential_data(
    pool: &crate::db::Pool,
    id: i64,
    credential_data: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "UPDATE user_credentials SET credential_data = {}, updated_at = {} WHERE id = {}",
        ph(1),
        ph(2),
        ph(3)
    );
    sqlx::query(&sql)
        .bind(credential_data)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_verified(pool: &crate::db::Pool, id: i64, verified: bool) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    let sql = format!(
        "UPDATE user_credentials SET verified = {}, updated_at = {} WHERE id = {}",
        ph(1),
        ph(2),
        ph(3)
    );
    sqlx::query(&sql)
        .bind(if verified { 1 } else { 0 })
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<bool> {
    let sql = format!("DELETE FROM user_credentials WHERE id = {}", ph(1));
    let result = sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<Option<UserCredential>> {
    let sql = format!("SELECT * FROM user_credentials WHERE id = {}", ph(1));
    sqlx::query_as::<_, UserCredential>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}
