use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    OutboxStatus {
        Pending = "pending",
        Processing = "processing",
        Completed = "completed",
        Failed = "failed",
        Dead = "dead",
    }
);

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
pub struct WalletOutbox {
    pub id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub currency: String,
    pub amount: i64,
    pub entry_type: String,
    pub tx_type: String,
    pub transaction_no: String,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub metadata: Option<String>,
    pub tenant_id: Option<String>,
    pub status: OutboxStatus,
    pub attempts: i64,
    pub max_attempts: i64,
    pub last_error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::CreateWalletOutboxCmd,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        &mut *tx,
        "wallet_outbox",
        [
            "id" => id,
            "user_id" => cmd.user_id,
            "currency" => &cmd.currency,
            "amount" => cmd.amount,
            "entry_type" => &cmd.entry_type,
            "tx_type" => cmd.tx_type,
            "transaction_no" => &cmd.transaction_no,
            "reference_type" => cmd.reference_type,
            "reference_id" => &cmd.reference_id,
            "metadata" => &cmd.metadata,
            "status" => OutboxStatus::Pending,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    Ok(())
}

pub async fn fetch_pending(pool: &crate::db::Pool, limit: i64) -> AppResult<Vec<WalletOutbox>> {
    raisfast_derive::check_schema!(
        "wallet_outbox",
        "status",
        "attempts",
        "max_attempts",
        "created_at"
    );
    let sql = format!(
        "SELECT * FROM wallet_outbox WHERE status IN ('pending', 'failed') AND attempts < max_attempts ORDER BY created_at ASC LIMIT {}",
        Driver::ph(1)
    );
    let result: Vec<WalletOutbox> = sqlx::query_as::<crate::db::pool::Db, WalletOutbox>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| AppError::Internal(e.into()))?;
    Ok(result)
}

pub async fn mark_processing(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::check_schema!("wallet_outbox", "status", "updated_at", "id");
    let sql = format!(
        "UPDATE wallet_outbox SET status = 'processing', updated_at = {} WHERE id = {} AND status IN ('pending', 'failed')",
        crate::db::Driver::now_fn(),
        Driver::ph(1)
    );
    sqlx::query(&sql).bind(id).execute(pool).await?;
    Ok(())
}

pub async fn mark_completed(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_update!(pool, "wallet_outbox",
        raw: ["status" => "'completed'", "updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id)
    )?;
    Ok(())
}

pub async fn mark_failed(pool: &crate::db::Pool, id: SnowflakeId, error: &str) -> AppResult<()> {
    raisfast_derive::check_schema!(
        "wallet_outbox",
        "status",
        "attempts",
        "max_attempts",
        "last_error",
        "updated_at",
        "id"
    );
    let sql = format!(
        "UPDATE wallet_outbox SET status = CASE WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'failed' END, attempts = attempts + 1, last_error = {}, updated_at = {} WHERE id = {}",
        Driver::ph(1),
        crate::db::Driver::now_fn(),
        Driver::ph(2)
    );
    sqlx::query(&sql).bind(error).bind(id).execute(pool).await?;
    Ok(())
}
