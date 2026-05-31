use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::cart::{CartItemResponse, CartResponse};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::product::ProductStatus;
use crate::types::snowflake_id::SnowflakeId;

const MAX_CART_ITEMS: usize = 100;

#[async_trait]
pub trait CartService: Send + Sync {
    async fn add_item(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        product_id: String,
        quantity: i64,
        variant_id: Option<String>,
        attributes: Option<String>,
    ) -> AppResult<()>;

    async fn remove_item(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()>;

    async fn update_quantity(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
        quantity: i64,
    ) -> AppResult<()>;

    async fn list_items(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<CartResponse>;

    async fn clear_cart(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<()>;

    async fn checkout(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
    ) -> AppResult<(
        crate::models::order::Order,
        Vec<crate::models::order_item::OrderItem>,
    )>;
}

pub struct CartServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl CartServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CartService for CartServiceImpl {
    async fn add_item(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        product_id: String,
        quantity: i64,
        variant_id: Option<String>,
        attributes: Option<String>,
    ) -> AppResult<()> {
        let product_id = crate::types::snowflake_id::parse_id(&product_id)?;
        let product = crate::models::product::find_by_id(&self.pool, product_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        if product.status != ProductStatus::Active {
            return Err(AppError::BadRequest("product_not_active".into()));
        }

        let resolved_variant_id: Option<i64> = match variant_id.as_deref() {
            Some(vid) => Some(*crate::types::snowflake_id::parse_id(vid)?),
            None => None,
        };

        if quantity > 10000 {
            return Err(AppError::BadRequest("quantity_exceeds_limit".into()));
        }

        let existing = crate::models::cart_item::find_by_user_and_product(
            &self.pool,
            user_id,
            product.id,
            resolved_variant_id,
            auth.tenant_id(),
        )
        .await?;

        if let Some(item) = existing {
            let new_quantity = item
                .quantity
                .checked_add(quantity)
                .ok_or_else(|| AppError::BadRequest("quantity_overflow".into()))?;
            crate::models::cart_item::update_quantity(
                &self.pool,
                item.id,
                new_quantity,
                auth.tenant_id(),
            )
            .await?;
        } else {
            let count =
                crate::models::cart_item::count_by_user(&self.pool, user_id, auth.tenant_id())
                    .await?;
            if count as usize >= MAX_CART_ITEMS {
                return Err(AppError::BadRequest("cart_full".into()));
            }
            crate::models::cart_item::insert(
                &self.pool,
                user_id,
                product.id,
                resolved_variant_id,
                quantity,
                attributes.as_deref(),
                auth.tenant_id(),
            )
            .await?;
        }

        Ok(())
    }

    async fn remove_item(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
    ) -> AppResult<()> {
        let item = crate::models::cart_item::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("cart_item"))?;

        if item.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        crate::models::cart_item::delete_by_id(&self.pool, id, auth.tenant_id()).await?;

        Ok(())
    }

    async fn update_quantity(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        user_id: SnowflakeId,
        quantity: i64,
    ) -> AppResult<()> {
        if quantity < 1 {
            return Err(AppError::BadRequest("invalid_quantity".into()));
        }

        let item = crate::models::cart_item::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("cart_item"))?;

        if item.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        crate::models::cart_item::update_quantity(&self.pool, item.id, quantity, auth.tenant_id())
            .await?;

        Ok(())
    }

    async fn list_items(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<CartResponse> {
        let items =
            crate::models::cart_item::find_by_user_id(&self.pool, user_id, auth.tenant_id())
                .await?;

        let mut response_items = Vec::with_capacity(items.len());
        let mut total: i64 = 0;

        for item in &items {
            let product =
                crate::models::product::find_by_id(&self.pool, item.product_id, auth.tenant_id())
                    .await?;

            let (title, price, cover_url) = match product {
                Some(ref p) => (p.title.clone(), p.price, p.cover_url.clone()),
                None => ("(deleted)".to_string(), 0, None),
            };

            let line_total = price.checked_mul(item.quantity).unwrap_or(0);
            total = total.checked_add(line_total).unwrap_or(i64::MAX);

            response_items.push(CartItemResponse {
                id: item.id.to_string(),
                quantity: item.quantity,
                attributes: item.attributes.clone(),
                title,
                price,
                cover_url,
                created_at: item.created_at.to_string(),
                updated_at: item.updated_at.to_string(),
            });
        }

        Ok(CartResponse {
            items: response_items,
            total,
        })
    }

    async fn clear_cart(&self, auth: &AuthUser, user_id: SnowflakeId) -> AppResult<()> {
        crate::models::cart_item::delete_by_user_id(&self.pool, user_id, auth.tenant_id()).await?;
        Ok(())
    }

    async fn checkout(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
    ) -> AppResult<(
        crate::models::order::Order,
        Vec<crate::models::order_item::OrderItem>,
    )> {
        let cart_items =
            crate::models::cart_item::find_by_user_id(&self.pool, user_id, auth.tenant_id())
                .await?;

        if cart_items.is_empty() {
            return Err(AppError::BadRequest("cart_empty".into()));
        }

        let mut order_items_data: Vec<(i64, i64, crate::models::product::Product)> = Vec::new();
        let mut subtotal: i64 = 0;

        for item in &cart_items {
            let product =
                crate::models::product::find_by_id(&self.pool, item.product_id, auth.tenant_id())
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
        let total_amount = subtotal;

        let order = crate::in_transaction!(&self.pool, tx, {
            let order = crate::models::order::tx_insert(
                &mut tx,
                &crate::commands::CreateOrderCmd {
                    user_id,
                    order_no,
                    subtotal,
                    discount_amount: 0,
                    shipping_amount: 0,
                    total_amount,
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
                auth.tenant_id(),
            )
            .await?;

            let mut items = Vec::new();
            for (quantity, line_total, product) in &order_items_data {
                items.push(crate::commands::CreateOrderItemCmd {
                    order_id: order.id,
                    product_id: Some(*product.id),
                    variant_id: None,
                    title: product.title.clone(),
                    description: product.description.clone(),
                    sku: None,
                    unit_price: product.price,
                    quantity: *quantity,
                    subtotal: *line_total,
                    tax_amount: 0,
                    cover_url: product.cover_url.clone(),
                    attributes: product.attributes.clone(),
                });
            }
            crate::models::order_item::tx_insert_batch(&mut tx, items, auth.tenant_id()).await?;

            crate::models::cart_item::tx_delete_by_user_id(&mut tx, user_id, auth.tenant_id())
                .await?;

            Ok(order)
        })?;

        let items =
            crate::models::order_item::find_by_order_id(&self.pool, order.id, auth.tenant_id())
                .await?;
        Ok((order, items))
    }
}
