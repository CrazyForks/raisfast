use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::commands::CreateOrderCmd;
use crate::dto::{CreateOrderRequest, ShipOrderRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::order::{Order, OrderStatus};
use crate::models::order_item::OrderItem;
use crate::models::product::ProductStatus;

const MAX_ITEMS_PER_ORDER: usize = 100;
const MAX_QUANTITY: i64 = 10000;

#[async_trait]
pub trait OrderService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        user_id: i64,
        req: CreateOrderRequest,
    ) -> AppResult<(Order, Vec<OrderItem>)>;
    async fn cancel(&self, auth: &AuthUser, order_id: &str, user_id: i64) -> AppResult<()>;
    async fn mark_paid(&self, auth: &AuthUser, order_id: &str) -> AppResult<Order>;
    async fn ship(&self, auth: &AuthUser, order_id: &str, req: &ShipOrderRequest) -> AppResult<()>;
    async fn confirm_receipt(&self, auth: &AuthUser, order_id: &str, user_id: i64)
    -> AppResult<()>;
    async fn refund(&self, auth: &AuthUser, order_id: &str) -> AppResult<()>;
    async fn admin_cancel(&self, auth: &AuthUser, order_id: &str) -> AppResult<()>;
    async fn get(&self, auth: &AuthUser, order_id: &str) -> AppResult<(Order, Vec<OrderItem>)>;
    async fn list_user(
        &self,
        auth: &AuthUser,
        user_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)>;
    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)>;
    async fn update_admin_remark(
        &self,
        auth: &AuthUser,
        order_id: &str,
        admin_remark: &str,
    ) -> AppResult<()>;
    async fn get_stats(&self, auth: &AuthUser) -> AppResult<crate::dto::OrderStatsResponse>;
}

pub struct OrderServiceImpl {
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
}

impl OrderServiceImpl {
    pub fn new(aspect_engine: Arc<AspectEngine>, pool: Arc<crate::db::Pool>) -> Self {
        Self {
            aspect_engine,
            pool,
        }
    }

    async fn before_create(
        &self,
        auth: &AuthUser,
        req: CreateOrderRequest,
    ) -> AppResult<(CreateOrderRequest, crate::aspects::Dispatched)> {
        self.aspect_engine.before_create("orders", auth, req).await
    }

    fn after_created(&self, order: &Order) {
        self.aspect_engine.emit(Event::OrderCreated(order.clone()));
    }

    fn after_paid(&self, order: &Order) {
        self.aspect_engine.emit(Event::OrderPaid(order.clone()));
    }

    fn after_shipped(&self, order: &Order) {
        self.aspect_engine.emit(Event::OrderShipped(order.clone()));
    }

    fn after_completed(&self, order: &Order) {
        self.aspect_engine
            .emit(Event::OrderCompleted(order.clone()));
    }

    fn after_cancelled(&self, order: &Order) {
        self.aspect_engine
            .emit(Event::OrderCancelled(order.clone()));
    }
}

#[async_trait]
impl OrderService for OrderServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        user_id: i64,
        req: CreateOrderRequest,
    ) -> AppResult<(Order, Vec<OrderItem>)> {
        let (req, _d) = self.before_create(auth, req).await?;

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
            let product = crate::models::product::find_by_document_id(
                &self.pool,
                &item.product_id,
                auth.tenant_id(),
            )
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

        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));

        let currency = req.currency.as_deref().unwrap_or("CNY");
        let total_amount = subtotal;

        let order = crate::in_transaction!(&self.pool, tx, {
            let order = crate::models::order::tx_insert(
                &mut tx,
                &CreateOrderCmd {
                    user_id,
                    order_no,
                    subtotal,
                    discount_amount: 0,
                    shipping_amount: 0,
                    total_amount,
                    currency: currency.into(),
                    buyer_name: req.buyer_name.clone(),
                    buyer_phone: req.buyer_phone.clone(),
                    buyer_email: req.buyer_email.clone(),
                    shipping_address: req.shipping_address.clone(),
                    remark: req.remark.clone(),
                },
                auth.tenant_id(),
            )
            .await?;

            let mut items = Vec::new();
            for (quantity, line_total, product) in &order_items_data {
                items.push(crate::commands::CreateOrderItemCmd {
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

        self.after_created(&order);
        let items =
            crate::models::order_item::find_by_order_id(&self.pool, order.id, auth.tenant_id())
                .await?;
        Ok((order, items))
    }

    async fn cancel(&self, auth: &AuthUser, order_id: &str, user_id: i64) -> AppResult<()> {
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.user_id != user_id {
            return Err(AppError::Forbidden);
        }
        if order.status != OrderStatus::Pending {
            return Err(AppError::BadRequest("only_pending_can_cancel".into()));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Cancelled)
            .await?;

        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    OrderStatus::Cancelled,
                    Some("cancelled_at"),
                    OrderStatus::Pending,
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

        self.after_cancelled(&order);
        Ok(())
    }

    async fn mark_paid(&self, auth: &AuthUser, order_id: &str) -> AppResult<Order> {
        auth.ensure_admin()?;
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.status != OrderStatus::Pending {
            return Err(AppError::BadRequest("only_pending_can_pay".into()));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Paid)
            .await?;

        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    OrderStatus::Paid,
                    Some("paid_at"),
                    OrderStatus::Pending,
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

        let paid = crate::models::order::find_by_id(&self.pool, order.id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("order"))?;

        self.after_paid(&paid);
        Ok(paid)
    }

    async fn ship(&self, auth: &AuthUser, order_id: &str, req: &ShipOrderRequest) -> AppResult<()> {
        auth.ensure_admin()?;
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.status != OrderStatus::Paid {
            return Err(AppError::BadRequest("only_paid_can_ship".into()));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Shipped)
            .await?;

        let order_id = order.id;
        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_shipped(
                    &mut tx,
                    order_id,
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
        result?;

        self.after_shipped(&order);
        Ok(())
    }

    async fn confirm_receipt(
        &self,
        auth: &AuthUser,
        order_id: &str,
        user_id: i64,
    ) -> AppResult<()> {
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.user_id != user_id {
            return Err(AppError::Forbidden);
        }
        if order.status != OrderStatus::Shipped {
            return Err(AppError::BadRequest("only_shipped_can_confirm".into()));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Completed)
            .await?;

        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    OrderStatus::Completed,
                    Some("completed_at"),
                    OrderStatus::Shipped,
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

        self.after_completed(&order);
        Ok(())
    }

    async fn refund(&self, auth: &AuthUser, order_id: &str) -> AppResult<()> {
        auth.ensure_admin()?;
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.status != OrderStatus::Paid && order.status != OrderStatus::Shipped {
            return Err(AppError::BadRequest(
                "only_paid_or_shipped_can_refund".into(),
            ));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Refunding)
            .await?;

        let expected = order.status;
        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    OrderStatus::Refunding,
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

    async fn admin_cancel(&self, auth: &AuthUser, order_id: &str) -> AppResult<()> {
        auth.ensure_admin()?;
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;

        if order.status != OrderStatus::Pending && order.status != OrderStatus::Paid {
            return Err(AppError::BadRequest(
                "only_pending_or_paid_can_admin_cancel".into(),
            ));
        }

        self.aspect_engine
            .before_update("orders", auth, &order, OrderStatus::Cancelled)
            .await?;

        let expected = order.status;
        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order.id,
                    OrderStatus::Cancelled,
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
        result?;
        self.after_cancelled(&order);
        Ok(())
    }

    async fn get(&self, auth: &AuthUser, order_id: &str) -> AppResult<(Order, Vec<OrderItem>)> {
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;
        if auth.role() != "admin" {
            let user_int_id = auth.user_int_id().ok_or(AppError::Unauthorized)?;
            if order.user_id != user_int_id {
                return Err(AppError::Forbidden);
            }
        }
        let items =
            crate::models::order_item::find_by_order_id(&self.pool, order.id, auth.tenant_id())
                .await?;
        Ok((order, items))
    }

    async fn list_user(
        &self,
        auth: &AuthUser,
        user_id: i64,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)> {
        let (orders, total) = crate::models::order::find_by_user_paginated(
            &self.pool,
            user_id,
            auth.tenant_id(),
            page,
            page_size,
        )
        .await?;
        let mut result = Vec::with_capacity(orders.len());
        for o in orders {
            let items =
                crate::models::order_item::find_by_order_id(&self.pool, o.id, auth.tenant_id())
                    .await?;
            result.push((o, items));
        }
        Ok((result, total))
    }

    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)> {
        auth.ensure_admin()?;
        let (orders, total) = crate::models::order::find_all_admin_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            status,
        )
        .await?;
        let mut result = Vec::with_capacity(orders.len());
        for o in orders {
            let items =
                crate::models::order_item::find_by_order_id(&self.pool, o.id, auth.tenant_id())
                    .await?;
            result.push((o, items));
        }
        Ok((result, total))
    }

    async fn update_admin_remark(
        &self,
        auth: &AuthUser,
        order_id: &str,
        admin_remark: &str,
    ) -> AppResult<()> {
        auth.ensure_admin()?;
        let order =
            crate::models::order::find_by_document_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("order"))?;
        crate::models::order::update_admin_remark(
            &self.pool,
            order.id,
            admin_remark,
            auth.tenant_id(),
        )
        .await
    }

    async fn get_stats(&self, auth: &AuthUser) -> AppResult<crate::dto::OrderStatsResponse> {
        auth.ensure_admin()?;
        crate::models::order::get_stats_query(&self.pool, auth.tenant_id()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{CreateOrderItemRequest, ShipOrderRequest};

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn make_service(pool: crate::db::Pool) -> Arc<dyn OrderService> {
        Arc::new(OrderServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
        ))
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some("u1".to_string()),
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    #[allow(dead_code)]
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
        let p = crate::models::product::insert(
            pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: title.to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
            },
            None,
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

    fn make_create_req(prod_doc_id: &str, quantity: i64) -> CreateOrderRequest {
        CreateOrderRequest {
            items: vec![CreateOrderItemRequest {
                product_id: prod_doc_id.to_string(),
                quantity,
            }],
            currency: None,
            buyer_name: None,
            buyer_phone: None,
            buyer_email: None,
            shipping_address: None,
            remark: None,
        }
    }

    async fn seed_order(
        svc: &dyn OrderService,
        pool: &crate::db::Pool,
        auth: &AuthUser,
    ) -> (i64, Order) {
        let uid = seed_user(pool).await;
        let prod = seed_active_product(pool, "Widget", 1000).await;
        let (order, _) = svc
            .create(auth, uid, make_create_req(&prod.document_id, 1))
            .await
            .unwrap();
        (uid, order)
    }

    #[tokio::test]
    async fn create_order_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let (order, items) = svc
            .create(&a, uid, make_create_req(&prod.document_id, 2))
            .await
            .unwrap();

        assert_eq!(order.user_id, uid);
        assert_eq!(order.subtotal, 2000);
        assert_eq!(order.total_amount, 2000);
        assert_eq!(order.status, OrderStatus::Pending);
        assert!(order.order_no.starts_with("ORD-"));

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Widget");
        assert_eq!(items[0].unit_price, 1000);
        assert_eq!(items[0].quantity, 2);
        assert_eq!(items[0].subtotal, 2000);
    }

    #[tokio::test]
    async fn create_order_multiple_items() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let p1 = seed_active_product(&pool, "Item1", 100).await;
        let p2 = seed_active_product(&pool, "Item2", 200).await;

        let (order, items) = svc
            .create(
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
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn create_order_empty_items_error() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = svc
            .create(
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = svc
            .create(
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let draft_product = crate::models::product::insert(
            &pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Draft Product".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 100,
                currency: "CNY".to_string(),
                attributes: None,
                sort_order: 0,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: "piece".to_string(),
                min_purchase: 1,
                max_purchase: None,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
            },
            None,
        )
        .await
        .unwrap();

        let err = svc
            .create(&a, uid, make_create_req(&draft_product.document_id, 1))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "product_not_active"));
    }

    #[tokio::test]
    async fn cancel_order_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        svc.cancel(&a, &order.document_id, uid).await.unwrap();
        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Cancelled);
        assert!(found.cancelled_at.is_some());
    }

    #[tokio::test]
    async fn cancel_order_wrong_user() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc.cancel(&a, &order.document_id, 999).await.unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn cancel_order_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();

        let err = svc.cancel(&a, &order.document_id, uid).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_cancel"));
    }

    #[tokio::test]
    async fn mark_paid_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let paid = svc.mark_paid(&a, &order.document_id).await.unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);
        assert!(paid.paid_at.is_some());
    }

    #[tokio::test]
    async fn mark_paid_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(
            &pool,
            order.id,
            "cancelled",
            Some("cancelled_at"),
            None,
        )
        .await
        .unwrap();

        let err = svc.mark_paid(&a, &order.document_id).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_pay"));
    }

    #[tokio::test]
    async fn ship_order_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool.clone(), order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        svc.ship(
            &a,
            &order.document_id,
            &ShipOrderRequest {
                tracking_no: Some("TRK001".into()),
                carrier: Some("FedEx".into()),
            },
        )
        .await
        .unwrap();

        let found = crate::models::order::find_by_id(&pool, order.id, None)
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc
            .ship(
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
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        svc.confirm_receipt(&a, &order.document_id, uid)
            .await
            .unwrap();
        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Completed);
        assert!(found.completed_at.is_some());
    }

    #[tokio::test]
    async fn confirm_receipt_wrong_user() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        let err = svc
            .confirm_receipt(&a, &order.document_id, 999)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn confirm_receipt_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc
            .confirm_receipt(&a, &order.document_id, uid)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_shipped_can_confirm"));
    }

    #[tokio::test]
    async fn refund_order_from_paid() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        svc.refund(&a, &order.document_id).await.unwrap();

        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Refunding);
        assert!(found.refunding_at.is_some());
    }

    #[tokio::test]
    async fn refund_order_from_shipped() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), None, None)
            .await
            .unwrap();
        svc.refund(&a, &order.document_id).await.unwrap();

        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Refunding);
    }

    #[tokio::test]
    async fn refund_order_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc.refund(&a, &order.document_id).await.unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(ref s) if s == "only_paid_or_shipped_can_refund")
        );
    }

    #[tokio::test]
    async fn get_order_with_items() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let (found_order, items) = svc.get(&a, &order.document_id).await.unwrap();
        assert_eq!(found_order.id, order.id);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Widget");
    }

    #[tokio::test]
    async fn get_order_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let a = auth(None);
        assert!(svc.get(&a, "nonexistent").await.is_err());
    }

    #[tokio::test]
    async fn list_user_orders() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        for _ in 0..3 {
            svc.create(&a, uid, make_create_req(&prod.document_id, 1))
                .await
                .unwrap();
        }

        let (orders_with_items, total) = svc.list_user(&a, uid, 1, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(orders_with_items.len(), 3);
    }

    #[tokio::test]
    async fn list_admin_orders() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        svc.create(&a, uid, make_create_req(&prod.document_id, 1))
            .await
            .unwrap();

        let (orders_with_items, total) = svc.list_admin(&a, 1, 10, None).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(orders_with_items.len(), 1);
    }

    #[tokio::test]
    async fn update_admin_remark_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        svc.update_admin_remark(&a, &order.document_id, "verified")
            .await
            .unwrap();
        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.admin_remark.unwrap(), "verified");
    }

    #[tokio::test]
    async fn get_stats() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let (o1, _) = svc
            .create(&a, uid, make_create_req(&prod.document_id, 1))
            .await
            .unwrap();

        let (_o2, _) = svc
            .create(&a, uid, make_create_req(&prod.document_id, 2))
            .await
            .unwrap();

        crate::models::order::update_status(&pool, o1.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_status(&pool, o1.id, "shipped", None, None)
            .await
            .unwrap();
        crate::models::order::update_status(&pool, o1.id, "completed", Some("completed_at"), None)
            .await
            .unwrap();

        let stats = svc.get_stats(&a).await.unwrap();
        assert_eq!(stats.total_orders, 2);
        assert_eq!(stats.pending_orders, 1);
        assert_eq!(stats.completed_orders, 1);
        assert_eq!(stats.total_revenue, 1000);
    }

    #[tokio::test]
    async fn full_lifecycle_pending_to_completed() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let (o, items) = svc.get(&a, &order.document_id).await.unwrap();
        assert_eq!(o.status, OrderStatus::Pending);
        assert_eq!(items.len(), 1);

        let paid = svc.mark_paid(&a, &order.document_id).await.unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);

        svc.ship(
            &a,
            &order.document_id,
            &ShipOrderRequest {
                tracking_no: Some("TRK123".into()),
                carrier: Some("DHL".into()),
            },
        )
        .await
        .unwrap();

        svc.confirm_receipt(&a, &order.document_id, uid)
            .await
            .unwrap();

        let (final_order, _) = svc.get(&a, &order.document_id).await.unwrap();
        assert_eq!(final_order.status, OrderStatus::Completed);
        assert!(final_order.paid_at.is_some());
        assert!(final_order.completed_at.is_some());
        assert_eq!(final_order.tracking_no.unwrap(), "TRK123");
        assert_eq!(final_order.carrier.unwrap(), "DHL");
    }
}
