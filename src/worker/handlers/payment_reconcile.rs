use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::models::payment_channel;
use crate::models::payment_order::PaymentStatus;
use crate::services::audit::AuditService;
use crate::worker::{Job, JobHandler};

pub struct ReconcilePaymentsHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl ReconcilePaymentsHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for ReconcilePaymentsHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::ReconcilePayments = job else {
            return Ok(());
        };

        let yesterday_start = (chrono::Utc::now() - chrono::Duration::days(1))
            .format("%Y-%m-%dT00:00:00Z")
            .to_string();
        let yesterday_end = chrono::Utc::now().format("%Y-%m-%dT00:00:00Z").to_string();

        let sql = format!(
            "SELECT * FROM payment_orders WHERE status = 'paid' AND paid_at >= {} AND paid_at < {} LIMIT 500",
            Driver::ph(1),
            Driver::ph(2)
        );
        let orders: Vec<crate::models::payment_order::PaymentOrder> = sqlx::query_as(&sql)
            .bind(&yesterday_start)
            .bind(&yesterday_end)
            .fetch_all(&self.pool)
            .await?;

        let mut reconciled = 0u64;
        let mut mismatches = 0u64;

        for order in &orders {
            match self.reconcile_order(order).await {
                Ok(true) => reconciled += 1,
                Ok(false) => mismatches += 1,
                Err(e) => {
                    tracing::warn!(
                        "[reconcile_payments] error reconciling order {}: {e}",
                        order.id.to_string()
                    );
                }
            }
        }

        tracing::info!(
            "[reconcile_payments] checked {} orders, {reconciled} ok, {mismatches} mismatches",
            orders.len()
        );
        Ok(())
    }
}

impl ReconcilePaymentsHandler {
    async fn reconcile_order(
        &self,
        order: &crate::models::payment_order::PaymentOrder,
    ) -> AppResult<bool> {
        let Some(ref provider_order_id) = order.provider_order_id else {
            return Ok(true);
        };

        let key = get_encrypt_key(&self.config)?;
        let provider = match crate::payment::providers::get_provider(&order.provider, &key) {
            Ok(p) => p,
            Err(_) => return Ok(true),
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
        let audit = AuditService::new(self.pool.clone());

        if status.status != PaymentStatus::Paid {
            let detail = format!(
                "local=paid provider={:?} order={} amount={}",
                status.status, order.id, order.amount
            );
            tracing::error!("[reconcile_payments] critical: {detail}");
            if let Err(audit_err) = audit
                .log(
                    "",
                    None,
                    None,
                    "payment_reconcile_mismatch",
                    "payment_order",
                    Some(&order.id.to_string()),
                    Some(&detail),
                    None,
                    None,
                )
                .await
            {
                tracing::error!("[reconcile_payments] audit log failed: {audit_err}");
            }
            return Ok(false);
        }

        if let Some(provider_amount) = status.amount
            && provider_amount != order.amount
        {
            let detail = format!(
                "amount_mismatch local={} provider={} order={}",
                order.amount, provider_amount, order.id
            );
            tracing::error!("[reconcile_payments] critical: {detail}");
            if let Err(audit_err) = audit
                .log(
                    "",
                    None,
                    None,
                    "payment_reconcile_amount_mismatch",
                    "payment_order",
                    Some(&order.id.to_string()),
                    Some(&detail),
                    None,
                    None,
                )
                .await
            {
                tracing::error!("[reconcile_payments] audit log failed: {audit_err}");
            }
            return Ok(false);
        }

        Ok(true)
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
        let handler = ReconcilePaymentsHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn handles_reconcile_job() {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = Arc::new(AppConfig::test_defaults());
        let handler = ReconcilePaymentsHandler::new(pool, config);
        let job = Job::ReconcilePayments;
        assert!(handler.handle(&job).await.is_ok());
    }
}
