use crate::dto::{CreateProductRequest, UpdateProductRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::product::Product;
use crate::repositories::ProductRepository;

pub async fn create_product(
    product_repo: &dyn ProductRepository,
    auth: &AuthUser,
    req: CreateProductRequest,
) -> AppResult<Product> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let product_type = req.product_type.as_deref().unwrap_or("custom");
    let fulfillment_type = req.fulfillment_type.as_deref().unwrap_or("digital");
    let currency = req.currency.as_deref().unwrap_or("CNY");
    let generated_slug = generate_slug(&req.title);
    let slug = req.slug.as_deref().or(Some(generated_slug.as_str()));
    product_repo
        .insert(
            &document_id,
            None,
            &req.title,
            req.description.as_deref(),
            req.cover_url.as_deref(),
            product_type,
            fulfillment_type,
            req.delivery_hook.as_deref(),
            req.weight,
            req.price,
            currency,
            req.attributes.as_deref(),
            req.sort_order.unwrap_or(0),
            slug,
            req.content.as_deref(),
            req.image_ids.as_deref(),
            req.original_price,
            req.specs.as_deref(),
            req.unit.as_deref().unwrap_or("piece"),
            req.min_purchase.unwrap_or(1),
            req.max_purchase,
            req.virtual_sales.unwrap_or(0),
            req.meta_title.as_deref(),
            req.meta_description.as_deref(),
            auth.tenant_id(),
        )
        .await
}

fn generate_slug(title: &str) -> String {
    let lower = title.to_lowercase();
    let slug: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    slug.split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

pub async fn update_product(
    product_repo: &dyn ProductRepository,
    auth: &AuthUser,
    id: &str,
    req: UpdateProductRequest,
) -> AppResult<Product> {
    let existing = product_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("product"))?;

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

    let updated = product_repo
        .update(
            existing.id,
            None,
            title,
            req.description
                .as_deref()
                .or(existing.description.as_deref()),
            req.cover_url.as_deref().or(existing.cover_url.as_deref()),
            product_type,
            fulfillment_type,
            req.delivery_hook
                .as_deref()
                .or(existing.delivery_hook.as_deref()),
            req.weight.or(existing.weight),
            price,
            currency,
            status,
            req.attributes.as_deref().or(existing.attributes.as_deref()),
            sort_order,
            slug,
            req.content.as_deref().or(existing.content.as_deref()),
            req.image_ids.as_deref().or(existing.image_ids.as_deref()),
            req.original_price.or(existing.original_price),
            req.specs.as_deref().or(existing.specs.as_deref()),
            unit,
            min_purchase,
            req.max_purchase.or(existing.max_purchase),
            total_sales,
            virtual_sales,
            req.meta_title.as_deref().or(existing.meta_title.as_deref()),
            req.meta_description
                .as_deref()
                .or(existing.meta_description.as_deref()),
            published_at,
            req.version,
            auth.tenant_id(),
        )
        .await?;

    if !updated {
        return Err(AppError::Conflict("version_conflict".into()));
    }

    product_repo
        .find_by_id(existing.id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("product"))
}

pub async fn delete_product(
    product_repo: &dyn ProductRepository,
    id: &str,
    auth: &AuthUser,
) -> AppResult<()> {
    let existing = product_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("product"))?;
    product_repo
        .delete_by_id(existing.id, auth.tenant_id())
        .await?;
    Ok(())
}

pub async fn get_product(
    product_repo: &dyn ProductRepository,
    id: &str,
    auth: &AuthUser,
) -> AppResult<Product> {
    product_repo
        .find_by_document_id(id, auth.tenant_id())
        .await?
        .ok_or_else(|| AppError::not_found("product"))
}

pub async fn list_active_products(
    product_repo: &dyn ProductRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Product>, i64)> {
    product_repo
        .find_active_paginated(auth.tenant_id(), page, page_size)
        .await
}

pub async fn list_admin_products(
    product_repo: &dyn ProductRepository,
    auth: &AuthUser,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<Product>, i64)> {
    product_repo
        .find_all_admin(auth.tenant_id(), page, page_size, status)
        .await
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

    #[tokio::test]
    async fn create_product_basic() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        let found = super::get_product(&repo, &p.document_id, &a).await.unwrap();
        assert_eq!(found.id, p.id);
    }

    #[tokio::test]
    async fn get_product_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        assert!(super::get_product(&repo, "nonexistent", &a).await.is_err());
    }

    #[tokio::test]
    async fn update_product_changes_title() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        let updated = super::update_product(
            &repo,
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
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        let err = super::update_product(
            &repo,
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
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let err = super::update_product(
            &repo,
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
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        super::delete_product(&repo, &p.document_id, &a)
            .await
            .unwrap();
        assert!(super::get_product(&repo, &p.document_id, &a).await.is_err());
    }

    #[tokio::test]
    async fn delete_product_not_found() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        assert!(
            super::delete_product(&repo, "nonexistent", &a)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn list_active_products() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        for i in 0..3 {
            let p = super::create_product(
                &repo,
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
        let (items, total) = super::list_active_products(&repo, &a, 1, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn list_admin_products_with_filter() {
        let pool = setup_pool().await;
        let repo = SqlxProductRepository::new(pool.clone());
        let a = auth(None);
        let p = super::create_product(
            &repo,
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
        super::create_product(
            &repo,
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

        let (all, total_all) = super::list_admin_products(&repo, &a, 1, 10, None)
            .await
            .unwrap();
        assert_eq!(total_all, 2);
        let (active, total_active) = super::list_admin_products(&repo, &a, 1, 10, Some("active"))
            .await
            .unwrap();
        assert_eq!(total_active, 1);
        assert_eq!(active.len(), 1);
    }
}
