use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::models::payment_channel;
use crate::models::payment_order::PaymentStatus;
use crate::worker::{Job, JobHandler};

pub struct ExpirePaymentOrdersHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl ExpirePaymentOrdersHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for ExpirePaymentOrdersHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::ExpirePaymentOrders = job else {
            return Ok(());
        };

        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(30);

        let sql = format!(
            "SELECT * FROM payment_orders WHERE status = 'pending' AND created_at < {} LIMIT 500",
            Driver::ph(1)
        );
        let orders: Vec<crate::models::payment_order::PaymentOrder> = sqlx::query_as(&sql)
            .bind(cutoff.format("%Y-%m-%d %H:%M:%S").to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut expired = 0u64;
        let mut skipped = 0u64;

        for order in &orders {
            match self.should_expire(order).await {
                Ok(true) => {
                    crate::in_transaction!(&self.pool, tx, {
                        let rows = crate::models::payment_order::tx_update_status_cas(
                            &mut tx,
                            order.id,
                            PaymentStatus::Expired,
                            Some("expired_at"),
                            PaymentStatus::Pending,
                        )
                        .await?;
                        if rows > 0 {
                            expired += 1;
                        } else {
                            tracing::info!(
                                "[expire_payment_orders] order {} CAS failed, skipped",
                                order.id.to_string()
                            );
                            skipped += 1;
                        }
                        Ok(())
                    })?;
                }
                Ok(false) => {
                    skipped += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        "[expire_payment_orders] error checking order {}: {e}",
                        order.id.to_string()
                    );
                    skipped += 1;
                }
            }
        }

        if expired > 0 || skipped > 0 {
            tracing::info!("[expire_payment_orders] expired {expired} order(s), skipped {skipped}");
        }
        Ok(())
    }
}

impl ExpirePaymentOrdersHandler {
    async fn should_expire(
        &self,
        order: &crate::models::payment_order::PaymentOrder,
    ) -> AppResult<bool> {
        let Some(ref provider_order_id) = order.provider_order_id else {
            return Ok(true);
        };

        let key = get_encrypt_key(&self.config)?;
        let provider = match crate::payment::providers::get_provider(&order.provider, &key) {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    "[expire_payment_orders] provider '{}' not available, skipping order {}",
                    order.provider,
                    order.id.to_string()
                );
                return Ok(false);
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
            PaymentStatus::Pending => Ok(true),
            PaymentStatus::Paid => {
                tracing::info!(
                    "[expire_payment_orders] order {} is paid at provider, skipping expiry",
                    order.id.to_string()
                );
                Ok(false)
            }
            _ => Ok(false),
        }
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

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = ExpirePaymentOrdersHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }
}
