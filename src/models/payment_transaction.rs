use serde::{Deserialize, Serialize};

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaymentTransaction {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub payment_order_id: i64,
    pub order_id: Option<String>,
    pub user_id: i64,
    pub tx_type: String,
    pub amount: i64,
    pub currency: String,
    pub provider_tx_id: String,
    pub status: String,
    pub raw_payload: Option<String>,
    pub created_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(PaymentTransaction {
    required { id, document_id, payment_order_id, user_id, tx_type, amount, currency, provider_tx_id, status, created_at }
    optional { order_id, raw_payload }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentTransaction>> {
    let sql = format!(
        "SELECT * FROM payment_transactions WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentTransaction>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_payment_order_id(
    pool: &crate::db::Pool,
    payment_order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentTransaction>> {
    let sql = format!(
        "SELECT * FROM payment_transactions WHERE payment_order_id = {}{} ORDER BY created_at DESC",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentTransaction>(&sql).bind(payment_order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

pub async fn find_by_order_id(
    pool: &crate::db::Pool,
    order_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentTransaction>> {
    let sql = format!(
        "SELECT * FROM payment_transactions WHERE order_id = {}{} ORDER BY created_at DESC",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentTransaction>(&sql).bind(order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

pub async fn find_by_provider_tx_id(
    pool: &crate::db::Pool,
    provider_tx_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentTransaction>> {
    let sql = format!(
        "SELECT * FROM payment_transactions WHERE provider_tx_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentTransaction>(&sql).bind(provider_tx_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentTransaction>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 1);
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM payment_transactions WHERE 1=1{}",
        tenant_ph
    );
    let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let (total,): (i64,) = cq.fetch_one(pool).await?;
    let base = usize::from(tenant_id.is_some()) + 1;
    let sql = format!(
        "SELECT * FROM payment_transactions WHERE 1=1{} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        tenant_filter_ph(tenant_id, 1),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, PaymentTransaction>(&sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    let rows = dq.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &crate::db::Pool,
    document_id: &str,
    payment_order_id: i64,
    order_id: Option<&str>,
    user_id: i64,
    tx_type: &str,
    amount: i64,
    currency: &str,
    provider_tx_id: &str,
    status: &str,
    raw_payload: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<PaymentTransaction> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO payment_transactions (document_id, tenant_id, payment_order_id, order_id, user_id, tx_type, amount, currency, provider_tx_id, status, raw_payload, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(tid)
                .bind(payment_order_id)
                .bind(order_id)
                .bind(user_id)
                .bind(tx_type)
                .bind(amount)
                .bind(currency)
                .bind(provider_tx_id)
                .bind(status)
                .bind(raw_payload)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO payment_transactions (document_id, payment_order_id, order_id, user_id, tx_type, amount, currency, provider_tx_id, status, raw_payload, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(payment_order_id)
                .bind(order_id)
                .bind(user_id)
                .bind(tx_type)
                .bind(amount)
                .bind(currency)
                .bind(provider_tx_id)
                .bind(status)
                .bind(raw_payload)
                .execute(pool)
                .await?;
        }
    }
    let sql2 = format!(
        "SELECT * FROM payment_transactions WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentTransaction>(&sql2).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
    payment_order_id: i64,
    order_id: Option<&str>,
    user_id: i64,
    tx_type: &str,
    amount: i64,
    currency: &str,
    provider_tx_id: &str,
    status: &str,
    raw_payload: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    if let Some(tid) = tenant_id {
        let sql = format!(
            "INSERT INTO payment_transactions (document_id, payment_order_id, order_id, user_id, tx_type, amount, currency, provider_tx_id, status, raw_payload, tenant_id, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
            ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11)
        );
        sqlx::query(&sql)
            .bind(document_id)
            .bind(payment_order_id)
            .bind(order_id)
            .bind(user_id)
            .bind(tx_type)
            .bind(amount)
            .bind(currency)
            .bind(provider_tx_id)
            .bind(status)
            .bind(raw_payload)
            .bind(tid)
            .execute(&mut *tx)
            .await?;
    } else {
        let sql = format!(
            "INSERT INTO payment_transactions (document_id, payment_order_id, order_id, user_id, tx_type, amount, currency, provider_tx_id, status, raw_payload, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
            ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10)
        );
        sqlx::query(&sql)
            .bind(document_id)
            .bind(payment_order_id)
            .bind(order_id)
            .bind(user_id)
            .bind(tx_type)
            .bind(amount)
            .bind(currency)
            .bind(provider_tx_id)
            .bind(status)
            .bind(raw_payload)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
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
        let doc_id = uuid::Uuid::now_v7().to_string();
        let name = format!("stripe-{}", &doc_id[..8]);
        crate::models::payment_channel::insert(
            pool,
            &doc_id,
            "stripe",
            &name,
            false,
            r#"{"api_key":"test"}"#,
            None,
            None,
            true,
            0,
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM payment_channels WHERE document_id = ?")
            .bind(&doc_id)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_payment_order(pool: &crate::db::Pool, user_id: i64, channel_id: i64) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let idem_key = format!("idem_{}", &doc_id[..16]);
        crate::models::payment_order::insert(
            pool,
            &doc_id,
            user_id,
            Some("order-ref-1"),
            "Test Payment",
            1000,
            "USD",
            channel_id,
            "stripe",
            None,
            None,
            None,
            &idem_key,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM payment_orders WHERE document_id = ?")
            .bind(&doc_id)
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
        let doc_id = uuid::Uuid::now_v7().to_string();
        super::insert(
            pool,
            &doc_id,
            payment_order_id,
            Some("order-ref-1"),
            user_id,
            tx_type,
            1000,
            "USD",
            provider_tx_id,
            "succeeded",
            Some(r#"{"event":"charge.succeeded"}"#),
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
        assert_eq!(found.payment_order_id, po_id);
        assert_eq!(found.tx_type, "charge");
        assert_eq!(found.amount, 1000);
        assert_eq!(found.provider_tx_id, "ch_abc123");
        assert_eq!(found.status, "succeeded");
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
        seed_tx(&pool, po_id, uid, "charge", "ch_001").await;
        seed_tx(&pool, po_id, uid, "refund", "re_001").await;
        let txs = super::find_by_payment_order_id(&pool, po_id, None)
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
