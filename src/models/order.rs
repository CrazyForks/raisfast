use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::tenant::tenant_filter_ph;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
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
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Order {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
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
    pub tax_amount: i64,
    pub coupon_id: Option<SnowflakeId>,
    pub shipping_address_id: Option<SnowflakeId>,
    pub billing_address_id: Option<SnowflakeId>,
    pub paid_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub cancelled_at: Option<Timestamp>,
    pub refunding_at: Option<Timestamp>,
    pub refunded_at: Option<Timestamp>,
    pub expired_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Order>> {
    raisfast_derive::crud_find!(pool, "orders", Order, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_order_no(
    pool: &crate::db::Pool,
    order_no: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Order>> {
    raisfast_derive::crud_find!(pool, "orders", Order, where: ("order_no", order_no), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_user_paginated(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Order>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Order,
        table: "orders",
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
    keyword: Option<&str>,
) -> AppResult<(Vec<Order>, i64)> {
    if let Some(kw) = keyword.filter(|k| !k.is_empty()) {
        let pattern = format!("%{kw}%");
        let offset = (page - 1).saturating_mul(page_size);
        let tf = tenant_filter_ph(tenant_id, 4);
        let status_filter = if status.is_some() {
            format!(" AND status = {}", crate::db::Driver::ph(3))
        } else {
            String::new()
        };
        let sql = format!(
            "SELECT * FROM orders WHERE (order_no LIKE {} OR buyer_name LIKE {}){}{tf} ORDER BY created_at DESC LIMIT {} OFFSET {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            status_filter,
            crate::db::Driver::ph(5),
            crate::db::Driver::ph(6),
        );
        let mut q = sqlx::query_as::<_, Order>(&sql)
            .bind(&pattern)
            .bind(&pattern);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(tid) = tenant_id {
            q = q.bind(tid);
        }
        let orders = q.bind(page_size).bind(offset).fetch_all(pool).await?;

        let count_sql = format!(
            "SELECT {} FROM orders WHERE (order_no LIKE {} OR buyer_name LIKE {}){}{tf}",
            Driver::cast_int("COUNT(*)"),
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2),
            status_filter,
        );
        let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql)
            .bind(&pattern)
            .bind(&pattern);
        if let Some(s) = status {
            cq = cq.bind(s);
        }
        if let Some(tid) = tenant_id {
            cq = cq.bind(tid);
        }
        let total = cq.fetch_one(pool).await?;
        return Ok((orders, total.0));
    }

    let result = raisfast_derive::crud_query_paged!(
        pool, Order,
        table: "orders",
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
    cmd: &crate::commands::CreateOrderCmd,
    tenant_id: Option<&str>,
) -> AppResult<Order> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "orders",
        [
            "id" => id,
            "user_id" => cmd.user_id,
            "order_no" => &cmd.order_no,
            "subtotal" => cmd.subtotal,
            "discount_amount" => cmd.discount_amount,
            "shipping_amount" => cmd.shipping_amount,
            "total_amount" => cmd.total_amount,
            "currency" => &cmd.currency,
            "buyer_name" => &cmd.buyer_name,
            "buyer_phone" => &cmd.buyer_phone,
            "buyer_email" => &cmd.buyer_email,
            "shipping_address" => &cmd.shipping_address,
            "remark" => &cmd.remark,
            "tax_amount" => cmd.tax_amount,
            "coupon_id" => cmd.coupon_id,
            "shipping_address_id" => cmd.shipping_address_id,
            "billing_address_id" => cmd.billing_address_id,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id).await?.ok_or_else(|| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
            "order not found after insert"
        ))
    })
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
    id: SnowflakeId,
    status: &str,
    timestamp_col: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!("orders", "status", "updated_at", "id", "tenant_id");
    let sql = if let Some(col) = timestamp_col {
        validate_timestamp_col(col)?;
        format!(
            "UPDATE orders SET status = {}, {} = {}, updated_at = {} WHERE id = {}{}",
            Driver::ph(1),
            col,
            crate::db::Driver::now_fn(),
            crate::db::Driver::now_fn(),
            Driver::ph(2),
            tenant_filter_ph(tenant_id, 3)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = {} WHERE id = {}{}",
            Driver::ph(1),
            crate::db::Driver::now_fn(),
            Driver::ph(2),
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
    id: SnowflakeId,
    tracking_no: Option<&str>,
    carrier: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool, "orders",
        bind: [
            "status" => OrderStatus::Shipped.as_str(),
            "tracking_no" => tracking_no,
            "carrier" => carrier,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(())
}

pub async fn update_admin_remark(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    admin_remark: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool, "orders",
        bind: ["admin_remark" => admin_remark],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(())
}

pub async fn update_delivery_data(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    delivery_data: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool, "orders",
        bind: ["delivery_data" => delivery_data],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(())
}

pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::CreateOrderCmd,
    tenant_id: Option<&str>,
) -> AppResult<Order> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        &mut *tx,
        "orders",
        [
            "id" => id,
            "user_id" => cmd.user_id,
            "order_no" => &cmd.order_no,
            "subtotal" => cmd.subtotal,
            "discount_amount" => cmd.discount_amount,
            "shipping_amount" => cmd.shipping_amount,
            "total_amount" => cmd.total_amount,
            "currency" => &cmd.currency,
            "buyer_name" => &cmd.buyer_name,
            "buyer_phone" => &cmd.buyer_phone,
            "buyer_email" => &cmd.buyer_email,
            "shipping_address" => &cmd.shipping_address,
            "remark" => &cmd.remark,
            "tax_amount" => cmd.tax_amount,
            "coupon_id" => cmd.coupon_id,
            "shipping_address_id" => cmd.shipping_address_id,
            "billing_address_id" => cmd.billing_address_id,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    let sql = if tenant_id.is_some() {
        format!(
            "SELECT * FROM orders WHERE id = {} AND tenant_id = {}",
            Driver::ph(1),
            Driver::ph(2)
        )
    } else {
        format!("SELECT * FROM orders WHERE id = {}", Driver::ph(1))
    };
    raisfast_derive::crud_query!(&mut *tx, Order, &sql, [id], fetch_one, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn tx_update_status_cas(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    new_status: OrderStatus,
    timestamp_col: Option<&str>,
    expected_status: OrderStatus,
) -> AppResult<u64> {
    raisfast_derive::check_schema!("orders", "status", "updated_at", "id");
    let sql = if let Some(col) = timestamp_col {
        validate_timestamp_col(col)?;
        format!(
            "UPDATE orders SET status = {}, {} = {}, updated_at = {} WHERE id = {} AND status = {}",
            Driver::ph(1),
            col,
            crate::db::Driver::now_fn(),
            crate::db::Driver::now_fn(),
            Driver::ph(2),
            Driver::ph(3)
        )
    } else {
        format!(
            "UPDATE orders SET status = {}, updated_at = {} WHERE id = {} AND status = {}",
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

pub async fn tx_update_shipped(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    tracking_no: Option<&str>,
    carrier: Option<&str>,
) -> AppResult<u64> {
    let result = raisfast_derive::crud_update!(&mut *tx, "orders",
        bind: [
            "status" => OrderStatus::Shipped.as_str(),
            "tracking_no" => tracking_no,
            "carrier" => carrier
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: AND(("id", id), ("status", OrderStatus::Paid.as_str()))
    )?;
    Ok(result.rows_affected())
}

pub async fn get_stats_query(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<crate::dto::OrderStatsResponse> {
    raisfast_derive::check_schema!(
        "orders",
        "status",
        "tenant_id",
        "total_amount",
        "created_at"
    );
    let sql = format!(
        "SELECT status, {} as cnt FROM orders WHERE 1=1{} GROUP BY status",
        Driver::cast_int("COUNT(*)"),
        tenant_filter_ph(tenant_id, 1)
    );
    let rows =
        raisfast_derive::crud_query!(pool, (String, i64), &sql, [], fetch_all, tenant: tenant_id)?;

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
        "SELECT {} FROM orders WHERE status = 'completed'{}",
        Driver::cast_int("COALESCE(SUM(total_amount), 0)"),
        tenant_filter_ph(tenant_id, 1)
    );
    let (total_revenue,) =
        raisfast_derive::crud_query!(pool, (i64,), &rev_sql, [], fetch_one, tenant: tenant_id)?;

    let today_filter = format!(
        " AND {} = {}",
        Driver::date_trunc_day("created_at"),
        Driver::current_date()
    );
    let today_sql = format!(
        "SELECT {} FROM orders WHERE 1=1{}{}",
        Driver::cast_int("COUNT(*)"),
        tenant_filter_ph(tenant_id, 1),
        today_filter
    );
    let (today_orders,) =
        raisfast_derive::crud_query!(pool, (i64,), &today_sql, [], fetch_one, tenant: tenant_id)?;

    let today_rev_sql = format!(
        "SELECT {} FROM orders WHERE status = 'completed'{}{}",
        Driver::cast_int("COALESCE(SUM(total_amount), 0)"),
        tenant_filter_ph(tenant_id, 1),
        today_filter
    );
    let (today_revenue,) = raisfast_derive::crud_query!(pool, (i64,), &today_rev_sql, [], fetch_one, tenant: tenant_id)?;

    Ok(crate::dto::OrderStatsResponse {
        total_orders,
        pending_orders,
        paid_orders,
        completed_orders,
        total_revenue,
        today_orders,
        today_revenue,
    })
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

    async fn seed_order(pool: &crate::db::Pool, user_id: i64) -> Order {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        super::insert(
            pool,
            &crate::commands::CreateOrderCmd {
                user_id: SnowflakeId(user_id),
                order_no,
                subtotal: 1000,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: 1000,
                currency: "CNY".into(),
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    async fn get_status(pool: &crate::db::Pool, id: SnowflakeId) -> String {
        let (s,): (String,) = sqlx::query_as("SELECT status FROM orders WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        s
    }

    async fn get_optional_field(
        pool: &crate::db::Pool,
        id: SnowflakeId,
        col: &str,
    ) -> Option<String> {
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
        assert_eq!(found.user_id, SnowflakeId(uid));
        assert_eq!(found.total_amount, 1000);
        assert_eq!(found.status, OrderStatus::Pending);
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
        let o = seed_order(&pool, uid).await;
        assert_eq!(o.status, OrderStatus::Pending);
        assert_eq!(o.subtotal, 1000);
        assert_eq!(o.discount_amount, 0);
        assert_eq!(o.shipping_amount, 0);
        assert_eq!(o.currency, "CNY");
        assert!(o.paid_at.is_none());
        assert_eq!(o.tenant_id, Some("default".to_string()));
    }

    #[tokio::test]
    async fn insert_with_buyer_info() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let order_no = format!(
            "ORD-{}",
            &uuid::Uuid::now_v7().to_string().replace('-', "")[..16]
        );
        let o = super::insert(
            &pool,
            &crate::commands::CreateOrderCmd {
                user_id: SnowflakeId(uid),
                order_no,
                subtotal: 500,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: 500,
                currency: "USD".into(),
                buyer_name: Some("John".into()),
                buyer_phone: Some("1234567890".into()),
                buyer_email: Some("john@test.com".into()),
                shipping_address: Some("123 Main St".into()),
                remark: Some("please be careful".into()),
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
            },
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

        let (items, total) = super::find_by_user_paginated(&pool, SnowflakeId(uid1), None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.user_id == SnowflakeId(uid1)));
    }

    #[tokio::test]
    async fn find_by_user_paginated_paging() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        for _ in 0..5 {
            seed_order(&pool, uid).await;
        }
        let (p1, total) = super::find_by_user_paginated(&pool, SnowflakeId(uid), None, 1, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(p1.len(), 3);
        let (p2, _) = super::find_by_user_paginated(&pool, SnowflakeId(uid), None, 2, 3)
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
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, None, None)
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

        let (items, total) =
            super::find_all_admin_paginated(&pool, None, 1, 10, Some("paid"), None)
                .await
                .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|o| o.status == OrderStatus::Paid));
    }

    #[tokio::test]
    async fn find_all_admin_paginated_empty() {
        let pool = setup_pool().await;
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, None, None)
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
