use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::dialect::ph;
use crate::errors::app_error::AppResult;
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
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

#[allow(clippy::too_many_arguments)]
pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
    user_id: i64,
    currency: &str,
    amount: i64,
    entry_type: &str,
    tx_type: WalletTxType,
    transaction_no: &str,
    reference_type: Option<WalletReferenceType>,
    reference_id: Option<&str>,
    metadata: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO wallet_outbox (document_id, user_id, currency, amount, entry_type, tx_type, transaction_no, reference_type, reference_id, metadata, tenant_id, status, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, 'pending', datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(user_id)
                .bind(currency)
                .bind(amount)
                .bind(entry_type)
                .bind(tx_type)
                .bind(transaction_no)
                .bind(reference_type)
                .bind(reference_id)
                .bind(metadata)
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
                .bind(document_id)
                .bind(user_id)
                .bind(currency)
                .bind(amount)
                .bind(entry_type)
                .bind(tx_type)
                .bind(transaction_no)
                .bind(reference_type)
                .bind(reference_id)
                .bind(metadata)
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
