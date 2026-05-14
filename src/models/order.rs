use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

define_enum!(
    OrderStatus {
        Pending = "pending",
        Paid = "paid",
        Shipped = "shipped",
        Completed = "completed",
        Cancelled = "cancelled",
        Refunding = "refunding",
        Refunded = "refunded",
        Expired = "expired",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Order {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub user_id: i64,
    pub order_no: String,
    pub subtotal: i64,
    pub discount_amount: i64,
    pub shipping_amount: i64,
    pub total_amount: i64,
    pub currency: String,
    pub status: OrderStatus,
    pub buyer_name: Option<String>,
    pub buyer_phone: Option<String>,
    pub buyer_email: Option<String>,
    pub shipping_address: Option<String>,
    pub tracking_no: Option<String>,
    pub carrier: Option<String>,
    pub remark: Option<String>,
    pub admin_remark: Option<String>,
    pub delivery_data: Option<String>,
    pub paid_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub cancelled_at: Option<Timestamp>,
    pub refunding_at: Option<Timestamp>,
    pub refunded_at: Option<Timestamp>,
    pub expired_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Order {
    required { id, document_id, user_id, order_no, subtotal, discount_amount, shipping_amount, total_amount, currency, status, created_at, updated_at }
    optional { buyer_name, buyer_phone, buyer_email, shipping_address, tracking_no, carrier, remark, admin_remark, delivery_data, paid_at, completed_at, cancelled_at, refunding_at, refunded_at, expired_at }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Order>> {
    let sql = format!(
        "SELECT * FROM orders WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Order>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Order>> {
    let sql = format!(
        "SELECT * FROM orders WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Order>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_order_no(
    pool: &crate::db::Pool,
    order_no: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Order>> {
    let sql = format!(
        "SELECT * FROM orders WHERE order_no = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Order>(&sql).bind(order_no);
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
) -> AppResult<(Vec<Order>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 2);
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM orders WHERE user_id = {}{}",
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
        "SELECT * FROM orders WHERE user_id = {}{} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        ph(1),
        tenant_filter_ph(tenant_id, 2),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, Order>(&sql).bind(user_id);
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
) -> AppResult<(Vec<Order>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 1);
    let has_tenant = tenant_id.is_some();
    let status_ph_idx = if has_tenant { 2 } else { 1 };
    let (count_sql, data_sql_base) = if let Some(_s) = status {
        (
            format!(
                "SELECT COUNT(*) as count FROM orders WHERE status = {}{}",
                ph(status_ph_idx),
                tenant_ph
            ),
            format!(
                "SELECT * FROM orders WHERE status = {}{} ORDER BY created_at DESC",
                ph(status_ph_idx),
                tenant_ph
            ),
        )
    } else {
        (
            format!(
                "SELECT COUNT(*) as count FROM orders WHERE 1=1{}",
                tenant_ph
            ),
            format!(
                "SELECT * FROM orders WHERE 1=1{} ORDER BY created_at DESC",
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
    let mut q2 = sqlx::query_as::<_, Order>(&sql);
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
    order_no: &str,
    subtotal: i64,
    discount_amount: i64,
    shipping_amount: i64,
    total_amount: i64,
    currency: &str,
    buyer_name: Option<&str>,
    buyer_phone: Option<&str>,
    buyer_email: Option<&str>,
    shipping_address: Option<&str>,
    remark: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Order> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO orders (document_id, tenant_id, user_id, order_no, subtotal, discount_amount, shipping_amount, total_amount, currency, buyer_name, buyer_phone, buyer_email, shipping_address, remark, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
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
                .bind(tid)
                .bind(user_id)
                .bind(order_no)
                .bind(subtotal)
                .bind(discount_amount)
                .bind(shipping_amount)
                .bind(total_amount)
                .bind(currency)
                .bind(buyer_name)
                .bind(buyer_phone)
                .bind(buyer_email)
                .bind(shipping_address)
                .bind(remark)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO orders (document_id, user_id, order_no, subtotal, discount_amount, shipping_amount, total_amount, currency, buyer_name, buyer_phone, buyer_email, shipping_address, remark, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
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
                ph(13)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(user_id)
                .bind(order_no)
                .bind(subtotal)
                .bind(discount_amount)
                .bind(shipping_amount)
                .bind(total_amount)
                .bind(currency)
                .bind(buyer_name)
                .bind(buyer_phone)
                .bind(buyer_email)
                .bind(shipping_address)
                .bind(remark)
                .execute(pool)
                .await?;
        }
    }
    find_by_document_id(pool, document_id, tenant_id)
        .await?
        .ok_or_else(|| crate::errors::app_error::AppError::Internal(anyhow::anyhow!("order not found after insert")))
}

fn validate_timestamp_col(col: &str) -> AppResult<()> {
    let allowed = [
        "paid_at",
        "completed_at",
        "cancelled_at",
        "refunding_at",
        "refunded_at",
        "expired_at",
    ];
    if allowed.contains(&col) {
        Ok(())
    } else {
        Err(crate::errors::app_error::AppError::Internal(
            anyhow::anyhow!("invalid timestamp column: {col}"),
        ))
    }
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: i64,
    status: &str,
    timestamp_col: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = if let Some(col) = timestamp_col {
        validate_timestamp_col(col)?;
        format!(
            "UPDATE orders SET status = {}, {} = datetime('now'), updated_at = datetime('now') WHERE id = {}{}",
            ph(1), col, ph(2), tenant_filter_ph(tenant_id, 3)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = datetime('now') WHERE id = {}{}",
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

pub async fn update_shipped(
    pool: &crate::db::Pool,
    id: i64,
    tracking_no: Option<&str>,
    carrier: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE orders SET status = {}, tracking_no = {}, carrier = {}, updated_at = datetime('now') WHERE id = {}{}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        tenant_filter_ph(tenant_id, 5)
    );
    let mut q = sqlx::query(&sql)
        .bind(OrderStatus::Shipped.as_str())
        .bind(tracking_no)
        .bind(carrier)
        .bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn update_admin_remark(
    pool: &crate::db::Pool,
    id: i64,
    admin_remark: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE orders SET admin_remark = {}, updated_at = datetime('now') WHERE id = {}{}",
        ph(1),
        ph(2),
        tenant_filter_ph(tenant_id, 3)
    );
    let mut q = sqlx::query(&sql).bind(admin_remark).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn update_delivery_data(
    pool: &crate::db::Pool,
    id: i64,
    delivery_data: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE orders SET delivery_data = {}, updated_at = datetime('now') WHERE id = {}{}",
        ph(1),
        ph(2),
        tenant_filter_ph(tenant_id, 3)
    );
    let mut q = sqlx::query(&sql).bind(delivery_data).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(pool).await?;
    Ok(())
}

pub async fn tx_find_id_by_document_id(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
) -> AppResult<Option<i64>> {
    let sql = format!("SELECT id FROM orders WHERE document_id = {}", ph(1));
    let result: Option<(i64,)> = sqlx::query_as(&sql)
        .bind(document_id)
        .fetch_optional(&mut *tx)
        .await?;
    Ok(result.map(|(id,)| id))
}

pub async fn tx_find_by_document_id(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
) -> AppResult<Option<Order>> {
    let sql = format!("SELECT * FROM orders WHERE document_id = {}", ph(1));
    sqlx::query_as::<_, Order>(&sql)
        .bind(document_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(Into::into)
}

pub async fn tx_update_status(
    tx: &mut crate::db::pool::DbConnection,
    id: i64,
    status: OrderStatus,
    timestamp_col: Option<&str>,
) -> AppResult<()> {
    let sql = if let Some(col) = timestamp_col {
        validate_timestamp_col(col)?;
        format!(
            "UPDATE orders SET status = {}, {} = datetime('now'), updated_at = datetime('now') WHERE id = {}",
            ph(1), col, ph(2)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = datetime('now') WHERE id = {}",
            ph(1),
            ph(2)
        )
    };
    sqlx::query(&sql)
        .bind(status)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    document_id: &str,
    user_id: i64,
    order_no: &str,
    subtotal: i64,
    discount_amount: i64,
    shipping_amount: i64,
    total_amount: i64,
    currency: &str,
    buyer_name: Option<&str>,
    buyer_phone: Option<&str>,
    buyer_email: Option<&str>,
    shipping_address: Option<&str>,
    remark: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Order> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO orders (document_id, tenant_id, user_id, order_no, subtotal, discount_amount, shipping_amount, total_amount, currency, buyer_name, buyer_phone, buyer_email, shipping_address, remark, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11), ph(12), ph(13), ph(14)
            );
            sqlx::query(&sql)
                .bind(document_id).bind(tid).bind(user_id).bind(order_no)
                .bind(subtotal).bind(discount_amount).bind(shipping_amount).bind(total_amount)
                .bind(currency).bind(buyer_name).bind(buyer_phone).bind(buyer_email)
                .bind(shipping_address).bind(remark)
                .execute(&mut *tx).await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO orders (document_id, user_id, order_no, subtotal, discount_amount, shipping_amount, total_amount, currency, buyer_name, buyer_phone, buyer_email, shipping_address, remark, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1), ph(2), ph(3), ph(4), ph(5), ph(6), ph(7), ph(8), ph(9), ph(10), ph(11), ph(12), ph(13)
            );
            sqlx::query(&sql)
                .bind(document_id).bind(user_id).bind(order_no)
                .bind(subtotal).bind(discount_amount).bind(shipping_amount).bind(total_amount)
                .bind(currency).bind(buyer_name).bind(buyer_phone).bind(buyer_email)
                .bind(shipping_address).bind(remark)
                .execute(&mut *tx).await?;
        }
    }
    let sql = if tenant_id.is_some() {
        format!("SELECT * FROM orders WHERE document_id = {} AND tenant_id = {}", ph(1), ph(2))
    } else {
        format!("SELECT * FROM orders WHERE document_id = {}", ph(1))
    };
    let mut q = sqlx::query_as::<_, Order>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(&mut *tx).await.map_err(Into::into)
}

pub async fn tx_update_status_cas(
    tx: &mut crate::db::pool::DbConnection,
    id: i64,
    new_status: OrderStatus,
    timestamp_col: Option<&str>,
    expected_status: OrderStatus,
) -> AppResult<u64> {
    let sql = if let Some(col) = timestamp_col {
        validate_timestamp_col(col)?;
        format!(
            "UPDATE orders SET status = {}, {} = datetime('now'), updated_at = datetime('now') WHERE id = {} AND status = {}",
            ph(1), col, ph(2), ph(3)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = datetime('now') WHERE id = {} AND status = {}",
            ph(1), ph(2), ph(3)
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

pub async fn tx_update_shipped(
    tx: &mut crate::db::pool::DbConnection,
    id: i64,
    tracking_no: Option<&str>,
    carrier: Option<&str>,
) -> AppResult<u64> {
    let sql = format!(
        "UPDATE orders SET status = {}, tracking_no = {}, carrier = {}, updated_at = datetime('now') WHERE id = {} AND status = {}",
        ph(1), ph(2), ph(3), ph(4), ph(5)
    );
    let result = sqlx::query(&sql)
        .bind(OrderStatus::Shipped.as_str())
        .bind(tracking_no)
        .bind(carrier)
        .bind(id)
        .bind(OrderStatus::Paid.as_str())
        .execute(&mut *tx)
        .await?;
    Ok(result.rows_affected())
}

pub async fn get_stats_query(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<crate::dto::OrderStatsResponse> {
    let sql = format!(
        "SELECT status, COUNT(*) as cnt FROM orders WHERE 1=1{} GROUP BY status",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut q = sqlx::query_as::<_, (String, i64)>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let rows = q.fetch_all(pool).await?;

    let mut total_orders: i64 = 0;
    let mut pending_orders: i64 = 0;
    let mut paid_orders: i64 = 0;
    let mut completed_orders: i64 = 0;
    for (status, cnt) in &rows {
        total_orders += cnt;
        match status.as_str() {
            "pending" => pending_orders = *cnt,
            "paid" => paid_orders = *cnt,
            "completed" => completed_orders = *cnt,
            _ => {}
        }
    }

    let rev_sql = format!(
        "SELECT COALESCE(SUM(total_amount), 0) FROM orders WHERE status = 'completed'{}",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut rq = sqlx::query_as::<_, (i64,)>(&rev_sql);
    if let Some(tid) = tenant_id {
        rq = rq.bind(tid);
    }
    let (total_revenue,) = rq.fetch_one(pool).await?;

    Ok(crate::dto::OrderStatsResponse {
        total_orders,
        pending_orders,
        paid_orders,
        completed_orders,
        total_revenue,
    })
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

    async fn seed_order(pool: &crate::db::Pool, user_id: i64) -> Order {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        super::insert(
            pool, &doc_id, user_id, &order_no, 1000, 0, 0, 1000, "CNY", None, None, None, None,
            None, None,
        )
        .await
        .unwrap()
        .into()
    }

    async fn get_status(pool: &crate::db::Pool, id: i64) -> String {
        let (s,): (String,) = sqlx::query_as("SELECT status FROM orders WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        s
    }

    async fn get_optional_field(pool: &crate::db::Pool, id: i64, col: &str) -> Option<String> {
        let sql = format!("SELECT {col} FROM orders WHERE id = ?");
        let (v,): (Option<String>,) = sqlx::query_as(&sql).bind(id).fetch_one(pool).await.unwrap();
        v
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(found.id, o.id);
        assert_eq!(found.user_id, uid);
        assert_eq!(found.total_amount, 1000);
        assert_eq!(found.status, OrderStatus::Pending);
    }

    #[tokio::test]
    async fn find_by_document_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        let found = super::find_by_document_id(&pool, &o.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, o.id);
    }

    #[tokio::test]
    async fn find_by_order_no() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        let found = super::find_by_order_no(&pool, &o.order_no, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, o.id);
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
    async fn find_by_document_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_document_id(&pool, "nonexistent", None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn insert_sets_defaults() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        assert_eq!(o.status, OrderStatus::Pending);
        assert_eq!(o.subtotal, 1000);
        assert_eq!(o.discount_amount, 0);
        assert_eq!(o.shipping_amount, 0);
        assert_eq!(o.currency, "CNY");
        assert!(o.paid_at.is_none());
        assert!(o.tenant_id.is_none());
    }

    #[tokio::test]
    async fn insert_with_buyer_info() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let doc_id = uuid::Uuid::now_v7().to_string();
        let order_no = format!(
            "ORD-{}",
            &uuid::Uuid::now_v7().to_string().replace('-', "")[..16]
        );
        let o = super::insert(
            &pool,
            &doc_id,
            uid,
            &order_no,
            500,
            0,
            0,
            500,
            "USD",
            Some("John"),
            Some("1234567890"),
            Some("john@test.com"),
            Some("123 Main St"),
            Some("please be careful"),
            None,
        )
        .await
        .unwrap();
        assert_eq!(o.buyer_name.unwrap(), "John");
        assert_eq!(o.buyer_phone.unwrap(), "1234567890");
        assert_eq!(o.buyer_email.unwrap(), "john@test.com");
        assert_eq!(o.shipping_address.unwrap(), "123 Main St");
        assert_eq!(o.remark.unwrap(), "please be careful");
        assert_eq!(o.currency, "USD");
    }

    #[tokio::test]
    async fn update_status_to_paid() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_status(&pool, o.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, OrderStatus::Paid);
        assert!(found.paid_at.is_some());
    }

    #[tokio::test]
    async fn update_status_to_cancelled() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_status(&pool, o.id, "cancelled", Some("cancelled_at"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, OrderStatus::Cancelled);
        assert!(found.cancelled_at.is_some());
    }

    #[tokio::test]
    async fn update_status_without_timestamp() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_status(&pool, o.id, "expired", None, None)
            .await
            .unwrap();
        let status = get_status(&pool, o.id).await;
        assert_eq!(status, "expired");
        let expired_at = get_optional_field(&pool, o.id, "expired_at").await;
        assert!(expired_at.is_none());
    }

    #[tokio::test]
    async fn update_shipped() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_status(&pool, o.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        super::update_shipped(&pool, o.id, Some("TRACK123"), Some("FedEx"), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, OrderStatus::Shipped);
        assert_eq!(found.tracking_no.unwrap(), "TRACK123");
        assert_eq!(found.carrier.unwrap(), "FedEx");
    }

    #[tokio::test]
    async fn update_admin_remark() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_admin_remark(&pool, o.id, "fraud suspected", None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(found.admin_remark.unwrap(), "fraud suspected");
    }

    #[tokio::test]
    async fn update_delivery_data() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        super::update_delivery_data(&pool, o.id, r#"{"tracking_url":"https://t.co/abc"}"#, None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, o.id, None).await.unwrap().unwrap();
        assert_eq!(
            found.delivery_data.unwrap(),
            r#"{"tracking_url":"https://t.co/abc"}"#
        );
    }

    #[tokio::test]
    async fn find_by_user_paginated() {
        let pool = setup_pool().await;
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        for _ in 0..3 {
            seed_order(&pool, uid1).await;
        }
        seed_order(&pool, uid2).await;

        let (items, total) = super::find_by_user_paginated(&pool, uid1, None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.user_id == uid1));
    }

    #[tokio::test]
    async fn find_by_user_paginated_paging() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        for _ in 0..5 {
            seed_order(&pool, uid).await;
        }
        let (p1, total) = super::find_by_user_paginated(&pool, uid, None, 1, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(p1.len(), 3);
        let (p2, _) = super::find_by_user_paginated(&pool, uid, None, 2, 3)
            .await
            .unwrap();
        assert_eq!(p2.len(), 2);
    }

    #[tokio::test]
    async fn find_all_admin_paginated_no_filter() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        for _ in 0..4 {
            seed_order(&pool, uid).await;
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
        for _ in 0..3 {
            let o = seed_order(&pool, uid).await;
            super::update_status(&pool, o.id, "paid", Some("paid_at"), None)
                .await
                .unwrap();
        }
        seed_order(&pool, uid).await;

        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, Some("paid"))
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.status == OrderStatus::Paid));
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
    async fn full_lifecycle_pending_to_completed() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;
        assert_eq!(get_status(&pool, o.id).await, "pending");

        super::update_status(&pool, o.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        assert_eq!(get_status(&pool, o.id).await, "paid");

        super::update_shipped(&pool, o.id, Some("TRK001"), Some("UPS"), None)
            .await
            .unwrap();
        assert_eq!(get_status(&pool, o.id).await, "shipped");

        super::update_status(&pool, o.id, "completed", Some("completed_at"), None)
            .await
            .unwrap();
        assert_eq!(get_status(&pool, o.id).await, "completed");
    }

    #[tokio::test]
    async fn lifecycle_pending_to_refunded() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let o = seed_order(&pool, uid).await;

        super::update_status(&pool, o.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        super::update_status(&pool, o.id, "refunding", Some("refunding_at"), None)
            .await
            .unwrap();
        assert_eq!(get_status(&pool, o.id).await, "refunding");

        super::update_status(&pool, o.id, "refunded", Some("refunded_at"), None)
            .await
            .unwrap();
        assert_eq!(get_status(&pool, o.id).await, "refunded");
    }
}
