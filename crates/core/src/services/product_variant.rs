use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::product_variant::{CreateProductVariantRequest, UpdateProductVariantRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::product_variant::ProductVariant;
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait ProductVariantService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductVariantRequest,
    ) -> AppResult<ProductVariant>;

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductVariantRequest,
    ) -> AppResult<ProductVariant>;

    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ProductVariant>;

    async fn list_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
    ) -> AppResult<Vec<ProductVariant>>;

    async fn list_active_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
    ) -> AppResult<Vec<ProductVariant>>;
}

pub struct ProductVariantServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl ProductVariantServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProductVariantService for ProductVariantServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductVariantRequest,
    ) -> AppResult<ProductVariant> {
        let product_id_parsed = crate::types::snowflake_id::parse_id(&req.product_id)?;
        let product =
            crate::models::product::find_by_id(&self.pool, product_id_parsed, auth.tenant_id())
                .await?
                .ok_or_else(|| AppError::not_found("product"))?;

        crate::models::product_variant::insert(
            &self.pool,
            &crate::commands::CreateProductVariantCmd {
                product_id: product.id,
                sku: req.sku,
                title: req.title,
                price: req.price,
                original_price: req.original_price,
                stock: req.stock.unwrap_or(0),
                attributes: req.attributes,
                image_url: req.image_url,
                weight: req.weight,
                sort_order: req.sort_order.unwrap_or(0),
                is_active: req.is_active.unwrap_or(true),
            },
            auth.tenant_id(),
        )
        .await
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductVariantRequest,
    ) -> AppResult<ProductVariant> {
        let existing = crate::models::product_variant::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_variant"))?;

        let updated = crate::models::product_variant::update(
            &self.pool,
            &crate::commands::UpdateProductVariantCmd {
                id: existing.id,
                sku: req.sku.or(existing.sku),
                title: req.title.unwrap_or(existing.title),
                price: req.price.unwrap_or(existing.price),
                original_price: req.original_price.or(existing.original_price),
                stock: req.stock.unwrap_or(existing.stock),
                attributes: req.attributes.or(existing.attributes),
                image_url: req.image_url.or(existing.image_url),
                weight: req.weight.or(existing.weight),
                sort_order: req.sort_order.unwrap_or(existing.sort_order),
                is_active: req.is_active.unwrap_or(existing.is_active),
            },
            auth.tenant_id(),
        )
        .await?;

        if !updated {
            return Err(AppError::not_found("product_variant"));
        }

        crate::models::product_variant::find_by_id(&self.pool, existing.id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_variant"))
    }

    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        let existing = crate::models::product_variant::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_variant"))?;

        crate::models::product_variant::delete_by_id(&self.pool, existing.id, auth.tenant_id())
            .await?;

        Ok(())
    }

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ProductVariant> {
        crate::models::product_variant::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_variant"))
    }

    async fn list_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
    ) -> AppResult<Vec<ProductVariant>> {
        let pid = crate::types::snowflake_id::parse_id(product_id)?;
        let product = crate::models::product::find_by_id(&self.pool, pid, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        crate::models::product_variant::find_by_product_id(&self.pool, product.id, auth.tenant_id())
            .await
    }

    async fn list_active_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
    ) -> AppResult<Vec<ProductVariant>> {
        let pid = crate::types::snowflake_id::parse_id(product_id)?;
        let product = crate::models::product::find_by_id(&self.pool, pid, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        crate::models::product_variant::find_active_by_product_id(
            &self.pool,
            product.id,
            auth.tenant_id(),
        )
        .await
    }
}
