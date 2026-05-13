use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::payment_channel;
use crate::models::payment_order::{self, PaymentStatus};
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
                order.document_id,
                order.status
            );
            return Ok(());
        }

        let Some(ref provider_order_id) = order.provider_order_id else {
            tracing::warn!(
                "[retry_payment_callback] order {} has no provider_order_id, expiring",
                order.document_id
            );
            payment_order::update_status(
                &self.pool,
                order.id,
                PaymentStatus::Expired.as_str(),
                Some("expired_at"),
                None,
            )
            .await?;
            return Ok(());
        };

        let key = get_encrypt_key(&self.config)?;
        let provider = match crate::payment::providers::get_provider(&order.provider, &key) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(
                    "[retry_payment_callback] provider '{}' not available for order {}: {e}",
                    order.provider,
                    order.document_id
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
                    order.document_id
                );
                payment_order::update_status(
                    &self.pool,
                    order.id,
                    PaymentStatus::Paid.as_str(),
                    Some("paid_at"),
                    None,
                )
                .await?;
            }
            PaymentStatus::Cancelled => {
                payment_order::update_status(
                    &self.pool,
                    order.id,
                    PaymentStatus::Cancelled.as_str(),
                    Some("cancelled_at"),
                    None,
                )
                .await?;
            }
            PaymentStatus::Expired => {
                payment_order::update_status(
                    &self.pool,
                    order.id,
                    PaymentStatus::Expired.as_str(),
                    Some("expired_at"),
                    None,
                )
                .await?;
            }
            PaymentStatus::Pending => {
                tracing::info!(
                    "[retry_payment_callback] order {} still pending at provider, will retry later",
                    order.document_id
                );
            }
            _ => {
                tracing::info!(
                    "[retry_payment_callback] order {} provider status {:?}, no action",
                    order.document_id,
                    status.status
                );
            }
        }

        Ok(())
    }
}

fn get_encrypt_key(config: &AppConfig) -> AppResult<[u8; 32]> {
    let key_str = config
        .app_key
        .as_deref()
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "APP_KEY not configured"
            ))
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
            payment_order_id: 42,
        };
        let result = handler.handle(&job).await;
        assert!(result.is_err());
    }
}
