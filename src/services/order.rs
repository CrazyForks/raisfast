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
use crate::types::snowflake_id::SnowflakeId;

const MAX_ITEMS_PER_ORDER: usize = 100;
const MAX_QUANTITY: i64 = 10000;

#[async_trait]
pub trait OrderService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        req: CreateOrderRequest,
    ) -> AppResult<(Order, Vec<OrderItem>)>;
    async fn cancel(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()>;
    async fn mark_paid(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<Order>;
    async fn ship(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        req: &ShipOrderRequest,
    ) -> AppResult<()>;
    async fn confirm_receipt(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()>;
    async fn refund(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<()>;
    async fn admin_cancel(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<()>;
    async fn get(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
    ) -> AppResult<(Order, Vec<OrderItem>)>;
    async fn list_user(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)>;
    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        keyword: Option<&str>,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)>;
    async fn update_admin_remark(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        admin_remark: &str,
    ) -> AppResult<()>;
    async fn get_stats(&self, auth: &AuthUser) -> AppResult<crate::dto::OrderStatsResponse>;
}

pub struct OrderServiceImpl {
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
    options: Arc<crate::services::options::OptionsService>,
    coupon_service: Option<Arc<dyn crate::services::coupon::CouponService>>,
    shipping_template_service:
        Option<Arc<dyn crate::services::shipping_template::ShippingTemplateService>>,
}

impl OrderServiceImpl {
    pub fn new(
        aspect_engine: Arc<AspectEngine>,
        pool: Arc<crate::db::Pool>,
        options: Arc<crate::services::options::OptionsService>,
    ) -> Self {
        Self {
            aspect_engine,
            pool,
            options,
            coupon_service: None,
            shipping_template_service: None,
        }
    }

    pub fn with_coupon_service(
        mut self,
        svc: Arc<dyn crate::services::coupon::CouponService>,
    ) -> Self {
        self.coupon_service = Some(svc);
        self
    }

    pub fn with_shipping_template_service(
        mut self,
        svc: Arc<dyn crate::services::shipping_template::ShippingTemplateService>,
    ) -> Self {
        self.shipping_template_service = Some(svc);
        self
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
        user_id: SnowflakeId,
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
        let mut variant_map: std::collections::HashMap<
            i64,
            crate::models::product_variant::ProductVariant,
        > = std::collections::HashMap::new();
        let mut subtotal: i64 = 0;

        for item in &req.items {
            if item.quantity > MAX_QUANTITY {
                return Err(AppError::BadRequest("quantity_exceeds_limit".into()));
            }
            let product_id = crate::types::snowflake_id::parse_id(&item.product_id)?;
            let product =
                crate::models::product::find_by_id(&self.pool, product_id, auth.tenant_id())
                    .await?
                    .ok_or_else(|| AppError::not_found("product"))?;

            if product.status != ProductStatus::Active {
                return Err(AppError::BadRequest("product_not_active".into()));
            }

            if let Some(ref vid_str) = item.variant_id {
                let vid = crate::types::snowflake_id::parse_id(vid_str)?;
                let variant =
                    crate::models::product_variant::find_by_id(&self.pool, vid, auth.tenant_id())
                        .await?
                        .ok_or_else(|| AppError::not_found("product_variant"))?;
                if variant.product_id != product.id {
                    return Err(AppError::BadRequest("variant_not_belong_to_product".into()));
                }
                if !variant.is_active {
                    return Err(AppError::BadRequest("variant_not_active".into()));
                }
                variant_map.insert(*vid, variant);
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

        let default_currency = self
            .options
            .get(auth.tenant_id(), "default_currency")
            .await
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "USD".to_string());
        let currency = req.currency.as_deref().unwrap_or(&default_currency);
        crate::models::currencies::ensure_active(&self.pool, currency, auth.tenant_id()).await?;

        let (discount_amount, coupon_id) = if let Some(ref coupon_svc) = self.coupon_service {
            let c_id = req
                .coupon_id
                .as_deref()
                .map(crate::types::snowflake_id::parse_id)
                .transpose()?;
            let c_code = req.coupon_code.as_deref();
            match (c_id, c_code) {
                (Some(_), _) | (_, Some(_)) => {
                    let coupon = coupon_svc
                        .validate_coupon(c_id, c_code, user_id, subtotal, auth.tenant_id())
                        .await?;
                    let discount = coupon_svc.calculate_discount(&coupon, subtotal);
                    (discount, Some(*coupon.id))
                }
                _ => (0, None),
            }
        } else {
            (0, None)
        };

        let shipping_address_id = req
            .shipping_address_id
            .as_deref()
            .map(crate::types::snowflake_id::parse_id)
            .transpose()?;
        let billing_address_id = req
            .billing_address_id
            .as_deref()
            .map(crate::types::snowflake_id::parse_id)
            .transpose()?;

        let shipping_addr = if let Some(addr_id) = shipping_address_id {
            let addr =
                crate::models::user_address::find_by_id(&self.pool, addr_id, auth.tenant_id())
                    .await?
                    .ok_or_else(|| AppError::not_found("shipping_address"))?;
            if addr.user_id != user_id {
                return Err(AppError::Forbidden);
            }
            Some(addr)
        } else {
            None
        };

        let shipping_amount = if let Some(ref ship_svc) = self.shipping_template_service {
            let product_weights: Vec<(SnowflakeId, i64, i64)> = order_items_data
                .iter()
                .map(|(quantity, _, product)| (product.id, product.weight.unwrap_or(0), *quantity))
                .collect();
            let region = shipping_addr.as_ref().map(|a| a.province.clone());
            match ship_svc
                .calculate_shipping(&product_weights, region.as_deref(), auth.tenant_id())
                .await
            {
                Ok(result) => result.shipping_amount,
                Err(_) => 0,
            }
        } else {
            0
        };

        let total_amount = subtotal - discount_amount + shipping_amount;

        let shipping_address_text = req.shipping_address.clone().or_else(|| {
            shipping_addr.as_ref().map(|a| {
                format!(
                    "{} {} {} {} {}",
                    a.recipient_name, a.phone, a.province, a.city, a.address_line1
                )
            })
        });

        let buyer_name = req
            .buyer_name
            .clone()
            .or_else(|| shipping_addr.as_ref().map(|a| a.recipient_name.clone()));
        let buyer_phone = req
            .buyer_phone
            .clone()
            .or_else(|| shipping_addr.as_ref().map(|a| a.phone.clone()));

        let order = crate::in_transaction!(&self.pool, tx, {
            let order = crate::models::order::tx_insert(
                &mut tx,
                &CreateOrderCmd {
                    user_id,
                    order_no,
                    subtotal,
                    discount_amount,
                    shipping_amount,
                    total_amount,
                    currency: currency.into(),
                    buyer_name,
                    buyer_phone,
                    buyer_email: req.buyer_email.clone(),
                    shipping_address: shipping_address_text,
                    remark: req.remark.clone(),
                    tax_amount: 0,
                    coupon_id,
                    shipping_address_id: shipping_address_id.map(|id| *id),
                    billing_address_id: billing_address_id.map(|id| *id),
                },
                auth.tenant_id(),
            )
            .await?;

            if let Some(cid) = coupon_id {
                crate::models::coupon::tx_increment_used(
                    &mut tx,
                    SnowflakeId(cid),
                    auth.tenant_id(),
                )
                .await?;
            }

            let mut items = Vec::new();
            for (idx, (quantity, _line_total, product)) in order_items_data.iter().enumerate() {
                let variant_opt = req.items[idx].variant_id.as_deref().and_then(|vid| {
                    let id = crate::types::snowflake_id::parse_id(vid).ok()?;
                    variant_map.get(&(*id))
                });

                let (variant_id, sku, unit_price, attributes) = match variant_opt {
                    Some(v) => (Some(*v.id), v.sku.clone(), v.price, v.attributes.clone()),
                    None => (None, None, product.price, product.attributes.clone()),
                };

                let actual_line_total = unit_price
                    .checked_mul(*quantity)
                    .ok_or_else(|| AppError::BadRequest("line_total_overflow".into()))?;

                items.push(crate::commands::CreateOrderItemCmd {
                    order_id: order.id,
                    product_id: Some(*product.id),
                    variant_id,
                    title: product.title.clone(),
                    description: product.description.clone(),
                    sku,
                    unit_price,
                    quantity: *quantity,
                    subtotal: actual_line_total,
                    tax_amount: 0,
                    cover_url: product.cover_url.clone(),
                    attributes,
                });
            }
            crate::models::order_item::tx_insert_batch(&mut tx, items, auth.tenant_id()).await?;

            for (idx, (quantity, _, product)) in order_items_data.iter().enumerate() {
                match req.items[idx]
                    .variant_id
                    .as_deref()
                    .map(crate::types::snowflake_id::parse_id)
                {
                    Some(Ok(vid)) => {
                        crate::models::product_variant::tx_deduct_stock(
                            &mut tx,
                            vid,
                            *quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    }
                    _ => {
                        crate::models::product::tx_deduct_stock(
                            &mut tx,
                            product.id,
                            *quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    }
                }
            }

            Ok(order)
        })?;

        self.after_created(&order);
        let items =
            crate::models::order_item::find_by_order_id(&self.pool, order.id, auth.tenant_id())
                .await?;
        Ok((order, items))
    }

    async fn cancel(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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

                let order_items = crate::models::order_item::find_by_order_id(
                    &self.pool,
                    order.id,
                    auth.tenant_id(),
                )
                .await?;
                for item in &order_items {
                    if let Some(vid) = item.variant_id {
                        crate::models::product_variant::tx_replenish_stock(
                            &mut tx,
                            vid,
                            item.quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    } else if let Some(pid) = item.product_id {
                        crate::models::product::tx_replenish_stock(
                            &mut tx,
                            pid,
                            item.quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    }
                }

                Ok(())
            })
        }
        .await;
        result?;

        self.after_cancelled(&order);
        Ok(())
    }

    async fn mark_paid(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<Order> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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

    async fn ship(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
        req: &ShipOrderRequest,
    ) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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
        order_id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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

    async fn refund(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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

    async fn admin_cancel(&self, auth: &AuthUser, order_id: SnowflakeId) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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
        let order_id = order.id;
        let result: Result<(), AppError> = async {
            crate::in_transaction!(&self.pool, tx, {
                let rows = crate::models::order::tx_update_status_cas(
                    &mut tx,
                    order_id,
                    OrderStatus::Cancelled,
                    Some("cancelled_at"),
                    expected,
                )
                .await?;
                if rows == 0 {
                    return Err(AppError::BadRequest("concurrent_status_change".into()));
                }

                let order_items = crate::models::order_item::find_by_order_id(
                    &self.pool,
                    order_id,
                    auth.tenant_id(),
                )
                .await?;
                for item in &order_items {
                    if let Some(vid) = item.variant_id {
                        crate::models::product_variant::tx_replenish_stock(
                            &mut tx,
                            vid,
                            item.quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    } else if let Some(pid) = item.product_id {
                        crate::models::product::tx_replenish_stock(
                            &mut tx,
                            pid,
                            item.quantity,
                            auth.tenant_id(),
                        )
                        .await?;
                    }
                }

                Ok(())
            })
        }
        .await;
        result?;

        self.after_cancelled(&order);
        Ok(())
    }

    async fn get(
        &self,
        auth: &AuthUser,
        order_id: SnowflakeId,
    ) -> AppResult<(Order, Vec<OrderItem>)> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("order"))?;
        if !auth.is_admin() {
            let user_id = auth.ensure_snowflake_user_id()?;
            if order.user_id != user_id {
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
        user_id: SnowflakeId,
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
        keyword: Option<&str>,
    ) -> AppResult<(Vec<(Order, Vec<OrderItem>)>, i64)> {
        let (orders, total) = crate::models::order::find_all_admin_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            status,
            keyword,
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
        order_id: SnowflakeId,
        admin_remark: &str,
    ) -> AppResult<()> {
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
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

    async fn make_service(pool: crate::db::Pool) -> Arc<dyn OrderService> {
        Arc::new(OrderServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
            Arc::new(
                crate::services::options::OptionsService::new(Arc::new(setup_pool().await), false)
                    .await,
            ),
        ))
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string())
                .or_else(|| Some("default".to_string())),
        )
    }

    #[allow(dead_code)]
    fn auth_with_id(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            Some("default".to_string()),
        )
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("testuser_{id}");
        let _: crate::db::pool::DbQueryResult = sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')")
            .bind(id)
            .bind(&username)
            .execute(pool)
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
                stock: 100,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();
        let _: crate::db::pool::DbQueryResult =
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

    fn make_create_req(prod_id: &str, quantity: i64) -> CreateOrderRequest {
        CreateOrderRequest {
            items: vec![CreateOrderItemRequest {
                product_id: prod_id.to_string(),
                quantity,
                variant_id: None,
            }],
            currency: None,
            buyer_name: None,
            buyer_phone: None,
            buyer_email: None,
            shipping_address: None,
            shipping_address_id: None,
            billing_address_id: None,
            remark: None,
            coupon_id: None,
            coupon_code: None,
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
            .create(
                auth,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 1),
            )
            .await
            .unwrap();
        (uid, order)
    }

    #[tokio::test]
    async fn create_order_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let (order, items) = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 2),
            )
            .await
            .unwrap();

        assert_eq!(order.user_id, SnowflakeId(uid));
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let p1 = seed_active_product(&pool, "Item1", 100).await;
        let p2 = seed_active_product(&pool, "Item2", 200).await;

        let (order, items) = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![
                        CreateOrderItemRequest {
                            product_id: p1.id.to_string(),
                            quantity: 3,
                            variant_id: None,
                        },
                        CreateOrderItemRequest {
                            product_id: p2.id.to_string(),
                            quantity: 1,
                            variant_id: None,
                        },
                    ],
                    currency: Some("USD".into()),
                    buyer_name: Some("John".into()),
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: None,
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: None,
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "items_empty"));
    }

    #[tokio::test]
    async fn create_order_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;

        let err = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: "99999999".into(),
                        quantity: 1,
                        variant_id: None,
                    }],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: None,
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_order_product_not_active() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
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
                stock: 0,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
            None,
        )
        .await
        .unwrap();

        let err = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&draft_product.id.to_string(), 1),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "product_not_active"));
    }

    #[tokio::test]
    async fn cancel_order_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        svc.cancel(&a, order.id, SnowflakeId(uid)).await.unwrap();
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc
            .cancel(&a, order.id, SnowflakeId(999))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn cancel_order_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();

        let err = svc
            .cancel(&a, order.id, SnowflakeId(uid))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_cancel"));
    }

    #[tokio::test]
    async fn mark_paid_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let paid = svc.mark_paid(&a, order.id).await.unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);
        assert!(paid.paid_at.is_some());
    }

    #[tokio::test]
    async fn mark_paid_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
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

        let err = svc.mark_paid(&a, order.id).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_pending_can_pay"));
    }

    #[tokio::test]
    async fn ship_order_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool.clone(), order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        svc.ship(
            &a,
            order.id,
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc
            .ship(
                &a,
                order.id,
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        svc.confirm_receipt(&a, order.id, SnowflakeId(uid))
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();

        let err = svc
            .confirm_receipt(&a, order.id, SnowflakeId(999))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn confirm_receipt_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc
            .confirm_receipt(&a, order.id, SnowflakeId(uid))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_shipped_can_confirm"));
    }

    #[tokio::test]
    async fn refund_order_from_paid() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        svc.refund(&a, order.id).await.unwrap();

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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        crate::models::order::update_status(&pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(&pool, order.id, Some("TRK"), None, None)
            .await
            .unwrap();
        svc.refund(&a, order.id).await.unwrap();

        let found = crate::models::order::find_by_id(&pool, order.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, OrderStatus::Refunding);
    }

    #[tokio::test]
    async fn refund_order_wrong_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let err = svc.refund(&a, order.id).await.unwrap_err();
        assert!(
            matches!(err, AppError::BadRequest(ref s) if s == "only_paid_or_shipped_can_refund")
        );
    }

    #[tokio::test]
    async fn get_order_with_items() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let (found_order, items) = svc.get(&a, order.id).await.unwrap();
        assert_eq!(found_order.id, order.id);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Widget");
    }

    #[tokio::test]
    async fn get_order_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool).await;
        let a = auth(None);
        assert!(svc.get(&a, SnowflakeId(0)).await.is_err());
    }

    #[tokio::test]
    async fn list_user_orders() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        for _ in 0..3 {
            svc.create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 1),
            )
            .await
            .unwrap();
        }

        let (orders_with_items, total) = svc.list_user(&a, SnowflakeId(uid), 1, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(orders_with_items.len(), 3);
    }

    #[tokio::test]
    async fn list_admin_orders() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        svc.create(
            &a,
            SnowflakeId(uid),
            make_create_req(&prod.id.to_string(), 1),
        )
        .await
        .unwrap();

        let (orders_with_items, total) = svc.list_admin(&a, 1, 10, None, None).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(orders_with_items.len(), 1);
    }

    #[tokio::test]
    async fn update_admin_remark_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (_, order) = seed_order(svc.as_ref(), &pool, &a).await;

        svc.update_admin_remark(&a, order.id, "verified")
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let (o1, _) = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 1),
            )
            .await
            .unwrap();

        let (_o2, _) = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 2),
            )
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
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let (uid, order) = seed_order(svc.as_ref(), &pool, &a).await;

        let (o, items) = svc.get(&a, order.id).await.unwrap();
        assert_eq!(o.status, OrderStatus::Pending);
        assert_eq!(items.len(), 1);

        let paid = svc.mark_paid(&a, order.id).await.unwrap();
        assert_eq!(paid.status, OrderStatus::Paid);

        svc.ship(
            &a,
            order.id,
            &ShipOrderRequest {
                tracking_no: Some("TRK123".into()),
                carrier: Some("DHL".into()),
            },
        )
        .await
        .unwrap();

        svc.confirm_receipt(&a, order.id, SnowflakeId(uid))
            .await
            .unwrap();

        let (final_order, _) = svc.get(&a, order.id).await.unwrap();
        assert_eq!(final_order.status, OrderStatus::Completed);
        assert!(final_order.paid_at.is_some());
        assert!(final_order.completed_at.is_some());
        assert_eq!(final_order.tracking_no.unwrap(), "TRK123");
        assert_eq!(final_order.carrier.unwrap(), "DHL");
    }

    async fn get_product_stock(pool: &crate::db::Pool, id: SnowflakeId) -> i64 {
        let (s,): (i64,) = sqlx::query_as("SELECT stock FROM products WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn create_order_deducts_stock() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        assert_eq!(get_product_stock(&pool, prod.id).await, 100);

        svc.create(
            &a,
            SnowflakeId(uid),
            make_create_req(&prod.id.to_string(), 3),
        )
        .await
        .unwrap();

        assert_eq!(get_product_stock(&pool, prod.id).await, 97);
    }

    #[tokio::test]
    async fn cancel_order_replenishes_stock() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        svc.create(
            &a,
            SnowflakeId(uid),
            make_create_req(&prod.id.to_string(), 5),
        )
        .await
        .unwrap();
        assert_eq!(get_product_stock(&pool, prod.id).await, 95);

        let (order, _) = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 3),
            )
            .await
            .unwrap();
        assert_eq!(get_product_stock(&pool, prod.id).await, 92);

        svc.cancel(&a, order.id, SnowflakeId(uid)).await.unwrap();
        assert_eq!(get_product_stock(&pool, prod.id).await, 95);
    }

    #[tokio::test]
    async fn create_order_insufficient_stock() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let _: crate::db::pool::DbQueryResult =
            sqlx::query("UPDATE products SET stock = ? WHERE id = ?")
                .bind(2i64)
                .bind(prod.id)
                .execute(&pool)
                .await
                .unwrap();

        let err = svc
            .create(
                &a,
                SnowflakeId(uid),
                make_create_req(&prod.id.to_string(), 5),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "insufficient_stock"));
        assert_eq!(get_product_stock(&pool, prod.id).await, 2);
    }

    async fn seed_address(
        pool: &crate::db::Pool,
        user_id: i64,
    ) -> crate::models::user_address::UserAddress {
        crate::models::user_address::insert(
            pool,
            &crate::commands::CreateUserAddressCmd {
                user_id: SnowflakeId(user_id),
                label: "Home".to_string(),
                recipient_name: "Zhang San".to_string(),
                phone: "13800138000".to_string(),
                country: "CN".to_string(),
                province: "Guangdong".to_string(),
                city: "Shenzhen".to_string(),
                district: "Nanshan".to_string(),
                address_line1: "123 Tech Park".to_string(),
                address_line2: None,
                postal_code: Some("518000".to_string()),
                is_default: true,
                address_type: "shipping".to_string(),
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn create_order_with_shipping_address_id() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;
        let addr = seed_address(&pool, uid).await;

        let (order, _) = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: prod.id.to_string(),
                        quantity: 1,
                        variant_id: None,
                    }],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: Some(addr.id.to_string()),
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap();

        let shipping = order.shipping_address.unwrap();
        assert!(shipping.contains("Zhang San"));
        assert!(shipping.contains("13800138000"));
        assert!(shipping.contains("Guangdong"));
        assert!(shipping.contains("Shenzhen"));
        assert!(shipping.contains("123 Tech Park"));
        assert_eq!(order.buyer_name.unwrap(), "Zhang San");
        assert_eq!(order.buyer_phone.unwrap(), "13800138000");
    }

    #[tokio::test]
    async fn create_order_with_wrong_user_address_forbidden() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;
        let addr = seed_address(&pool, uid1).await;

        let err = svc
            .create(
                &a,
                SnowflakeId(uid2),
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: prod.id.to_string(),
                        quantity: 1,
                        variant_id: None,
                    }],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: Some(addr.id.to_string()),
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn create_order_with_nonexistent_address_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let err = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: prod.id.to_string(),
                        quantity: 1,
                        variant_id: None,
                    }],
                    currency: None,
                    buyer_name: None,
                    buyer_phone: None,
                    buyer_email: None,
                    shipping_address: None,
                    shipping_address_id: Some(SnowflakeId(99999).to_string()),
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn create_order_address_text_used_when_no_id() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let uid = seed_user(&pool).await;
        let prod = seed_active_product(&pool, "Widget", 1000).await;

        let (order, _) = svc
            .create(
                &a,
                SnowflakeId(uid),
                CreateOrderRequest {
                    items: vec![CreateOrderItemRequest {
                        product_id: prod.id.to_string(),
                        quantity: 1,
                        variant_id: None,
                    }],
                    currency: None,
                    buyer_name: Some("Test".to_string()),
                    buyer_phone: Some("123".to_string()),
                    buyer_email: None,
                    shipping_address: Some("456 Manual Road".to_string()),
                    shipping_address_id: None,
                    billing_address_id: None,
                    remark: None,
                    coupon_id: None,
                    coupon_code: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(order.shipping_address.unwrap(), "456 Manual Road");
        assert_eq!(order.buyer_name.unwrap(), "Test");
        assert_eq!(order.buyer_phone.unwrap(), "123");
    }
}
