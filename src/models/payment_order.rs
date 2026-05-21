use serde::{Deserialize, Serialize};

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
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
        Disputed = "disputed",
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

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PaymentOrder {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub order_id: Option<String>,
    pub title: String,
    pub amount: i64,
    pub currency: String,
    pub channel_id: SnowflakeId,
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
    pub client_language: Option<String>,
    pub client_country: Option<String>,
    pub client_user_agent: Option<String>,
    pub channel_selected_by: Option<String>,
    pub metadata: Option<String>,
    pub paid_at: Option<Timestamp>,
    pub cancelled_at: Option<Timestamp>,
    pub expired_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    raisfast_derive::crud_find!(pool, "payment_orders", PaymentOrder, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_idempotency_key(
    pool: &crate::db::Pool,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    raisfast_derive::crud_find!(pool, "payment_orders", PaymentOrder, where: ("idempotency_key", key), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_provider_order_id(
    pool: &crate::db::Pool,
    provider_order_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentOrder>> {
    raisfast_derive::crud_find!(pool, "payment_orders", PaymentOrder, where: ("provider_order_id", provider_order_id), tenant: tenant_id).map_err(Into::into)
}

pub async fn find_by_user_paginated(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, PaymentOrder,
        table: "payment_orders",
        where: ("user_id", user_id),
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<PaymentOrder>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, PaymentOrder,
        table: "payment_orders",
        where: ["status" => status],
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePaymentOrderCmd,
    tenant_id: Option<&str>,
) -> AppResult<PaymentOrder> {
    let id = crate::utils::id::new_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "payment_orders",
        [
            "id" => id,
            "user_id" => cmd.user_id,
            "order_id" => &cmd.order_id,
            "title" => &cmd.title,
            "amount" => cmd.amount,
            "currency" => &cmd.currency,
            "channel_id" => cmd.channel_id,
            "provider" => &cmd.provider,
            "reference_type" => &cmd.reference_type,
            "reference_id" => &cmd.reference_id,
            "return_url" => &cmd.return_url,
            "idempotency_key" => &cmd.idempotency_key,
            "client_ip" => &cmd.client_ip,
            "client_language" => &cmd.client_language,
            "client_country" => &cmd.client_country,
            "client_user_agent" => &cmd.client_user_agent,
            "channel_selected_by" => &cmd.channel_selected_by,
            "metadata" => &cmd.metadata,
            "created_at" => &now,
            "updated_at" => &now
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

pub async fn update_provider_order_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    provider_order_id: &str,
    provider_data: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool, "payment_orders",
        bind: ["provider_order_id" => provider_order_id, "provider_data" => provider_data],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(())
}

pub async fn tx_update_status_cas(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    new_status: PaymentStatus,
    timestamp_col: Option<&str>,
    expected_status: PaymentStatus,
) -> AppResult<u64> {
    raisfast_derive::check_schema!("payment_orders", "status", "updated_at", "version", "id");
    let sql = if let Some(col) = timestamp_col {
        format!(
            "UPDATE payment_orders SET status = {}, {} = {}, updated_at = {}, version = version + 1 WHERE id = {} AND status = {}",
            Driver::ph(1),
            col,
            crate::db::Driver::now_fn(),
            crate::db::Driver::now_fn(),
            Driver::ph(2),
            Driver::ph(3)
        )
    } else {
        format!(
            "UPDATE payment_orders SET status = {}, updated_at = {}, version = version + 1 WHERE id = {} AND status = {}",
            Driver::ph(1),
            crate::db::Driver::now_fn(),
            Driver::ph(2),
            Driver::ph(3)
        )
    };
    let result = sqlx::query(&sql)
        .bind(new_status)
        .bind(id)
        .bind(expected_status)
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected())
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

    async fn seed_payment_order(
        pool: &crate::db::Pool,
        user_id: i64,
        channel_id: i64,
        amount: i64,
    ) -> PaymentOrder {
        let idem_key = format!("idem_{}", uuid::Uuid::now_v7());
        super::insert(
            pool,
            &crate::commands::CreatePaymentOrderCmd {
                user_id: SnowflakeId(user_id),
                order_id: None,
                title: "Test Payment".into(),
                amount,
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
        assert_eq!(found.user_id, SnowflakeId(uid));
        assert_eq!(found.amount, 1000);
        assert_eq!(found.currency, "USD");
        assert_eq!(found.status, PaymentStatus::Pending);
        assert_eq!(found.channel_id, SnowflakeId(ch_id));
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
            super::find_by_id(&pool, SnowflakeId(99999), None)
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
        assert_eq!(order.tenant_id, Some("default".to_string()));
        assert!(order.provider_order_id.is_none());
    }

    #[tokio::test]
    async fn update_status_to_paid() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let ch_id = seed_channel(&pool).await;
        let order = seed_payment_order(&pool, uid, ch_id, 1000).await;
        let mut tx = pool.begin().await.unwrap();
        let rows = super::tx_update_status_cas(
            &mut tx,
            order.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(rows, 1);
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
        let mut tx = pool.begin().await.unwrap();
        let rows = super::tx_update_status_cas(
            &mut tx,
            order.id,
            PaymentStatus::Cancelled,
            Some("cancelled_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(rows, 1);
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
        let (items, total) = super::find_by_user_paginated(&pool, SnowflakeId(uid), None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.user_id == SnowflakeId(uid)));
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
            let mut tx = pool.begin().await.unwrap();
            let rows = super::tx_update_status_cas(
                &mut tx,
                order.id,
                PaymentStatus::Paid,
                Some("paid_at"),
                PaymentStatus::Pending,
            )
            .await
            .unwrap();
            assert_eq!(rows, 1);
            tx.commit().await.unwrap();
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
        let mut tx = pool.begin().await.unwrap();
        let rows = super::tx_update_status_cas(
            &mut tx,
            order.id,
            PaymentStatus::Paid,
            Some("paid_at"),
            PaymentStatus::Pending,
        )
        .await
        .unwrap();
        assert_eq!(rows, 1);
        tx.commit().await.unwrap();

        let found = super::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, PaymentStatus::Paid);
        assert_eq!(found.provider_order_id.unwrap(), "pi_xyz");
        assert!(found.paid_at.is_some());
    }
}
