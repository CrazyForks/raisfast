use crate::dto::{CreateOrderRequest, ShipOrderRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::order::{Order, OrderStatus};
use crate::models::order_item::{InsertOrderItem, OrderItem};
use crate::models::product::ProductStatus;
use crate::repositories::{OrderRepository, ProductRepository};

const MAX_ITEMS_PER_ORDER: usize = 100;
const MAX_QUANTITY: i64 = 10000;

pub async fn create_order(
    pool: &crate::db::Pool,
    product_repo: &dyn ProductRepository,
    _order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    user_id: i64,
    req: CreateOrderRequest,
) -> AppResult<Order> {
    if req.items.is_empty() {
        return Err(AppError::BadRequest("items_empty".into()));
    }
    if req.items.len() > MAX_ITEMS_PER_ORDER {
        return Err(AppError::BadRequest("too_many_items".into()));
    }

    let mut order_items_data: Vec<(i64, i64, crate::models::product::Product)> = Vec::new();
    let mut subtotal: i64 = 0;

    for item in &req.items {
        if item.quantity > MAX_QUANTITY {
            return Err(AppError::BadRequest("quantity_exceeds_limit".into()));
        }
        let product = product_repo
            .find_by_document_id(&item.product_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        if product.status != ProductStatus::Active {
            return Err(AppError::BadRequest("product_not_active".into()));
        }

        let line_total = product
            .price
            .checked_mul(item.quantity)
            .ok_or_else(|| AppError::BadRequest("line_total_overflow".into()))?;
        subtotal = subtotal
            .checked_add(line_total)
            .ok_or_else(|| AppError::BadRequest("subtotal_overflow".into()))?;
        order_items_data.push((item.quantity, line_total, product));
    }

    let document_id = uuid::Uuid::now_v7().to_string();
    let uuid_str = uuid::Uuid::now_v7().to_string().replace('-', "");
    let order_no = format!("ORD-{}", &uuid_str[..16]);

    let currency = req.currency.as_deref().unwrap_or("CNY");
    let total_amount = subtotal;

    let order = crate::in_transaction!(pool, tx, {
        let order = crate::models::order::tx_insert(
            &mut tx,
            &document_id,
            user_id,
            &order_no,
            subtotal,
            0,
            0,
            total_amount,
            currency,
            req.buyer_name.as_deref(),
            req.buyer_phone.as_deref(),
            req.buyer_email.as_deref(),
            req.shipping_address.as_deref(),
            req.remark.as_deref(),
            auth.tenant_id(),
        )
        .await?;

        let mut items = Vec::new();
        for (quantity, line_total, product) in &order_items_data {
            items.push(InsertOrderItem {
                document_id: uuid::Uuid::now_v7().to_string(),
                order_id: order.id,
                product_id: Some(product.id),
                title: product.title.clone(),
                description: product.description.clone(),
                unit_price: product.price,
                quantity: *quantity,
                subtotal: *line_total,
                cover_url: product.cover_url.clone(),
                attributes: product.attributes.clone(),
            });
        }
        crate::models::order_item::tx_insert_batch(&mut tx, items, auth.tenant_id()).await?;

        Ok(order)
    })?;

    Ok(order)
}

pub async fn cancel_order(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
    user_id: i64,
) -> AppResult<()> {
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.user_id != user_id {
        return Err(AppError::Forbidden);
    }
    if order.status != OrderStatus::Pending {
        return Err(AppError::BadRequest("only_pending_can_cancel".into()));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Cancelled.as_str(),
                Some("cancelled_at"),
                OrderStatus::Pending.as_str(),
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result
}

pub async fn mark_paid(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
) -> AppResult<Order> {
    auth.ensure_admin()?;
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.status != OrderStatus::Pending {
        return Err(AppError::BadRequest("only_pending_can_pay".into()));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Paid.as_str(),
                Some("paid_at"),
                OrderStatus::Pending.as_str(),
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result?;

    order_repo
        .find_by_id(order.id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))
}

pub async fn ship_order(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
    req: &ShipOrderRequest,
) -> AppResult<()> {
    auth.ensure_admin()?;
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.status != OrderStatus::Paid {
        return Err(AppError::BadRequest("only_paid_can_ship".into()));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let rows = crate::models::order::tx_update_shipped(
                &mut tx,
                order.id,
                req.tracking_no.as_deref(),
                req.carrier.as_deref(),
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result
}

pub async fn confirm_receipt(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
    user_id: i64,
) -> AppResult<()> {
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.user_id != user_id {
        return Err(AppError::Forbidden);
    }
    if order.status != OrderStatus::Shipped {
        return Err(AppError::BadRequest("only_shipped_can_confirm".into()));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Completed.as_str(),
                Some("completed_at"),
                OrderStatus::Shipped.as_str(),
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result
}

pub async fn refund_order(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
) -> AppResult<()> {
    auth.ensure_admin()?;
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.status != OrderStatus::Paid && order.status != OrderStatus::Shipped {
        return Err(AppError::BadRequest(
            "only_paid_or_shipped_can_refund".into(),
        ));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let expected = order.status.as_str();
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Refunding.as_str(),
                Some("refunding_at"),
                expected,
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result
}

pub async fn admin_cancel(
    pool: &crate::db::Pool,
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
) -> AppResult<()> {
    auth.ensure_admin()?;
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;

    if order.status != OrderStatus::Pending && order.status != OrderStatus::Paid {
        return Err(AppError::BadRequest("only_pending_or_paid_can_admin_cancel".into()));
    }

    let result: Result<(), AppError> = async {
        crate::in_transaction!(pool, tx, {
            let expected = order.status.as_str();
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Cancelled.as_str(),
                Some("cancelled_at"),
                expected,
            )
            .await?;
            if rows == 0 {
                return Err(AppError::BadRequest("concurrent_status_change".into()));
            }
            Ok(())
        })
    }
    .await;
    result
}

pub async fn get_order(
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
) -> AppResult<(Order, Vec<OrderItem>)> {
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;
    if auth.role() != "admin" {
        let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
        if order.user_id != user_int_id {
            return Err(AppError::Forbidden);
        }
    }
    let items = order_repo
        .find_items_by_order_id(order.id, auth.tenant_id())
        .await?;
    Ok((order, items))
}

pub async fn list_user_orders(
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    user_id: i64,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Order>, i64)> {
    order_repo
        .find_by_user_paginated(user_id, auth.tenant_id(), page, page_size)
        .await
}

pub async fn list_admin_orders(
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<Order>, i64)> {
    auth.ensure_admin()?;
    order_repo
        .find_all_admin_paginated(auth.tenant_id(), page, page_size, status)
        .await
}

pub async fn update_admin_remark(
    order_repo: &dyn OrderRepository,
    auth: &AuthUser,
    order_id: &str,
    admin_remark: &str,
) -> AppResult<()> {
    auth.ensure_admin()?;
    let order = order_repo
        .find_by_document_id(order_id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("order"))?;
    order_repo
        .update_admin_remark(order.id, admin_remark, auth.tenant_id())
        .await
}

pub async fn get_stats(
    pool: &crate::db::Pool,
    auth: &AuthUser,
) -> AppResult<crate::dto::OrderStatsResponse> {
    auth.ensure_admin()?;
    crate::models::order::get_stats_query(pool, auth.tenant_id()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{CreateOrderItemRequest, ShipOrderRequest};
    use crate::repositories::sqlx_order::{SqlxOrderRepository, SqlxProductRepository};

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some("u1".to_string()),
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    fn auth_with_id(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(format!("u{user_int_id}")),
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            None,
        )
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

    async fn seed_active_product(
        pool: &crate::db::Pool,
        title: &str,
        price: i64,
    ) -> crate::models::product::Product {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let p = crate::models::product::insert(
            pool, &doc_id, None, title, None, None, "custom", "digital", None, None, price, "CNY",
            None, 0, None, None, None, None, None, "piece", 1, None, 0, None, None, None,
        )
        .await
        .unwrap();
        sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
            .bind(p.id)
            .execute(pool)
            .await
            .unwrap();
        crate::models::product::find_by_id(pool, p.id, None)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn create_order_basic() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let order = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: prod.document_id.clone(),
                    quantity: 2,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(order.user_id, uid);
        assert_eq!(order.subtotal, 2000);
        assert_eq!(order.total_amount, 2000);
        assert_eq!(order.status, OrderStatus::Pending);
        assert!(order.order_no.starts_with("ORD-"));

        let items = order_repo
            .find_items_by_order_id(order.id, None)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Widget");
        assert_eq!(items[0].unit_price, 1000);
        assert_eq!(items[0].quantity, 2);
        assert_eq!(items[0].subtotal, 2000);
    }

    #[tokio::test]
    async fn create_order_multiple_items() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let p1 = seed_active_product(&pool, "Item1", 100).await;
        let p2 = seed_active_product(&pool, "Item2", 200).await;

        let order = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![
                    CreateOrderItemRequest {
                        product_id: p1.document_id.clone(),
                        quantity: 3,
                    },
                    CreateOrderItemRequest {
                        product_id: p2.document_id.clone(),
                        quantity: 1,
                    },
                ],
                currency: Some("USD".into()),
                buyer_name: Some("John".into()),
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(order.subtotal, 500);
        assert_eq!(order.total_amount, 500);
        assert_eq!(order.currency, "USD");
        assert_eq!(order.buyer_name.unwrap(), "John");
        let items = order_repo
            .find_items_by_order_id(order.id, None)
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn create_order_empty_items_error() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "items_empty"));
    }

    #[tokio::test]
    async fn create_order_product_not_found() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: "nonexistent".into(),
                    quantity: 1,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_order_product_not_active() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::product::insert(
            &pool,
            &doc_id,
            None,
            "Draft Product",
            None,
            None,
            "custom",
            "digital",
            None,
            None,
            100,
            "CNY",
            None,
            0,
            None,
            None,
            None,
            None,
            None,
            "piece",
            1,
            None,
            0,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let err = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: doc_id,
                    quantity: 1,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "product_not_active"));
    }

    async fn seed_order_with_product(
        pool: &crate::db::Pool,
        product_repo: &dyn ProductRepository,
        order_repo: &dyn OrderRepository,
        auth: &AuthUser,
    ) -> (i64, Order) {
        let uid = seed_user(pool).await;
        let prod = seed_active_product(pool, "Widget", 1000).await;
        let order = create_order(
            pool,
            product_repo,
            order_repo,
            auth,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: prod.document_id.clone(),
                    quantity: 1,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();
        (uid, order)
    }

    #[tokio::test]
    async fn cancel_order_success() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        super::cancel_order(&pool, &order_repo, &a, &order.document_id, uid)
            .await
            .unwrap();
        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Cancelled);
        assert!(found.cancelled_at.is_some());
    }

    #[tokio::test]
    async fn cancel_order_wrong_user() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let err = super::cancel_order(&pool, &order_repo, &a, &order.document_id, 999)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn cancel_order_wrong_status() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();

        let err = super::cancel_order(&pool, &order_repo, &a, &order.document_id, uid)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_cancel"));
    }

    #[tokio::test]
    async fn mark_paid_success() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let paid = super::mark_paid(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);
        assert!(paid.paid_at.is_some());
    }

    #[tokio::test]
    async fn mark_paid_wrong_status() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "cancelled", Some("cancelled_at"), None)
            .await
            .unwrap();

        let err = super::mark_paid(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_pay"));
    }

    #[tokio::test]
    async fn ship_order_success() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        super::ship_order(
            &pool,
            &order_repo,
            &a,
            &order.document_id,
            &ShipOrderRequest {
                tracking_no: Some("TRK001".into()),
                carrier: Some("FedEx".into()),
            },
        )
        .await
        .unwrap();

        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Shipped);
        assert_eq!(found.tracking_no.unwrap(), "TRK001");
        assert_eq!(found.carrier.unwrap(), "FedEx");
    }

    #[tokio::test]
    async fn ship_order_wrong_status() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let err = super::ship_order(
            &pool,
            &order_repo,
            &a,
            &order.document_id,
            &ShipOrderRequest {
                tracking_no: None,
                carrier: None,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_paid_can_ship"));
    }

    #[tokio::test]
    async fn confirm_receipt_success() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        order_repo
            .update_shipped(order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        super::confirm_receipt(&pool, &order_repo, &a, &order.document_id, uid)
            .await
            .unwrap();
        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Completed);
        assert!(found.completed_at.is_some());
    }

    #[tokio::test]
    async fn confirm_receipt_wrong_user() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        order_repo
            .update_shipped(order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        let err = super::confirm_receipt(&pool, &order_repo, &a, &order.document_id, 999)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn confirm_receipt_wrong_status() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let err = super::confirm_receipt(&pool, &order_repo, &a, &order.document_id, uid)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_shipped_can_confirm"));
    }

    #[tokio::test]
    async fn refund_order_from_paid() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        super::refund_order(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap();

        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Refunding);
        assert!(found.refunding_at.is_some());
    }

    #[tokio::test]
    async fn refund_order_from_shipped() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        order_repo
            .update_status(order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        order_repo
            .update_shipped(order.id, Some("TRK"), None, None)
            .await
            .unwrap();
        super::refund_order(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap();

        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Refunding);
    }

    #[tokio::test]
    async fn refund_order_wrong_status() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let err = super::refund_order(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(ref s) if s == "only_paid_or_shipped_can_refund")
        );
    }

    #[tokio::test]
    async fn get_order_with_items() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let (found_order, items) = super::get_order(&order_repo, &a, &order.document_id)
            .await
            .unwrap();
        assert_eq!(found_order.id, order.id);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Widget");
    }

    #[tokio::test]
    async fn get_order_not_found() {
        let pool = setup_pool().await;
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        assert!(
            super::get_order(&order_repo, &a, "nonexistent")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_user_orders() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        for _ in 0..3 {
            super::create_order(
                &pool,
                &product_repo,
                &order_repo,
                &a,
                uid,
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: prod.document_id.clone(),
                        quantity: 1,
                    }],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    remark: None,
                },
            )
            .await
            .unwrap();
        }

        let (orders, total) = super::list_user_orders(&order_repo, &a, uid, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(orders.len(), 3);
    }

    #[tokio::test]
    async fn list_admin_orders() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: prod.document_id.clone(),
                    quantity: 1,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();

        let (orders, total) = super::list_admin_orders(&order_repo, &a, 1, 10, None)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(orders.len(), 1);
    }

    #[tokio::test]
    async fn update_admin_remark_success() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        super::update_admin_remark(&order_repo, &a, &order.document_id, "verified")
            .await
            .unwrap();
        let found = order_repo
            .find_by_id(order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.admin_remark.unwrap(), "verified");
    }

    #[tokio::test]
    async fn get_stats() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let o1 = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: prod.document_id.clone(),
                    quantity: 1,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();

        let o2 = super::create_order(
            &pool,
            &product_repo,
            &order_repo,
            &a,
            uid,
            CreateOrderRequest {
                items: vec![CreateOrderItemRequest {
                    product_id: prod.document_id.clone(),
                    quantity: 2,
                }],
                currency: None,
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
        )
        .await
        .unwrap();

        order_repo
            .update_status(o1.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        order_repo
            .update_status(o1.id, "shipped", None, None)
            .await
            .unwrap();
        order_repo
            .update_status(o1.id, "completed", Some("completed_at"), None)
            .await
            .unwrap();

        let stats = super::get_stats(&pool, &a).await.unwrap();
        assert_eq!(stats.total_orders, 2);
        assert_eq!(stats.pending_orders, 1);
        assert_eq!(stats.completed_orders, 1);
        assert_eq!(stats.total_revenue, 1000);
    }

    #[tokio::test]
    async fn full_lifecycle_pending_to_completed() {
        let pool = setup_pool().await;
        let product_repo = SqlxProductRepository::new(pool.clone());
        let order_repo = SqlxOrderRepository::new(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order_with_product(&pool, &product_repo, &order_repo, &a).await;

        let (o, items) = super::get_order(&order_repo, &a, &order.document_id)
            .await
            .unwrap();
        assert_eq!(o.status, OrderStatus::Pending);
        assert_eq!(items.len(), 1);

        let paid = super::mark_paid(&pool, &order_repo, &a, &order.document_id)
            .await
            .unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);

        super::ship_order(
            &pool,
            &order_repo,
            &a,
            &order.document_id,
            &ShipOrderRequest {
                tracking_no: Some("TRK123".into()),
                carrier: Some("DHL".into()),
            },
        )
        .await
        .unwrap();

        super::confirm_receipt(&pool, &order_repo, &a, &order.document_id, uid)
            .await
            .unwrap();

        let (final_order, _) = super::get_order(&order_repo, &a, &order.document_id)
            .await
            .unwrap();
        assert_eq!(final_order.status, OrderStatus::Completed);
        assert!(final_order.paid_at.is_some());
        assert!(final_order.completed_at.is_some());
        assert_eq!(final_order.tracking_no.unwrap(), "TRK123");
        assert_eq!(final_order.carrier.unwrap(), "DHL");
    }
}
