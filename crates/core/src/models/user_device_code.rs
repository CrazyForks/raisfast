//! User device code model and database queries
//!
//! Defines the data structure for one-time authorization codes used by desktop
//! applications (IDE) to exchange for authentication tokens.

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, FromRow, Serialize, Deserialize)]
pub struct UserDeviceCode {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub code: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Timestamp,
    pub used_at: Option<Timestamp>,
    pub created_at: Timestamp,
}

pub async fn create(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    code: &str,
    access_token: &str,
    refresh_token: &str,
    expires_at: &str,
) -> AppResult<()> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(pool, "user_device_codes", [
        "id" => id,
        "user_id" => user_id,
        "code" => code,
        "access_token" => access_token,
        "refresh_token" => refresh_token,
        "expires_at" => crate::utils::tz::parse_rfc3339(expires_at)?,
        "created_at" => now
    ])?;
    Ok(())
}

pub async fn find_by_code(pool: &crate::db::Pool, code: &str) -> AppResult<Option<UserDeviceCode>> {
    let result: Option<UserDeviceCode> = raisfast_derive::crud_find!(pool, "user_device_codes", UserDeviceCode, where: ("code", code))?;
    Ok(result)
}

pub async fn mark_used(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(pool, "user_device_codes",
        bind: ["used_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

pub async fn delete_expired(pool: &crate::db::Pool) -> AppResult<u64> {
    use crate::db::{DbDriver, Driver};
    let now = crate::utils::tz::now_utc();
    let sql = format!(
        "DELETE FROM user_device_codes WHERE expires_at < {} AND used_at IS NULL",
        Driver::ph(1),
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(now)
        .execute(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "failed to delete expired device codes: {e}"
            ))
        })?;
    Ok(result.rows_affected())
}

pub async fn tx_find_by_code(
    tx: &mut crate::db::pool::DbConnection,
    code: &str,
) -> AppResult<Option<UserDeviceCode>> {
    let result: Option<UserDeviceCode> = raisfast_derive::crud_find!(&mut *tx, "user_device_codes", UserDeviceCode, where: ("code", code))?;
    Ok(result)
}

pub async fn tx_mark_used(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(&mut *tx, "user_device_codes",
        bind: ["used_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}
