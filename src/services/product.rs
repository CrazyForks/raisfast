//! Product service.

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::engine::AspectEngine;
use crate::aspects::slug_aspect;
use crate::commands::{CreateProductCmd, UpdateProductCmd};
use crate::dto::{CreateProductRequest, UpdateProductRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::event::Event;
use crate::middleware::auth::AuthUser;
use crate::models::product::Product;
use crate::repositories::ProductRepository;

/// Product business logic trait.
#[async_trait]
pub trait ProductService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateProductRequest) -> AppResult<Product>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: &str,
        req: UpdateProductRequest,
    ) -> AppResult<Product>;
    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()>;
    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Product>;
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
    ) -> AppResult<(Vec<Product>, i64)>;
}

pub struct ProductServiceImpl {
    repo: Arc<dyn ProductRepository>,
    aspect_engine: Arc<AspectEngine>,
}

impl ProductServiceImpl {
    pub fn new(repo: Arc<dyn ProductRepository>, aspect_engine: Arc<AspectEngine>) -> Self {
        Self {
            repo,
            aspect_engine,
        }
    }

    async fn before_create(
        &self,
        auth: &AuthUser,
        req: CreateProductRequest,
    ) -> AppResult<(CreateProductRequest, crate::aspects::Dispatched)> {
        self.aspect_engine
            .before_create("products", auth, req)
            .await
    }

    async fn before_update(
        &self,
        auth: &AuthUser,
        existing: &Product,
        req: UpdateProductRequest,
    ) -> AppResult<(UpdateProductRequest, crate::aspects::Dispatched)> {
        self.aspect_engine
            .before_update("products", auth, existing, req)
            .await
    }

    async fn before_delete(
        &self,
        auth: &AuthUser,
        existing: &Product,
    ) -> AppResult<crate::aspects::Dispatched> {
        self.aspect_engine
            .before_delete("products", auth, existing)
            .await
    }

    fn after_created(&self, p: &Product) {
        self.aspect_engine.emit(Event::ProductCreated(p.clone()));
    }

    fn after_updated(&self, p: &Product) {
        self.aspect_engine.emit(Event::ProductUpdated(p.clone()));
    }

    fn after_deleted(&self, p: &Product) {
        self.aspect_engine.emit(Event::ProductDeleted(p.clone()));
    }
}

#[async_trait]
impl ProductService for ProductServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateProductRequest) -> AppResult<Product> {
        let (req, _d) = self.before_create(auth, req).await?;
        let product_type = req.product_type.as_deref().unwrap_or("custom");
        let fulfillment_type = req.fulfillment_type.as_deref().unwrap_or("digital");
        let currency = req.currency.as_deref().unwrap_or("CNY");
        let generated_slug = slug_aspect::generate_slug(&req.title);
        let slug = req.slug.as_deref().or(Some(generated_slug.as_str()));
        let p = self
            .repo
            .insert(
                &CreateProductCmd {
                    category_id: None,
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
                },
                auth.tenant_id(),
            )
            .await?;
        self.after_created(&p);
        Ok(p)
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: &str,
        req: UpdateProductRequest,
    ) -> AppResult<Product> {
        let existing = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
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

        let existing_published_at_str = existing
            .published_at
            .map(|t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string());
        let generated_published_at;
        let published_at: Option<&str> = if status == "active"
            && existing.status.as_str() != "active"
            && existing.published_at.is_none()
        {
            generated_published_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
            Some(generated_published_at.as_str())
        } else {
            existing_published_at_str.as_deref()
        };

        let updated = self
            .repo
            .update(
                &UpdateProductCmd {
                    id: existing.id,
                    category_id: None,
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
                    published_at: published_at.map(|s| s.to_string()),
                    version: req.version,
                },
                auth.tenant_id(),
            )
            .await?;

        if !updated {
            return Err(AppError::Conflict("version_conflict".into()));
        }

        let result = self
            .repo
            .find_by_id(existing.id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;
        self.after_updated(&result);
        Ok(result)
    }

    async fn delete(&self, id: &str, auth: &AuthUser) -> AppResult<()> {
        let existing = self
            .repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;
        self.before_delete(auth, &existing).await?;
        self.repo
            .delete_by_id(existing.id, auth.tenant_id())
            .await?;
        self.after_deleted(&existing);
        Ok(())
    }

    async fn get(&self, id: &str, auth: &AuthUser) -> AppResult<Product> {
        self.repo
            .find_by_document_id(id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))
    }

    async fn list_active(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<Product>, i64)> {
        self.repo
            .find_active_paginated(auth.tenant_id(), page, page_size)
            .await
    }

    async fn list_admin(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<Product>, i64)> {
        self.repo
            .find_all_admin(auth.tenant_id(), page, page_size, status)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::sqlx_order::SqlxProductRepository;

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

    fn make_service(pool: crate::db::Pool) -> Arc<dyn ProductService> {
        Arc::new(ProductServiceImpl::new(
            Arc::new(SqlxProductRepository::new(pool.clone())),
            Arc::new(AspectEngine::new()),
        ))
    }

    #[tokio::test]
    async fn create_product_basic() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
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
        let svc = make_service(pool.clone());
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
        let svc = make_service(pool.clone());
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
                },
            )
            .await
            .unwrap();
        let found = svc.get(&p.document_id, &a).await.unwrap();
        assert_eq!(found.id, p.id);
    }

    #[tokio::test]
    async fn get_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.get("nonexistent", &a).await.is_err());
    }

    #[tokio::test]
    async fn update_product_changes_title() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
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
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                &a,
                &p.document_id,
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
        let svc = make_service(pool.clone());
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
                },
            )
            .await
            .unwrap();
        let err = svc
            .update(
                &a,
                &p.document_id,
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
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn update_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        let err = svc
            .update(
                &a,
                "nonexistent",
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
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[tokio::test]
    async fn delete_product_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
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
                },
            )
            .await
            .unwrap();
        svc.delete(&p.document_id, &a).await.unwrap();
        assert!(svc.get(&p.document_id, &a).await.is_err());
    }

    #[tokio::test]
    async fn delete_product_not_found() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth(None);
        assert!(svc.delete("nonexistent", &a).await.is_err());
    }

    #[tokio::test]
    async fn list_active_products() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
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
        let svc = make_service(pool.clone());
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
            },
        )
        .await
        .unwrap();

        let (_, total_all) = svc.list_admin(&a, 1, 10, None).await.unwrap();
        assert_eq!(total_all, 2);
        let (active, total_active) = svc.list_admin(&a, 1, 10, Some("active")).await.unwrap();
        assert_eq!(total_active, 1);
        assert_eq!(active.len(), 1);
    }
}
