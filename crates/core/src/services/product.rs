//! Product service.

use std::sync::Arc;

use async_trait::async_trait;
use raisfast_derive::aspect_service;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreateProductCmd, UpdateProductCmd};
use crate::dto::{CreateProductRequest, UpdateProductRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::product::Product;
use crate::services::options::OptionsService;
use crate::types::snowflake_id::SnowflakeId;

/// Product business logic trait.
#[async_trait]
pub trait ProductService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateProductRequest) -> AppResult<Product>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductRequest,
    ) -> AppResult<Product>;
    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Product>;
    async fn list_active(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Product>, i64)>;
    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        keyword: Option<&str>,
        category_id: Option<&str>,
    ) -> AppResult<(Vec<Product>, i64)>;
    async fn batch(&self, auth: &AuthUser, action: &str, ids: &[String]) -> AppResult<usize>;
}

#[aspect_service(entity = "products", model = Product)]
pub struct ProductServiceImpl {
    #[engine]
    aspect_engine: Arc<AspectEngine>,
    pool: Arc<crate::db::Pool>,
    options: Arc<OptionsService>,
}

#[async_trait]
impl ProductService for ProductServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateProductRequest) -> AppResult<Product> {
        let (req, _d) = self.before_create(auth, req).await?;
        let product_type = req.product_type.as_deref().unwrap_or("custom");
        let fulfillment_type = req.fulfillment_type.as_deref().unwrap_or("digital");
        let default_currency = self
            .options
            .get(auth.tenant_id(), "default_currency")
            .await
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "USD".to_string());
        let currency = req.currency.as_deref().unwrap_or(&default_currency);
        crate::models::currencies::ensure_active(&self.pool, currency, auth.tenant_id()).await?;
        let generated_slug = slug_aspect::generate_slug(&req.title);
        let slug = req.slug.as_deref().or(Some(generated_slug.as_str()));
        let category_id = resolve_category_id(&self.pool, auth, req.category_id.as_deref()).await?;
        let tag_ids = parse_tag_ids(&self.pool, req.tag_ids.as_deref(), auth.tenant_id()).await?;
        let p = crate::models::product::insert(
            &self.pool,
            &CreateProductCmd {
                category_id,
                title: req.title,
                description: req.description,
                cover_url: req.cover_url,
                product_type: product_type.to_string(),
                fulfillment_type: fulfillment_type.to_string(),
                delivery_hook: req.delivery_hook,
                weight: req.weight,
                price: req.price,
                currency: currency.to_string(),
                attributes: req.attributes,
                sort_order: req.sort_order.unwrap_or(0),
                slug: slug.map(|s| s.to_string()),
                content: req.content,
                image_ids: req.image_ids,
                original_price: req.original_price,
                specs: req.specs,
                unit: req.unit.as_deref().unwrap_or("piece").to_string(),
                min_purchase: req.min_purchase.unwrap_or(1),
                max_purchase: req.max_purchase,
                virtual_sales: req.virtual_sales.unwrap_or(0),
                meta_title: req.meta_title,
                meta_description: req.meta_description,
                og_title: req.og_title,
                og_description: req.og_description,
                og_image: req.og_image,
                stock: req.stock.unwrap_or(0),
                cost_price: req.cost_price,
                sale_price: req.sale_price,
                has_variants: req.has_variants.unwrap_or(false),
                tag_ids: tag_ids.clone(),
            },
            auth.tenant_id(),
        )
        .await?;
        if let Some(ref ids) = tag_ids {
            let _guard = crate::db::connection::acquire_write().await;
            let mut tx = self.pool.begin().await?;
            crate::models::tagging::sync_tags_tx(&mut tx, "product", p.id, ids, auth.tenant_id())
                .await?;
            tx.commit().await?;
        }
        self.after_created(&p);
        Ok(p)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductRequest,
    ) -> AppResult<Product> {
        let existing = crate::models::product::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        let (req, _d) = self.before_update(auth, &existing, req).await?;

        let title = req.title.as_deref().unwrap_or(&existing.title);
        let product_type = req
            .product_type
            .as_deref()
            .unwrap_or(existing.product_type.as_str());
        let fulfillment_type = req
            .fulfillment_type
            .as_deref()
            .unwrap_or(existing.fulfillment_type.as_str());
        let currency = req.currency.as_deref().unwrap_or(&existing.currency);
        let status = req.status.as_deref().unwrap_or(existing.status.as_str());
        let price = req.price.unwrap_or(existing.price);
        let sort_order = req.sort_order.unwrap_or(existing.sort_order);
        let slug = req.slug.as_deref().or(existing.slug.as_deref());
        let unit = req.unit.as_deref().unwrap_or(&existing.unit);
        let min_purchase = req.min_purchase.unwrap_or(existing.min_purchase);
        let total_sales = existing.total_sales;
        let virtual_sales = req.virtual_sales.unwrap_or(existing.virtual_sales);
        let stock = req.stock.unwrap_or(existing.stock);
        let cost_price = req.cost_price.or(existing.cost_price);
        let sale_price = req.sale_price.or(existing.sale_price);
        let has_variants = req.has_variants.unwrap_or(existing.has_variants);

        let category_id = match req.category_id.as_deref() {
            Some(raw) if !raw.is_empty() => {
                resolve_category_id(&self.pool, auth, Some(raw)).await?
            }
            Some(_) => None,
            None => existing.category_id.map(|c| *c),
        };

        let published_at: Option<crate::utils::tz::Timestamp> = if status == "active"
            && existing.status.as_str() != "active"
            && existing.published_at.is_none()
        {
            Some(crate::utils::tz::now_utc())
        } else {
            existing.published_at
        };

        let updated = crate::models::product::update(
            &self.pool,
            &UpdateProductCmd {
                id: existing.id,
                category_id,
                title: title.to_string(),
                description: req.description.or(existing.description),
                cover_url: req.cover_url.or(existing.cover_url),
                product_type: product_type.to_string(),
                fulfillment_type: fulfillment_type.to_string(),
                delivery_hook: req.delivery_hook.or(existing.delivery_hook),
                weight: req.weight.or(existing.weight),
                price,
                currency: currency.to_string(),
                status: status.to_string(),
                attributes: req.attributes.or(existing.attributes),
                sort_order,
                slug: slug.map(|s| s.to_string()),
                content: req.content.or(existing.content),
                image_ids: req.image_ids.or(existing.image_ids),
                original_price: req.original_price.or(existing.original_price),
                specs: req.specs.or(existing.specs),
                unit: unit.to_string(),
                min_purchase,
                max_purchase: req.max_purchase.or(existing.max_purchase),
                total_sales,
                virtual_sales,
                meta_title: req.meta_title.or(existing.meta_title),
                meta_description: req.meta_description.or(existing.meta_description),
                og_title: req.og_title.or(existing.og_title),
                og_description: req.og_description.or(existing.og_description),
                og_image: req.og_image.or(existing.og_image),
                stock,
                cost_price,
                sale_price,
                has_variants,
                published_at,
                version: req.version,
                tag_ids: None,
            },
            auth.tenant_id(),
        )
        .await?;

        if !updated {
            return Err(AppError::Conflict("version_conflict".into()));
        }

        if let Some(ref raw) = req.tag_ids {
            let tag_ids = parse_tag_ids(&self.pool, Some(raw.as_str()), auth.tenant_id()).await?;
            if let Some(ids) = tag_ids {
                let _guard = crate::db::connection::acquire_write().await;
                let mut tx = self.pool.begin().await?;
                crate::models::tagging::sync_tags_tx(
                    &mut tx,
                    "product",
                    existing.id,
                    &ids,
                    auth.tenant_id(),
                )
                .await?;
                tx.commit().await?;
            }
        }

        let result = crate::models::product::find_by_id(&self.pool, existing.id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;
        self.after_updated(&result);
        Ok(result)
    }

    async fn delete(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<()> {
        let existing = crate::models::product::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;
        self.before_delete(auth, &existing).await?;
        crate::models::product::delete_by_id(&self.pool, existing.id, auth.tenant_id()).await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, id: SnowflakeId, auth: &AuthUser) -> AppResult<Product> {
        crate::models::product::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))
    }

    async fn list_active(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Product>, i64)> {
        crate::models::product::find_active_paginated(&self.pool, auth.tenant_id(), page, page_size)
            .await
    }

    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        keyword: Option<&str>,
        category_id: Option<&str>,
    ) -> AppResult<(Vec<Product>, i64)> {
        crate::models::product::find_all_admin(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            status,
            keyword,
            category_id,
        )
        .await
    }

    async fn batch(&self, auth: &AuthUser, action: &str, ids: &[String]) -> AppResult<usize> {
        let mut affected = 0usize;
        let target_status = match action {
            "delete" => None,
            "activate" => Some("active"),
            "deactivate" => Some("draft"),
            "archive" => Some("archived"),
            _ => return Ok(0),
        };

        for raw_id in ids {
            let Ok(parsed_id) = crate::types::snowflake_id::parse_id(raw_id) else {
                continue;
            };
            let Some(int_id) = raisfast_derive::crud_resolve_id!(&self.pool, "products", *parsed_id, tenant: auth.tenant_id())?
            else {
                continue;
            };

            if action == "delete" {
                if self.delete(SnowflakeId(int_id), auth).await.is_ok() {
                    affected += 1;
                }
                continue;
            }

            let Ok(Some(existing)) = crate::models::product::find_by_id(
                &self.pool,
                SnowflakeId(int_id),
                auth.tenant_id(),
            )
            .await
            else {
                continue;
            };

            let status = target_status.unwrap_or(existing.status.as_str());
            let published_at: Option<crate::utils::tz::Timestamp> = if status == "active"
                && existing.status.as_str() != "active"
                && existing.published_at.is_none()
            {
                Some(crate::utils::tz::now_utc())
            } else {
                existing.published_at
            };

            let cmd = UpdateProductCmd {
                id: existing.id,
                category_id: existing.category_id.map(|c| *c),
                title: existing.title,
                description: existing.description,
                cover_url: existing.cover_url,
                product_type: existing.product_type.to_string(),
                fulfillment_type: existing.fulfillment_type.to_string(),
                delivery_hook: existing.delivery_hook,
                weight: existing.weight,
                price: existing.price,
                currency: existing.currency,
                status: status.to_string(),
                attributes: existing.attributes,
                sort_order: existing.sort_order,
                slug: existing.slug,
                content: existing.content,
                image_ids: existing.image_ids,
                original_price: existing.original_price,
                specs: existing.specs,
                unit: existing.unit,
                min_purchase: existing.min_purchase,
                max_purchase: existing.max_purchase,
                total_sales: existing.total_sales,
                virtual_sales: existing.virtual_sales,
                meta_title: existing.meta_title,
                meta_description: existing.meta_description,
                og_title: existing.og_title,
                og_description: existing.og_description,
                og_image: existing.og_image,
                published_at,
                stock: existing.stock,
                cost_price: existing.cost_price,
                sale_price: existing.sale_price,
                has_variants: existing.has_variants,
                version: existing.version,
                tag_ids: None,
            };
            if crate::models::product::update(&self.pool, &cmd, auth.tenant_id())
                .await
                .is_ok()
            {
                affected += 1;
            }
        }
        Ok(affected)
    }
}

async fn resolve_category_id(
    pool: &crate::db::Pool,
    auth: &AuthUser,
    raw_id: Option<&str>,
) -> AppResult<Option<i64>> {
    match raw_id {
        Some(raw) if !raw.is_empty() => {
            let cid = crate::types::snowflake_id::parse_id(raw)?;
            let cat =
                crate::models::product_category::find_by_id(pool, cid, auth.tenant_id()).await?;
            Ok(Some(*cat.id))
        }
        _ => Ok(None),
    }
}

async fn parse_tag_ids(
    pool: &crate::db::Pool,
    raw: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<Option<Vec<i64>>> {
    let Some(raw) = raw else { return Ok(None) };
    let Ok(ids) = serde_json::from_str::<Vec<String>>(raw) else {
        return Ok(None);
    };
    let mut resolved = Vec::new();
    for raw_id in ids {
        let parsed = crate::types::snowflake_id::parse_id(&raw_id)?;
        if let Some(int_id) =
            raisfast_derive::crud_resolve_id!(pool, "tags", *parsed, tenant: tenant_id)?
        {
            resolved.push(int_id);
        }
    }
    Ok(Some(resolved))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn auth(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string())
                .or_else(|| Some("default".to_string())),
        )
    }

    async fn make_service(pool: crate::db::Pool) -> Arc<dyn ProductService> {
        Arc::new(ProductServiceImpl::new(
            Arc::new(AspectEngine::new()),
            Arc::new(pool),
            Arc::new(OptionsService::new(Arc::new(setup_pool().await), false).await),
        ))
    }

    #[tokio::test]
    async fn create_product_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Widget".into(),
                    description: Some("A nice widget".into()),
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 1000,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(p.title, "Widget");
        assert_eq!(p.price, 1000);
        assert_eq!(p.status, crate::models::product::ProductStatus::Draft);
    }

    #[tokio::test]
    async fn create_product_with_custom_type() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "E-Book".into(),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: Some("download".into()),
                    fulfillment_type: Some("digital".into()),
                    delivery_hook: None,
                    weight: None,
                    price: 500,
                    currency: Some("USD".into()),
                    attributes: None,
                    sort_order: Some(10),
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            p.product_type,
            crate::models::product::ProductType::Download
        );
        assert_eq!(
            p.fulfillment_type,
            crate::models::product::FulfillmentType::Digital
        );
        assert_eq!(p.currency, "USD");
        assert_eq!(p.sort_order, 10);
    }

    #[tokio::test]
    async fn get_product_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Found".into(),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 100,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        let found = svc.get(p.id, &a).await.unwrap();
        assert_eq!(found.id, p.id);
    }

    #[tokio::test]
    async fn get_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        assert!(svc.get(SnowflakeId(0), &a).await.is_err());
    }

    #[tokio::test]
    async fn update_product_changes_title() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Old".into(),
                    description: Some("old desc".into()),
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 100,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                &a,
                p.id,
                UpdateProductRequest {
                    title: Some("New".into()),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: None,
                    currency: None,
                    status: Some("active".into()),
                    attributes: None,
                    sort_order: None,
                    version: 1,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "New");
        assert_eq!(updated.description.unwrap(), "old desc");
        assert_eq!(updated.price, 100);
        assert_eq!(
            updated.status,
            crate::models::product::ProductStatus::Active
        );
        assert_eq!(updated.version, 2);
    }

    #[tokio::test]
    async fn update_product_version_conflict() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Conflict".into(),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 100,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        let err = svc
            .update(
                &a,
                p.id,
                UpdateProductRequest {
                    title: Some("New".into()),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: None,
                    currency: None,
                    status: None,
                    attributes: None,
                    sort_order: None,
                    version: 999,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn update_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let err = svc
            .update(
                &a,
                SnowflakeId(99999999),
                UpdateProductRequest {
                    title: Some("X".into()),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: None,
                    currency: None,
                    status: None,
                    attributes: None,
                    sort_order: None,
                    version: 1,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_product_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Bye".into(),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 100,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        svc.delete(p.id, &a).await.unwrap();
        assert!(svc.get(p.id, &a).await.is_err());
    }

    #[tokio::test]
    async fn delete_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        assert!(svc.delete(SnowflakeId(0), &a).await.is_err());
    }

    #[tokio::test]
    async fn list_active_products() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        for i in 0..3 {
            let p = svc
                .create(
                    &a,
                    CreateProductRequest {
                        title: format!("P{i}"),
                        description: None,
                        cover_url: None,
                        category_id: None,
                        product_type: None,
                        fulfillment_type: None,
                        delivery_hook: None,
                        weight: None,
                        price: 100,
                        currency: None,
                        attributes: None,
                        sort_order: None,
                        slug: None,
                        content: None,
                        image_ids: None,
                        original_price: None,
                        specs: None,
                        unit: None,
                        min_purchase: None,
                        max_purchase: None,
                        virtual_sales: None,
                        meta_title: None,
                        meta_description: None,
                        stock: None,
                        cost_price: None,
                        sale_price: None,
                        has_variants: None,
                        tag_ids: None,
                        og_title: None,
                        og_description: None,
                        og_image: None,
                    },
                )
                .await
                .unwrap();
            sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
                .bind(p.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let (items, total) = svc.list_active(&a, 1, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn list_admin_products_with_filter() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone()).await;
        let a = auth(None);
        let p = svc
            .create(
                &a,
                CreateProductRequest {
                    title: "Active".into(),
                    description: None,
                    cover_url: None,
                    category_id: None,
                    product_type: None,
                    fulfillment_type: None,
                    delivery_hook: None,
                    weight: None,
                    price: 100,
                    currency: None,
                    attributes: None,
                    sort_order: None,
                    slug: None,
                    content: None,
                    image_ids: None,
                    original_price: None,
                    specs: None,
                    unit: None,
                    min_purchase: None,
                    max_purchase: None,
                    virtual_sales: None,
                    meta_title: None,
                    meta_description: None,
                    og_title: None,
                    og_description: None,
                    og_image: None,
                    stock: None,
                    cost_price: None,
                    sale_price: None,
                    has_variants: None,
                    tag_ids: None,
                },
            )
            .await
            .unwrap();
        sqlx::query("UPDATE products SET status = 'active' WHERE id = ?")
            .bind(p.id)
            .execute(&pool)
            .await
            .unwrap();
        svc.create(
            &a,
            CreateProductRequest {
                title: "Draft".into(),
                description: None,
                cover_url: None,
                category_id: None,
                product_type: None,
                fulfillment_type: None,
                delivery_hook: None,
                weight: None,
                price: 100,
                currency: None,
                attributes: None,
                sort_order: None,
                slug: None,
                content: None,
                image_ids: None,
                original_price: None,
                specs: None,
                unit: None,
                min_purchase: None,
                max_purchase: None,
                virtual_sales: None,
                meta_title: None,
                meta_description: None,
                stock: None,
                cost_price: None,
                sale_price: None,
                has_variants: None,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
            },
        )
        .await
        .unwrap();

        let (_, total_all) = svc.list_admin(&a, 1, 10, None, None, None).await.unwrap();
        assert_eq!(total_all, 2);
        let (active, total_active) = svc
            .list_admin(&a, 1, 10, Some("active"), None, None)
            .await
            .unwrap();
        assert_eq!(total_active, 1);
        assert_eq!(active.len(), 1);
    }
}
