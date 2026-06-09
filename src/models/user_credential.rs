//! User credential model and database queries
//!
//! Defines data structures related to user authentication credentials
//! and CRUD operations on the `user_credentials` table.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
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
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
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
    let result: Option<UserCredential> = raisfast_derive::crud_find!(pool, "user_credentials", UserCredential, where: AND(("auth_type", auth_type), ("identifier", identifier)))?;
    Ok(result)
}

pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
) -> AppResult<Vec<UserCredential>> {
    let result: Vec<UserCredential> = raisfast_derive::crud_find_all!(pool, "user_credentials", UserCredential, where: ("user_id", user_id))?;
    Ok(result)
}

pub async fn count_by_user(pool: &crate::db::Pool, user_id: SnowflakeId) -> AppResult<i64> {
    Ok(raisfast_derive::crud_count!(pool, "user_credentials", where: ("user_id", user_id))?)
}

pub async fn create(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    auth_type: AuthType,
    identifier: &str,
    credential_data: &str,
    verified: bool,
) -> AppResult<UserCredential> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(pool, "user_credentials", [
        "id" => id,
        "user_id" => user_id,
        "auth_type" => auth_type,
        "identifier" => identifier,
        "credential_data" => credential_data,
        "verified" => if verified { 1 } else { 0 },
        "created_at" => now,
        "updated_at" => now
    ])?;
    let cred = raisfast_derive::crud_find_one!(pool, "user_credentials", UserCredential, where: ("id", id))?;
    Ok(cred)
}

pub async fn update_credential_data(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    credential_data: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    raisfast_derive::crud_update!(pool, "user_credentials",
        bind: ["credential_data" => credential_data, "updated_at" => &now],
        where: ("id", id)
    )?;
    Ok(())
}

pub async fn update_verified(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    verified: bool,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    raisfast_derive::crud_update!(pool, "user_credentials",
        bind: ["verified" => if verified { 1 } else { 0 }, "updated_at" => &now],
        where: ("id", id)
    )?;
    Ok(())
}

pub async fn delete_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<bool> {
    let result: crate::db::pool::DbQueryResult =
        raisfast_derive::crud_delete!(pool, "user_credentials", where: ("id", id))?;
    Ok(result.rows_affected() > 0)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<UserCredential>> {
    let result: Option<UserCredential> =
        raisfast_derive::crud_find!(pool, "user_credentials", UserCredential, where: ("id", id))?;
    Ok(result)
}

pub async fn tx_create(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    auth_type: AuthType,
    identifier: &str,
    credential_data: &str,
    verified: bool,
) -> AppResult<()> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(&mut *tx, "user_credentials", [
        "id" => id,
        "user_id" => user_id,
        "auth_type" => auth_type,
        "identifier" => identifier,
        "credential_data" => credential_data,
        "verified" => if verified { 1i64 } else { 0i64 },
        "created_at" => now,
        "updated_at" => now
    ])?;
    Ok(())
}

pub async fn tx_find_email_cred_by_user(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
) -> AppResult<Option<(SnowflakeId, AuthType)>> {
    Ok(raisfast_derive::crud_find!(
        &mut *tx,
        "user_credentials",
        (SnowflakeId, AuthType),
        where: AND(("user_id", user_id), ("auth_type", AuthType::Email))
    )?)
}

pub async fn tx_update_credential_data(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    credential_data: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    raisfast_derive::crud_update!(&mut *tx, "user_credentials",
        bind: ["credential_data" => credential_data, "updated_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

pub async fn tx_verify_email_by_user(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
) -> AppResult<()> {
    let now = crate::utils::tz::now_str();
    raisfast_derive::crud_update!(&mut *tx, "user_credentials",
        bind: ["verified" => 1i64, "updated_at" => now],
        where: AND(("user_id", user_id), ("auth_type", AuthType::Email))
    )?;
    Ok(())
}
