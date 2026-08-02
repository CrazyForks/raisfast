use std::sync::Arc;

use crate::commands::CreateWalletOutboxCmd;
use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::payment_channel;
use crate::models::payment_order::{self, PaymentStatus};
use crate::models::wallet_transaction::{WalletReferenceType, WalletTxType};
use crate::worker::{Job, JobHandler};

pub struct RetryPaymentCallbackHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl RetryPaymentCallbackHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for RetryPaymentCallbackHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let order_id = match job {
            Job::RetryPaymentCallback { payment_order_id } => *payment_order_id,
            _ => return Ok(()),
        };

        let order = payment_order::find_by_id(&self.pool, order_id, None)
            .await?
            .ok_or_else(|| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "payment order {order_id} not found"
                ))
            })?;

        if order.status != PaymentStatus::Pending {
            tracing::info!(
                "[retry_payment_callback] order {} status is {:?}, skipping",
                order.id.to_string(),
                order.status
            );
            return Ok(());
        }

        let Some(ref provider_order_id) = order.provider_order_id else {
            tracing::warn!(
                "[retry_payment_callback] order {} has no provider_order_id, expiring",
                order.id.to_string()
            );
            crate::in_transaction!(&self.pool, tx, {
                crate::models::payment_order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    PaymentStatus::Expired,
                    Some("expired_at"),
                    PaymentStatus::Pending,
                )
                .await?;
                Ok(())
            })?;
            return Ok(());
        };

        let key = get_encrypt_key(&self.config)?;
        let provider = match crate::payment::providers::get_provider(&order.provider, &key) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "[retry_payment_callback] provider '{}' not available for order {}: {e}",
                    order.provider,
                    order.id.to_string()
                );
                return Ok(());
            }
        };

        let channel = payment_channel::find_by_id(&self.pool, order.channel_id, None)
            .await?
            .ok_or_else(|| {
                crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                    "payment channel {} not found",
                    order.channel_id
                ))
            })?;

        let status = provider.query(&channel, provider_order_id).await?;

        match status.status {
            PaymentStatus::Paid => {
                tracing::info!(
                    "[retry_payment_callback] order {} confirmed paid via provider query",
                    order.id.to_string()
                );
                crate::in_transaction!(&self.pool, tx, {
                    let rows = crate::models::payment_order::tx_update_status_cas(
                        &mut tx,
                        order.id,
                        PaymentStatus::Paid,
                        Some("paid_at"),
                        PaymentStatus::Pending,
                    )
                    .await?;
                    if rows == 0 {
                        tracing::info!(
                            "[retry_payment_callback] order {} CAS failed, skipping",
                            order.id.to_string()
                        );
                        return Ok(());
                    }

                    let outbox_cmd = CreateWalletOutboxCmd {
                        user_id: order.user_id,
                        currency: order.currency.clone(),
                        amount: order.amount,
                        entry_type: "credit".into(),
                        tx_type: WalletTxType::Recharge,
                        transaction_no: format!("PAY-{}", order.id),
                        reference_type: Some(WalletReferenceType::Payment),
                        reference_id: Some(order.id.to_string()),
                        metadata: None,
                    };
                    crate::models::wallet_outbox::tx_insert(
                        &mut tx,
                        &outbox_cmd,
                        order.tenant_id.as_deref(),
                    )
                    .await?;

                    Ok(())
                })?;
            }
            PaymentStatus::Cancelled => {
                crate::in_transaction!(&self.pool, tx, {
                    let rows = crate::models::payment_order::tx_update_status_cas(
                        &mut tx,
                        order.id,
                        PaymentStatus::Cancelled,
                        Some("cancelled_at"),
                        PaymentStatus::Pending,
                    )
                    .await?;
                    if rows == 0 {
                        tracing::info!(
                            "[retry_payment_callback] order {} CAS failed for cancel",
                            order.id.to_string()
                        );
                    }
                    Ok(())
                })?;
            }
            PaymentStatus::Expired => {
                crate::in_transaction!(&self.pool, tx, {
                    let rows = crate::models::payment_order::tx_update_status_cas(
                        &mut tx,
                        order.id,
                        PaymentStatus::Expired,
                        Some("expired_at"),
                        PaymentStatus::Pending,
                    )
                    .await?;
                    if rows == 0 {
                        tracing::info!(
                            "[retry_payment_callback] order {} CAS failed for expire",
                            order.id.to_string()
                        );
                    }
                    Ok(())
                })?;
            }
            PaymentStatus::Pending => {
                tracing::info!(
                    "[retry_payment_callback] order {} still pending at provider, will retry later",
                    order.id.to_string()
                );
            }
            _ => {
                tracing::info!(
                    "[retry_payment_callback] order {} provider status {:?}, no action",
                    order.id.to_string(),
                    status.status
                );
            }
        }

        Ok(())
    }
}

fn get_encrypt_key(config: &AppConfig) -> AppResult<[u8; 32]> {
    let key_str = config.app_key.as_deref().ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!("APP_KEY not configured"))
    })?;
    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key_str)
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "APP_KEY base64 decode: {e}"
            ))
        })?;
    if decoded.len() != 32 {
        return Err(crate::errors::app_error::AppError::Internal(
            anyhow::anyhow!("APP_KEY must be 32 bytes, got {}", decoded.len()),
        ));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&decoded);
    Ok(arr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = RetryPaymentCallbackHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn handles_retry_job() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = RetryPaymentCallbackHandler::new(pool, config);
        let job = Job::RetryPaymentCallback {
            payment_order_id: SnowflakeId(42),
        };
        let result = handler.handle(&job).await;
        assert!(result.is_err());
    }
}
