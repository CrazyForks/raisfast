use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PaymentTransaction {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub payment_order_id: SnowflakeId,
    pub order_id: Option<String>,
    pub user_id: SnowflakeId,
    pub tx_type: String,
    pub amount: i64,
    pub currency: String,
    pub provider_tx_id: String,
    pub status: String,
    pub raw_payload: Option<String>,
    pub created_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentTransaction>> {
    raisfast_derive::crud_find!(pool, "payment_transactions", PaymentTransaction, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_payment_order_id(
    pool: &crate::db::Pool,
    payment_order_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentTransaction>> {
    raisfast_derive::crud_find_all!(pool, "payment_transactions", PaymentTransaction, where: ("payment_order_id", payment_order_id), tenant: tenant_id, order_by: "created_at DESC")
        .map_err(Into::into)
}

pub async fn find_by_order_id(
    pool: &crate::db::Pool,
    order_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentTransaction>> {
    raisfast_derive::crud_find_all!(pool, "payment_transactions", PaymentTransaction, where: ("order_id", order_id), tenant: tenant_id, order_by: "created_at DESC")
        .map_err(Into::into)
}

pub async fn find_by_provider_tx_id(
    pool: &crate::db::Pool,
    provider_tx_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentTransaction>> {
    raisfast_derive::crud_find!(pool, "payment_transactions", PaymentTransaction, where: ("provider_tx_id", provider_tx_id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentTransaction>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, PaymentTransaction,
        table: "payment_transactions",
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePaymentTransactionCmd,
    tenant_id: Option<&str>,
) -> AppResult<PaymentTransaction> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "payment_transactions",
        [
            "id" => id,
            "payment_order_id" => cmd.payment_order_id,
            "order_id" => &cmd.order_id,
            "user_id" => cmd.user_id,
            "tx_type" => &cmd.tx_type,
            "amount" => cmd.amount,
            "currency" => &cmd.currency,
            "provider_tx_id" => &cmd.provider_tx_id,
            "status" => &cmd.status,
            "raw_payload" => &cmd.raw_payload,
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, SnowflakeId(id), tenant_id)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "inserted row not found: {id}"
            ))
        })
}

pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::CreatePaymentTransactionCmd,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        &mut *tx,
        "payment_transactions",
        [
            "id" => id,
            "payment_order_id" => cmd.payment_order_id,
            "order_id" => &cmd.order_id,
            "user_id" => cmd.user_id,
            "tx_type" => &cmd.tx_type,
            "amount" => cmd.amount,
            "currency" => &cmd.currency,
            "provider_tx_id" => &cmd.provider_tx_id,
            "status" => &cmd.status,
            "raw_payload" => &cmd.raw_payload,
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("testuser_{id}");
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')")
            .bind(id)
            .bind(&username)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_channel(pool: &crate::db::Pool) -> i64 {
        let name = format!("stripe-{}", uuid::Uuid::now_v7());
        crate::models::payment_channel::insert(
            pool,
            &crate::commands::CreatePaymentChannelCmd {
                provider: "stripe".into(),
                name,
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: None,
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as(
            "SELECT id FROM payment_channels WHERE provider = 'stripe' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_payment_order(pool: &crate::db::Pool, user_id: i64, channel_id: i64) -> i64 {
        let idem_key = format!("idem_{}", uuid::Uuid::now_v7());
        crate::models::payment_order::insert(
            pool,
            &crate::commands::CreatePaymentOrderCmd {
                user_id: SnowflakeId(user_id),
                order_id: Some("order-ref-1".into()),
                title: "Test Payment".into(),
                amount: 1000,
                currency: "USD".into(),
                channel_id: SnowflakeId(channel_id),
                provider: "stripe".into(),
                reference_type: None,
                reference_id: None,
                return_url: None,
                idempotency_key: idem_key,
                client_ip: None,
                client_language: None,
                client_country: None,
                client_user_agent: None,
                channel_selected_by: None,
                metadata: None,
            },
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as(
            "SELECT id FROM payment_orders WHERE provider = 'stripe' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(pool)
        .await
        .unwrap();
        id
    }

    async fn seed_tx(
        pool: &crate::db::Pool,
        payment_order_id: i64,
        user_id: i64,
        tx_type: &str,
        provider_tx_id: &str,
    ) -> PaymentTransaction {
        super::insert(
            pool,
            &crate::commands::CreatePaymentTransactionCmd {
                payment_order_id: SnowflakeId(payment_order_id),
                order_id: Some("order-ref-1".into()),
                user_id: SnowflakeId(user_id),
                tx_type: tx_type.into(),
                amount: 1000,
                currency: "USD".into(),
                provider_tx_id: provider_tx_id.into(),
                status: "succeeded".into(),
                raw_payload: Some(r#"{"event":"charge.succeeded"}"#.into()),
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        let tx = seed_tx(&pool, po_id, uid, "charge", "ch_abc123").await;
        let found = super::find_by_id(&pool, tx.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, tx.id);
        assert_eq!(found.payment_order_id, SnowflakeId(po_id));
        assert_eq!(found.tx_type, "charge");
        assert_eq!(found.amount, 1000);
        assert_eq!(found.provider_tx_id, "ch_abc123");
        assert_eq!(found.status, "succeeded");
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_id(&pool, SnowflakeId(99999), None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_payment_order_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        seed_tx(&pool, po_id, uid, "charge", "ch_001").await;
        seed_tx(&pool, po_id, uid, "refund", "re_001").await;
        let txs = super::find_by_payment_order_id(&pool, SnowflakeId(po_id), None)
            .await
            .unwrap();
        assert_eq!(txs.len(), 2);
    }

    #[tokio::test]
    async fn find_by_order_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        seed_tx(&pool, po_id, uid, "charge", "ch_002").await;
        let txs = super::find_by_order_id(&pool, "order-ref-1", None)
            .await
            .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].order_id.as_deref().unwrap(), "order-ref-1");
    }

    #[tokio::test]
    async fn find_by_provider_tx_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        seed_tx(&pool, po_id, uid, "charge", "ch_unique").await;
        let found = super::find_by_provider_tx_id(&pool, "ch_unique", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.provider_tx_id, "ch_unique");
    }

    #[tokio::test]
    async fn find_by_provider_tx_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_provider_tx_id(&pool, "nonexistent", None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn raw_payload_stored() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        let tx = seed_tx(&pool, po_id, uid, "charge", "ch_payload").await;
        assert_eq!(tx.raw_payload.unwrap(), r#"{"event":"charge.succeeded"}"#);
    }
}
