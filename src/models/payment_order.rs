use serde::{Deserialize, Serialize};

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    PaymentStatus {
        Pending = "pending",
        Paid = "paid",
        Failed = "failed",
        Cancelled = "cancelled",
        Refunded = "refunded",
        PartiallyRefunded = "partially_refunded",
        Expired = "expired",
    }
);

define_enum!(
    PaymentTxType {
        Charge = "charge",
        Refund = "refund",
    }
);

define_enum!(
    PaymentRefundStatus {
        Pending = "pending",
        Processing = "processing",
        Succeeded = "succeeded",
        Failed = "failed",
    }
);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaymentOrder {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub user_id: i64,
    pub order_id: Option<String>,
    pub title: String,
    pub amount: i64,
    pub currency: String,
    pub channel_id: i64,
    pub provider: String,
    pub provider_order_id: Option<String>,
    pub provider_method: Option<String>,
    pub status: PaymentStatus,
    pub reference_type: Option<String>,
    pub reference_id: Option<String>,
    pub return_url: Option<String>,
    pub idempotency_key: String,
    pub version: i64,
    pub provider_data: Option<String>,
    pub client_ip: Option<String>,
    pub metadata: Option<String>,
    pub paid_at: Option<Timestamp>,
    pub cancelled_at: Option<Timestamp>,
    pub expired_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(PaymentOrder {
    required { id, document_id, user_id, title, amount, currency, channel_id, provider, status, idempotency_key, version, created_at, updated_at }
    optional { order_id, provider_order_id, provider_method, reference_type, reference_id, return_url, provider_data, client_ip, metadata, paid_at, cancelled_at, expired_at }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    let sql = format!(
        "SELECT * FROM payment_orders WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentOrder>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    let sql = format!(
        "SELECT * FROM payment_orders WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentOrder>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_idempotency_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    let sql = format!(
        "SELECT * FROM payment_orders WHERE idempotency_key = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentOrder>(&sql).bind(key);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_provider_order_id(
    pool: &crate::db::Pool,
    provider_order_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    let sql = format!(
        "SELECT * FROM payment_orders WHERE provider_order_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentOrder>(&sql).bind(provider_order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_user_paginated(
    pool: &crate::db::Pool,
    user_id: i64,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 2);
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM payment_orders WHERE user_id = {}{}",
        ph(1),
        tenant_ph
    );
    let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql).bind(user_id);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let (total,): (i64,) = cq.fetch_one(pool).await?;
    let base = usize::from(tenant_id.is_some()) + 2;
    let sql = format!(
        "SELECT * FROM payment_orders WHERE user_id = {}{} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        tenant_filter_ph(tenant_id, 2),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, PaymentOrder>(&sql).bind(user_id);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    let rows = dq.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 1);
    let has_tenant = tenant_id.is_some();
    let status_ph_idx = if has_tenant { 2 } else { 1 };
    let (count_sql, data_sql_base) = if let Some(_s) = status {
        (
            format!(
                "SELECT COUNT(*) as count FROM payment_orders WHERE status = {}{}",
                ph(status_ph_idx),
                tenant_ph
            ),
            format!(
                "SELECT * FROM payment_orders WHERE status = {}{} ORDER BY created_at DESC",
                ph(status_ph_idx),
                tenant_ph
            ),
        )
    } else {
        (
            format!(
                "SELECT COUNT(*) as count FROM payment_orders WHERE 1=1{}",
                tenant_ph
            ),
            format!(
                "SELECT * FROM payment_orders WHERE 1=1{} ORDER BY created_at DESC",
                tenant_ph
            ),
        )
    };
    let mut q = sqlx::query_as::<_, (i64,)>(&count_sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(ref s) = status {
        q = q.bind(s);
    }
    let (total,): (i64,) = q.fetch_one(pool).await?;
    let limit_base = status_ph_idx + usize::from(status.is_some());
    let sql = format!(
        "{} LIMIT {} OFFSET {}",
        data_sql_base,
        ph(limit_base + 1),
        ph(limit_base + 2)
    );
    let mut q2 = sqlx::query_as::<_, PaymentOrder>(&sql);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    if let Some(s) = status {
        q2 = q2.bind(s);
    }
    let rows = q2.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &crate::db::Pool,
    document_id: &str,
    user_id: i64,
    order_id: Option<&str>,
    title: &str,
    amount: i64,
    currency: &str,
    channel_id: i64,
    provider: &str,
    reference_type: Option<&str>,
    reference_id: Option<&str>,
    return_url: Option<&str>,
    idempotency_key: &str,
    client_ip: Option<&str>,
    metadata: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<PaymentOrder> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO payment_orders (document_id, tenant_id, user_id, order_id, title, amount, currency, channel_id, provider, reference_type, reference_id, return_url, idempotency_key, client_ip, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
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
                ph(11),
                ph(12),
                ph(13),
                ph(14),
                ph(15)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(tid)
                .bind(user_id)
                .bind(order_id)
                .bind(title)
                .bind(amount)
                .bind(currency)
                .bind(channel_id)
                .bind(provider)
                .bind(reference_type)
                .bind(reference_id)
                .bind(return_url)
                .bind(idempotency_key)
                .bind(client_ip)
                .bind(metadata)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO payment_orders (document_id, user_id, order_id, title, amount, currency, channel_id, provider, reference_type, reference_id, return_url, idempotency_key, client_ip, metadata, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
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
                ph(11),
                ph(12),
                ph(13),
                ph(14)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(user_id)
                .bind(order_id)
                .bind(title)
                .bind(amount)
                .bind(currency)
                .bind(channel_id)
                .bind(provider)
                .bind(reference_type)
                .bind(reference_id)
                .bind(return_url)
                .bind(idempotency_key)
                .bind(client_ip)
                .bind(metadata)
                .execute(pool)
                .await?;
        }
    }
    find_by_document_id(pool, document_id, tenant_id)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "inserted row not found: {document_id}"
            ))
        })
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: i64,
    status: &str,
    timestamp_col: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = if let Some(col) = timestamp_col {
        format!(
            "UPDATE payment_orders SET status = {}, {} = datetime('now'), updated_at = datetime('now'), version = version + 1 WHERE id = {}{}",
            ph(1),
            col,
            ph(2),
            tenant_filter_ph(tenant_id, 3)
        )
    } else {
        format!(
            "UPDATE payment_orders SET status = {}, updated_at = datetime('now'), version = version + 1 WHERE id = {}{}",
            ph(1),
            ph(2),
            tenant_filter_ph(tenant_id, 3)
        )
    };
    let mut q = sqlx::query(&sql).bind(status).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn update_provider_order_id(
    pool: &crate::db::Pool,
    id: i64,
    provider_order_id: &str,
    provider_data: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE payment_orders SET provider_order_id = {}, provider_data = {}, updated_at = datetime('now') WHERE id = {}{}",
        ph(1),
        ph(2),
        ph(3),
        tenant_filter_ph(tenant_id, 4)
    );
    let mut q = sqlx::query(&sql)
        .bind(provider_order_id)
        .bind(provider_data)
        .bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
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

    async fn seed_payment_order(
        pool: &crate::db::Pool,
        user_id: i64,
        channel_id: i64,
        amount: i64,
    ) -> PaymentOrder {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let idem_key = format!("idem_{}", doc_id);
        super::insert(
            pool,
            &doc_id,
            user_id,
            None,
            "Test Payment",
            amount,
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
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, order.id);
        assert_eq!(found.user_id, uid);
        assert_eq!(found.amount, 1000);
        assert_eq!(found.currency, "USD");
        assert_eq!(found.status, PaymentStatus::Pending);
        assert_eq!(found.channel_id, ch_id);
    }

    #[tokio::test]
    async fn find_by_document_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 500).await;
        let found = super::find_by_document_id(&pool, &order.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, order.id);
    }

    #[tokio::test]
    async fn find_by_idempotency_key_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 500).await;
        let found = super::find_by_idempotency_key(&pool, &order.idempotency_key, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, order.id);
    }

    #[tokio::test]
    async fn find_by_provider_order_id_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 500).await;
        super::update_provider_order_id(&pool, order.id, "pi_test123", None, None)
            .await
            .unwrap();
        let found = super::find_by_provider_order_id(&pool, "pi_test123", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, order.id);
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
    async fn insert_sets_defaults() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        assert_eq!(order.status, PaymentStatus::Pending);
        assert_eq!(order.version, 1);
        assert!(order.paid_at.is_none());
        assert!(order.tenant_id.is_none());
        assert!(order.provider_order_id.is_none());
    }

    #[tokio::test]
    async fn update_status_to_paid() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        super::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, PaymentStatus::Paid);
        assert!(found.paid_at.is_some());
        assert_eq!(found.version, order.version + 1);
    }

    #[tokio::test]
    async fn update_status_to_cancelled() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        super::update_status(&pool, order.id, "cancelled", Some("cancelled_at"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, PaymentStatus::Cancelled);
        assert!(found.cancelled_at.is_some());
    }

    #[tokio::test]
    async fn update_provider_order_id_sets_field() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        super::update_provider_order_id(
            &pool,
            order.id,
            "pi_abc123",
            Some(r#"{"status":"requires_action"}"#),
            None,
        )
        .await
        .unwrap();
        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.provider_order_id.unwrap(), "pi_abc123");
        assert_eq!(
            found.provider_data.unwrap(),
            r#"{"status":"requires_action"}"#
        );
    }

    #[tokio::test]
    async fn find_by_user_paginated_works() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        for _ in 0..3 {
            seed_payment_order(&pool, uid, ch_id, 100).await;
        }
        let (items, total) = super::find_by_user_paginated(&pool, uid, None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.user_id == uid));
    }

    #[tokio::test]
    async fn find_all_admin_paginated_no_filter() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        for _ in 0..4 {
            seed_payment_order(&pool, uid, ch_id, 100).await;
        }
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, None)
            .await
            .unwrap();
        assert_eq!(total, 4);
        assert_eq!(items.len(), 4);
    }

    #[tokio::test]
    async fn find_all_admin_paginated_status_filter() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        for _ in 0..3 {
            let order = seed_payment_order(&pool, uid, ch_id, 100).await;
            super::update_status(&pool, order.id, "paid", Some("paid_at"), None)
                .await
                .unwrap();
        }
        seed_payment_order(&pool, uid, ch_id, 100).await;
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, Some("paid"))
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.status == PaymentStatus::Paid));
    }

    #[tokio::test]
    async fn find_all_admin_paginated_empty() {
        let pool = setup_pool().await;
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, None)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn full_lifecycle_pending_to_paid() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 2000).await;
        assert_eq!(order.status, PaymentStatus::Pending);

        super::update_provider_order_id(&pool, order.id, "pi_xyz", None, None)
            .await
            .unwrap();
        super::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();

        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, PaymentStatus::Paid);
        assert_eq!(found.provider_order_id.unwrap(), "pi_xyz");
        assert!(found.paid_at.is_some());
    }
}
