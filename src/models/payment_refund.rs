use serde::{Deserialize, Serialize};

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaymentRefund {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub payment_order_id: i64,
    pub order_id: Option<String>,
    pub user_id: i64,
    pub amount: i64,
    pub currency: String,
    pub reason: Option<String>,
    pub provider_refund_id: Option<String>,
    pub status: String,
    pub payment_tx_id: Option<i64>,
    pub metadata: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(PaymentRefund {
    required { id, document_id, payment_order_id, user_id, amount, currency, status, created_at, updated_at }
    optional { order_id, reason, provider_refund_id, payment_tx_id, metadata }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentRefund>> {
    let sql = format!(
        "SELECT * FROM payment_refunds WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentRefund>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_payment_order_id(
    pool: &crate::db::Pool,
    payment_order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentRefund>> {
    let sql = format!(
        "SELECT * FROM payment_refunds WHERE payment_order_id = {}{} ORDER BY created_at DESC",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentRefund>(&sql).bind(payment_order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

pub async fn find_by_order_id(
    pool: &crate::db::Pool,
    order_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentRefund>> {
    let sql = format!(
        "SELECT * FROM payment_refunds WHERE order_id = {}{} ORDER BY created_at DESC",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentRefund>(&sql).bind(order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePaymentRefundCmd,
    tenant_id: Option<&str>,
) -> AppResult<PaymentRefund> {
    let document_id = uuid::Uuid::now_v7().to_string();
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO payment_refunds (document_id, tenant_id, payment_order_id, order_id, user_id, amount, currency, reason, provider_refund_id, status, payment_tx_id, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11), ph(12)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(cmd.payment_order_id)
                .bind(&cmd.order_id)
                .bind(cmd.user_id)
                .bind(cmd.amount)
                .bind(&cmd.currency)
                .bind(&cmd.reason)
                .bind(&cmd.provider_refund_id)
                .bind(&cmd.status)
                .bind(cmd.payment_tx_id)
                .bind(&cmd.metadata)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO payment_refunds (document_id, payment_order_id, order_id, user_id, amount, currency, reason, provider_refund_id, status, payment_tx_id, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(cmd.payment_order_id)
                .bind(&cmd.order_id)
                .bind(cmd.user_id)
                .bind(cmd.amount)
                .bind(&cmd.currency)
                .bind(&cmd.reason)
                .bind(&cmd.provider_refund_id)
                .bind(&cmd.status)
                .bind(cmd.payment_tx_id)
                .bind(&cmd.metadata)
                .execute(pool)
                .await?;
        }
    }
    let sql2 = format!(
        "SELECT * FROM payment_refunds WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentRefund>(&sql2).bind(&document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: i64,
    status: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE payment_refunds SET status = {}, updated_at = datetime('now') WHERE id = {}{}",
        ph(1),
        ph(2),
        tenant_filter_ph(tenant_id, 3)
    );
    let mut q = sqlx::query(&sql).bind(status).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn sum_refunded_by_order(
    pool: &crate::db::Pool,
    payment_order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let sql = format!(
        "SELECT COALESCE(SUM(amount), 0) as total FROM payment_refunds WHERE payment_order_id = {} AND status IN ('pending', 'processing', 'succeeded'){}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(payment_order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let (total,) = q.fetch_one(pool).await?;
    Ok(total)
}

pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::CreatePaymentRefundCmd,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let document_id = uuid::Uuid::now_v7().to_string();
    if let Some(tid) = tenant_id {
        let sql = format!(
            "INSERT INTO payment_refunds (document_id, payment_order_id, order_id, user_id, amount, currency, reason, provider_refund_id, status, metadata, tenant_id, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
            ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11)
        );
        sqlx::query(&sql)
            .bind(&document_id)
            .bind(cmd.payment_order_id)
            .bind(&cmd.order_id)
            .bind(cmd.user_id)
            .bind(cmd.amount)
            .bind(&cmd.currency)
            .bind(&cmd.reason)
            .bind(&cmd.provider_refund_id)
            .bind(&cmd.status)
            .bind(&cmd.metadata)
            .bind(tid)
            .execute(&mut *tx)
            .await?;
    } else {
        let sql = format!(
            "INSERT INTO payment_refunds (document_id, payment_order_id, order_id, user_id, amount, currency, reason, provider_refund_id, status, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
            ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10)
        );
        sqlx::query(&sql)
            .bind(&document_id)
            .bind(cmd.payment_order_id)
            .bind(&cmd.order_id)
            .bind(cmd.user_id)
            .bind(cmd.amount)
            .bind(&cmd.currency)
            .bind(&cmd.reason)
            .bind(&cmd.provider_refund_id)
            .bind(&cmd.status)
            .bind(&cmd.metadata)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

pub async fn tx_sum_refunded_by_order(
    tx: &mut crate::db::pool::DbConnection,
    payment_order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let sql = if tenant_id.is_some() {
        format!(
            "SELECT COALESCE(SUM(amount), 0) FROM payment_refunds WHERE payment_order_id = {} AND tenant_id = {} AND status IN ('succeeded', 'pending', 'processing')",
            ph(1), ph(2)
        )
    } else {
        format!(
            "SELECT COALESCE(SUM(amount), 0) FROM payment_refunds WHERE payment_order_id = {} AND status IN ('succeeded', 'pending', 'processing')",
            ph(1)
        )
    };
    let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(payment_order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let (total,) = q.fetch_one(&mut *tx).await?;
    Ok(total)
}

pub async fn tx_find_by_document_id(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
) -> AppResult<Option<PaymentRefund>> {
    let sql = format!(
        "SELECT * FROM payment_refunds WHERE document_id = {}",
        ph(1)
    );
    sqlx::query_as::<_, PaymentRefund>(&sql)
        .bind(document_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let username = format!("testuser_{doc_id}");
        sqlx::query("INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')")
            .bind(&doc_id)
            .bind(&username)
            .execute(pool)
            .await
            .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM users WHERE document_id = ?")
            .bind(&doc_id)
            .fetch_one(pool)
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
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM payment_channels WHERE provider = 'stripe' ORDER BY id DESC LIMIT 1")
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
                user_id,
                order_id: Some("order-ref-1".into()),
                title: "Test Payment".into(),
                amount: 1000,
                currency: "USD".into(),
                channel_id,
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
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM payment_orders WHERE provider = 'stripe' ORDER BY id DESC LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_refund(
        pool: &crate::db::Pool,
        payment_order_id: i64,
        user_id: i64,
        amount: i64,
        status: &str,
    ) -> PaymentRefund {
        let provider_refund_id = format!("re_{}", uuid::Uuid::now_v7());
        super::insert(
            pool,
            &crate::commands::CreatePaymentRefundCmd {
                payment_order_id,
                order_id: Some("order-ref-1".into()),
                user_id,
                amount,
                currency: "USD".into(),
                reason: Some("user_request".into()),
                provider_refund_id: Some(provider_refund_id),
                status: status.into(),
                payment_tx_id: None,
                metadata: None,
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
        let refund = seed_refund(&pool, po_id, uid, 500, "pending").await;
        let found = super::find_by_id(&pool, refund.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, refund.id);
        assert_eq!(found.payment_order_id, po_id);
        assert_eq!(found.amount, 500);
        assert_eq!(found.status, "pending");
        assert_eq!(found.reason.unwrap(), "user_request");
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_id(&pool, 99999, None)
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
        seed_refund(&pool, po_id, uid, 300, "pending").await;
        seed_refund(&pool, po_id, uid, 200, "succeeded").await;
        let refunds = super::find_by_payment_order_id(&pool, po_id, None)
            .await
            .unwrap();
        assert_eq!(refunds.len(), 2);
    }

    #[tokio::test]
    async fn find_by_order_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        seed_refund(&pool, po_id, uid, 500, "pending").await;
        let refunds = super::find_by_order_id(&pool, "order-ref-1", None)
            .await
            .unwrap();
        assert_eq!(refunds.len(), 1);
        assert_eq!(refunds[0].order_id.as_deref().unwrap(), "order-ref-1");
    }

    #[tokio::test]
    async fn update_status_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        let refund = seed_refund(&pool, po_id, uid, 500, "pending").await;
        super::update_status(&pool, refund.id, "succeeded", None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, refund.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, "succeeded");
    }

    #[tokio::test]
    async fn sum_refunded_by_order_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        seed_refund(&pool, po_id, uid, 300, "succeeded").await;
        seed_refund(&pool, po_id, uid, 200, "pending").await;
        seed_refund(&pool, po_id, uid, 100, "failed").await;
        let total = super::sum_refunded_by_order(&pool, po_id, None)
            .await
            .unwrap();
        assert_eq!(total, 500);
    }

    #[tokio::test]
    async fn sum_refunded_by_order_empty() {
        let pool = setup_pool().await;
        let total = super::sum_refunded_by_order(&pool, 99999, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
    }

    #[tokio::test]
    async fn insert_sets_defaults() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let po_id = seed_payment_order(&pool, uid, ch_id).await;
        let refund = seed_refund(&pool, po_id, uid, 500, "pending").await;
        assert!(refund.tenant_id.is_none());
        assert!(refund.payment_tx_id.is_none());
        assert!(refund.metadata.is_none());
        assert!(refund.provider_refund_id.is_some());
    }
}
