use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::wallet_outbox;
use crate::models::wallet_transaction::WalletTxType;
use crate::worker::{Job, JobHandler};

pub struct ProcessWalletOutboxHandler {
    pool: Pool,
    _config: Arc<AppConfig>,
}

impl ProcessWalletOutboxHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self {
            pool,
            _config: config,
        }
    }
}

#[async_trait::async_trait]
impl JobHandler for ProcessWalletOutboxHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::ProcessWalletOutbox = job else {
            return Ok(());
        };

        let entries = wallet_outbox::fetch_pending(&self.pool, 100).await?;

        if entries.is_empty() {
            return Ok(());
        }

        let mut processed = 0u64;
        let mut failed = 0u64;

        for entry in &entries {
            if let Err(e) = wallet_outbox::mark_processing(&self.pool, entry.id).await {
                tracing::warn!(
                    "[wallet_outbox] CAS failed for entry {}, skipping: {e}",
                    entry.id.to_string()
                );
                continue;
            }

            let auth = crate::middleware::auth::AuthUser::from_parts(
                Some(entry.user_id.0),
                crate::models::user::UserRole::Reader,
                entry.tenant_id.clone(),
            );
            let result = match entry.entry_type.as_str() {
                "credit" => {
                    crate::services::wallet::credit_wallet(
                        &self.pool,
                        &auth,
                        &entry.currency,
                        entry.amount,
                        parse_tx_type(&entry.tx_type),
                        &entry.transaction_no,
                        entry
                            .reference_type
                            .as_deref()
                            .and_then(parse_reference_type),
                        entry.reference_id.as_deref(),
                        entry.metadata.as_deref(),
                    )
                    .await
                }
                "debit" => {
                    crate::services::wallet::debit_wallet(
                        &self.pool,
                        &auth,
                        &entry.currency,
                        entry.amount,
                        parse_tx_type(&entry.tx_type),
                        &entry.transaction_no,
                        entry
                            .reference_type
                            .as_deref()
                            .and_then(parse_reference_type),
                        entry.reference_id.as_deref(),
                        entry.metadata.as_deref(),
                    )
                    .await
                }
                other => Err(crate::errors::app_error::AppError::Internal(
                    anyhow::anyhow!("unknown entry_type: {other}"),
                )),
            };

            match result {
                Ok(_) => {
                    wallet_outbox::mark_completed(&self.pool, entry.id).await?;
                    processed += 1;
                }
                Err(e) => {
                    let err_str = format!("{e}");
                    tracing::warn!(
                        "[wallet_outbox] failed to process entry {}: {err_str}",
                        entry.id.to_string()
                    );
                    wallet_outbox::mark_failed(&self.pool, entry.id, &err_str).await?;
                    failed += 1;
                }
            }
        }

        if processed > 0 || failed > 0 {
            tracing::info!("[wallet_outbox] processed {processed}, failed {failed}");
        }

        Ok(())
    }
}

fn parse_tx_type(s: &str) -> WalletTxType {
    match s {
        "recharge" => WalletTxType::Recharge,
        "payment" => WalletTxType::Payment,
        "refund" => WalletTxType::Refund,
        "transfer_out" => WalletTxType::TransferOut,
        "transfer_in" => WalletTxType::TransferIn,
        _ => WalletTxType::Recharge,
    }
}

fn parse_reference_type(s: &str) -> Option<crate::models::wallet_transaction::WalletReferenceType> {
    use crate::models::wallet_transaction::WalletReferenceType;
    match s {
        "admin" => Some(WalletReferenceType::Admin),
        "checkin" => Some(WalletReferenceType::Checkin),
        "order_reward" => Some(WalletReferenceType::OrderReward),
        "api_usage" => Some(WalletReferenceType::ApiUsage),
        "points_mall" => Some(WalletReferenceType::PointsMall),
        "order" => Some(WalletReferenceType::Order),
        "expiry" => Some(WalletReferenceType::Expiry),
        "payment" => Some(WalletReferenceType::Payment),
        "payment_refund" => Some(WalletReferenceType::PaymentRefund),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = ProcessWalletOutboxHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn handles_empty_outbox() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = ProcessWalletOutboxHandler::new(pool, config);
        let job = Job::ProcessWalletOutbox;
        assert!(handler.handle(&job).await.is_ok());
    }
}
