//! User credential model and database queries
//!
//! Defines data structures related to user authentication credentials
//! and CRUD operations on the `user_credentials` table.

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
    check_schema!("user_credentials", "auth_type", "identifier");
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
    crud_find_all!(pool, "user_credentials" => UserCredential, "user_id" => user_id)
        .map_err(Into::into)
}

pub async fn count_by_user(pool: &crate::db::Pool, user_id: i64) -> AppResult<i64> {
    check_schema!("user_credentials", "user_id");
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
    crud_insert!(pool, "user_credentials", [
        "document_id" => &document_id,
        "user_id" => user_id,
        "auth_type" => auth_type,
        "identifier" => identifier,
        "credential_data" => credential_data,
        "verified" => if verified { 1 } else { 0 },
        "created_at" => now,
        "updated_at" => now
    ])?;
    let cred =
        crud_find_one!(pool, "user_credentials" => UserCredential, "document_id" => &document_id)?;
    Ok(cred)
}

pub async fn update_credential_data(
    pool: &crate::db::Pool,
    id: i64,
    credential_data: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    crud_update!(pool, "user_credentials",
        bind: ["credential_data" => credential_data, "updated_at" => &now],
        where: "id" => id
    )?;
    Ok(())
}

pub async fn update_verified(pool: &crate::db::Pool, id: i64, verified: bool) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    crud_update!(pool, "user_credentials",
        bind: ["verified" => if verified { 1 } else { 0 }, "updated_at" => &now],
        where: "id" => id
    )?;
    Ok(())
}

pub async fn delete_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<bool> {
    let result = crud_delete!(pool, "user_credentials", "id" => id)?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_id(pool: &crate::db::Pool, id: i64) -> AppResult<Option<UserCredential>> {
    crud_find!(pool, "user_credentials" => UserCredential, "id" => id).map_err(Into::into)
}
