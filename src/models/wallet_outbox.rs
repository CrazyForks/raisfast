use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
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
    pub id: i64,
    pub document_id: String,
    pub user_id: i64,
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
    let document_id = uuid::Uuid::now_v7().to_string();
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO wallet_outbox (document_id, user_id, currency, amount, entry_type, tx_type, transaction_no, reference_type, reference_id, metadata, tenant_id, status, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'pending', datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(cmd.user_id)
                .bind(&cmd.currency)
                .bind(cmd.amount)
                .bind(&cmd.entry_type)
                .bind(cmd.tx_type)
                .bind(&cmd.transaction_no)
                .bind(cmd.reference_type)
                .bind(&cmd.reference_id)
                .bind(&cmd.metadata)
                .bind(tid)
                .execute(&mut *tx)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO wallet_outbox (document_id, user_id, currency, amount, entry_type, tx_type, transaction_no, reference_type, reference_id, metadata, status, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'pending', datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(cmd.user_id)
                .bind(&cmd.currency)
                .bind(cmd.amount)
                .bind(&cmd.entry_type)
                .bind(cmd.tx_type)
                .bind(&cmd.transaction_no)
                .bind(cmd.reference_type)
                .bind(&cmd.reference_id)
                .bind(&cmd.metadata)
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

pub async fn fetch_pending(
    pool: &crate::db::Pool,
    limit: i64,
) -> AppResult<Vec<WalletOutbox>> {
    let sql = format!(
        "SELECT * FROM wallet_outbox WHERE status IN ('pending', 'failed') AND attempts < max_attempts ORDER BY created_at ASC LIMIT {}",
        ph(1)
    );
    sqlx::query_as::<_, WalletOutbox>(&sql)
        .bind(limit)
        .fetch_all(pool)
        .await
        .map_err(Into::into)
}

pub async fn mark_processing(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let sql = format!(
        "UPDATE wallet_outbox SET status = 'processing', updated_at = datetime('now') WHERE id = {} AND status IN ('pending', 'failed')",
        ph(1)
    );
    sqlx::query(&sql)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_completed(pool: &crate::db::Pool, id: i64) -> AppResult<()> {
    let sql = format!(
        "UPDATE wallet_outbox SET status = 'completed', updated_at = datetime('now') WHERE id = {}",
        ph(1)
    );
    sqlx::query(&sql)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_failed(pool: &crate::db::Pool, id: i64, error: &str) -> AppResult<()> {
    let sql = format!(
        "UPDATE wallet_outbox SET status = CASE WHEN attempts + 1 >= max_attempts THEN 'dead' ELSE 'failed' END, attempts = attempts + 1, last_error = {}, updated_at = datetime('now') WHERE id = {}",
        ph(1), ph(2)
    );
    sqlx::query(&sql)
        .bind(error)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
