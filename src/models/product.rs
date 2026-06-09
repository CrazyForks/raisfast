use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::{CreateProductCmd, UpdateProductCmd};
use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    ProductType {
        VirtualCredit = "virtual_credit",
        Membership = "membership",
        ContentPaywall = "content_paywall",
        License = "license",
        Download = "download",
        Physical = "physical",
        Custom = "custom",
    }
);

define_enum!(
    FulfillmentType {
        Digital = "digital",
        Physical = "physical",
    }
);

define_enum!(
    ProductStatus {
        Draft = "draft",
        Active = "active",
        Archived = "archived",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Product {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub category_id: Option<SnowflakeId>,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub product_type: ProductType,
    pub fulfillment_type: FulfillmentType,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub shipping_template_id: Option<SnowflakeId>,
    pub price: i64,
    pub currency: String,
    pub status: ProductStatus,
    pub attributes: Option<String>,
    pub sort_order: i64,
    pub slug: Option<String>,
    pub content: Option<String>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub image_ids: Option<String>,
    pub original_price: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub specs: Option<String>,
    pub unit: String,
    pub min_purchase: i64,
    pub max_purchase: Option<i64>,
    pub total_sales: i64,
    pub virtual_sales: i64,
    pub meta_title: Option<String>,
    pub meta_description: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub published_at: Option<Timestamp>,
    pub stock: i64,
    pub cost_price: Option<i64>,
    pub sale_price: Option<i64>,
    pub has_variants: bool,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_slug(
    pool: &crate::db::Pool,
    slug: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Product>> {
    raisfast_derive::crud_find!(
        pool, "products", Product,
        where: AND(("slug", slug), ("status", "active")),
        tenant: tenant_id
    )
    .map_err(Into::into)
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Product>> {
    raisfast_derive::crud_find!(pool, "products", Product, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_active_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Product>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, Product,
        table: "products",
        where: ("status", "active"),
        order_by: "sort_order, created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_all_admin(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
    keyword: Option<&str>,
    category_id: Option<&str>,
) -> AppResult<(Vec<Product>, i64)> {
    let has_kw = keyword.is_some_and(|k| !k.is_empty());
    if has_kw || category_id.is_some() {
        let offset = (page - 1).saturating_mul(page_size);
        let kw_pattern = keyword.filter(|k| !k.is_empty()).map(|k| format!("%{k}%"));

        let mut conditions = vec!["1=1".to_string()];
        let mut ph_idx = 1usize;

        if status.is_some() {
            let ph = crate::db::Driver::ph(ph_idx);
            ph_idx += 1;
            conditions.push(format!("status = {ph}"));
        }

        if kw_pattern.is_some() {
            let ph1 = crate::db::Driver::ph(ph_idx);
            let ph2 = crate::db::Driver::ph(ph_idx + 1);
            ph_idx += 2;
            conditions.push(format!("(title LIKE {ph1} OR description LIKE {ph2})"));
        }

        if category_id.is_some() {
            let ph = crate::db::Driver::ph(ph_idx);
            ph_idx += 1;
            conditions.push(format!("category_id = {ph}"));
        }

        let tf = crate::db::tenant::tenant_filter_ph(tenant_id, ph_idx);
        ph_idx += if tenant_id.is_some() { 1 } else { 0 };

        let where_clause = conditions.join(" AND ");
        let limit_ph = crate::db::Driver::ph(ph_idx);
        let offset_ph = crate::db::Driver::ph(ph_idx + 1);

        let sql = format!(
            "SELECT * FROM products WHERE {where_clause}{tf} ORDER BY sort_order, created_at DESC LIMIT {limit_ph} OFFSET {offset_ph}"
        );
        let mut q = sqlx::query_as::<_, Product>(&sql);
        if let Some(s) = status {
            q = q.bind(s);
        }
        if let Some(p) = &kw_pattern {
            q = q.bind(p).bind(p);
        }
        if let Some(c) = category_id {
            q = q.bind(c);
        }
        if let Some(tid) = tenant_id {
            q = q.bind(tid);
        }
        let items = q.bind(page_size).bind(offset).fetch_all(pool).await?;

        let count_sql = format!("SELECT COUNT(*) FROM products WHERE {where_clause}{tf}");
        let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql);
        if let Some(s) = status {
            cq = cq.bind(s);
        }
        if let Some(p) = &kw_pattern {
            cq = cq.bind(p).bind(p);
        }
        if let Some(c) = category_id {
            cq = cq.bind(c);
        }
        if let Some(tid) = tenant_id {
            cq = cq.bind(tid);
        }
        let total = cq.fetch_one(pool).await?;
        return Ok((items, total.0));
    }

    let result = raisfast_derive::crud_query_paged!(
        pool, Product,
        table: "products",
        where: ["status" => status],
        order_by: "sort_order, created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &CreateProductCmd,
    tenant_id: Option<&str>,
) -> AppResult<Product> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "products",
        [
            "id" => id,
            "category_id" => cmd.category_id,
            "title" => &cmd.title,
            "description" => &cmd.description,
            "cover_url" => &cmd.cover_url,
            "product_type" => &cmd.product_type,
            "fulfillment_type" => &cmd.fulfillment_type,
            "delivery_hook" => &cmd.delivery_hook,
            "weight" => cmd.weight,
            "price" => cmd.price,
            "currency" => &cmd.currency,
            "attributes" => &cmd.attributes,
            "sort_order" => cmd.sort_order,
            "slug" => &cmd.slug,
            "content" => &cmd.content,
            "image_ids" => &cmd.image_ids,
            "original_price" => cmd.original_price,
            "specs" => &cmd.specs,
            "unit" => &cmd.unit,
            "min_purchase" => cmd.min_purchase,
            "max_purchase" => cmd.max_purchase,
            "virtual_sales" => cmd.virtual_sales,
            "meta_title" => &cmd.meta_title,
            "meta_description" => &cmd.meta_description,
            "og_title" => &cmd.og_title,
            "og_description" => &cmd.og_description,
            "og_image" => &cmd.og_image,
            "stock" => cmd.stock,
            "cost_price" => cmd.cost_price,
            "sale_price" => cmd.sale_price,
            "has_variants" => cmd.has_variants,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("product not found after insert")))
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &UpdateProductCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(
        pool, "products",
        bind: [
            "category_id" => cmd.category_id,
            "title" => &cmd.title,
            "description" => &cmd.description,
            "cover_url" => &cmd.cover_url,
            "product_type" => &cmd.product_type,
            "fulfillment_type" => &cmd.fulfillment_type,
            "delivery_hook" => &cmd.delivery_hook,
            "weight" => cmd.weight,
            "price" => cmd.price,
            "currency" => &cmd.currency,
            "status" => &cmd.status,
            "attributes" => &cmd.attributes,
            "sort_order" => cmd.sort_order,
            "slug" => &cmd.slug,
            "content" => &cmd.content,
            "image_ids" => &cmd.image_ids,
            "original_price" => cmd.original_price,
            "specs" => &cmd.specs,
            "unit" => &cmd.unit,
            "min_purchase" => cmd.min_purchase,
            "max_purchase" => cmd.max_purchase,
            "total_sales" => cmd.total_sales,
            "virtual_sales" => cmd.virtual_sales,
            "meta_title" => &cmd.meta_title,
            "meta_description" => &cmd.meta_description,
            "og_title" => &cmd.og_title,
            "og_description" => &cmd.og_description,
            "og_image" => &cmd.og_image,
            "published_at" => &cmd.published_at,
            "stock" => cmd.stock,
            "cost_price" => cmd.cost_price,
            "sale_price" => cmd.sale_price,
            "has_variants" => cmd.has_variants,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn(), "version" => "version + 1"],
        where: AND(("id", cmd.id), ("version", cmd.version)),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::pool::DbQueryResult =
        raisfast_derive::crud_delete!(pool, "products", where: ("id", id), tenant: tenant_id)?;
    Ok(result.rows_affected() > 0)
}

pub async fn tx_deduct_stock(
    tx: &mut crate::db::pool::DbConnection,
    product_id: SnowflakeId,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!(
        "products",
        "stock",
        "version",
        "updated_at",
        "id",
        "tenant_id"
    );
    let sql = if tenant_id.is_some() {
        format!(
            "UPDATE products SET stock = stock - {}, version = version + 1, updated_at = {} WHERE id = {} AND stock >= {} AND tenant_id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3),
            crate::db::Driver::ph(4)
        )
    } else {
        format!(
            "UPDATE products SET stock = stock - {}, version = version + 1, updated_at = {} WHERE id = {} AND stock >= {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3)
        )
    };
    let mut q = sqlx::query(&sql)
        .bind(quantity)
        .bind(product_id)
        .bind(quantity);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result: crate::db::pool::DbQueryResult = q.execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::BadRequest("insufficient_stock".into()));
    }
    Ok(())
}

pub async fn tx_replenish_stock(
    tx: &mut crate::db::pool::DbConnection,
    product_id: SnowflakeId,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!(
        "products",
        "stock",
        "version",
        "updated_at",
        "id",
        "tenant_id"
    );
    let sql = if tenant_id.is_some() {
        format!(
            "UPDATE products SET stock = stock + {}, version = version + 1, updated_at = {} WHERE id = {} AND tenant_id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3)
        )
    } else {
        format!(
            "UPDATE products SET stock = stock + {}, version = version + 1, updated_at = {} WHERE id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2)
        )
    };
    let mut q = sqlx::query(&sql).bind(quantity).bind(product_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result: crate::db::pool::DbQueryResult = q.execute(&mut *tx).await?;
    if result.rows_affected() == 0 {
        return Err(AppError::Internal(anyhow::anyhow!(
            "product not found for stock replenishment"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_product(pool: &crate::db::Pool, title: &str, _status: &str) -> Product {
        insert(
            pool,
            &CreateProductCmd {
                category_id: None,
                title: title.to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 1000,
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
                og_title: None,
                og_description: None,
                og_image: None,
                stock: 0,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    async fn set_status(pool: &crate::db::Pool, id: SnowflakeId, status: &str) {
        sqlx::query(&format!(
            "UPDATE products SET status = {} WHERE id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::ph(2)
        ))
        .bind(status)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn get_version(pool: &crate::db::Pool, id: SnowflakeId) -> i64 {
        let (v,): (i64,) = sqlx::query_as(&format!(
            "SELECT version FROM products WHERE id = {}",
            crate::db::Driver::ph(1)
        ))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        v
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let p = seed_product(&pool, "Widget", "draft").await;
        let found = super::find_by_id(&pool, p.id, None).await.unwrap().unwrap();
        assert_eq!(found.id, p.id);
        assert_eq!(found.title, "Widget");
        assert_eq!(found.price, 1000);
        assert_eq!(found.currency, "CNY");
        assert_eq!(found.version, 1);
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
        let p = super::insert(
            &pool,
            &CreateProductCmd {
                category_id: None,
                title: "Basic".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 500,
                currency: "USD".to_string(),
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
                og_title: None,
                og_description: None,
                og_image: None,
                stock: 0,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
            },
            None,
        )
        .await
        .unwrap();
        assert_eq!(p.product_type, ProductType::Custom);
        assert_eq!(p.fulfillment_type, FulfillmentType::Digital);
        assert_eq!(p.status, ProductStatus::Draft);
        assert_eq!(p.sort_order, 0);
        assert_eq!(p.version, 1);
        assert_eq!(p.tenant_id, Some("default".to_string()));
    }

    #[tokio::test]
    async fn update_changes_title_and_price() {
        let pool = setup_pool().await;
        let p = seed_product(&pool, "Old", "draft").await;
        let version = get_version(&pool, p.id).await;
        let ok = super::update(
            &pool,
            &UpdateProductCmd {
                id: p.id,
                category_id: None,
                title: "New".to_string(),
                description: Some("desc".to_string()),
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 2000,
                currency: "CNY".to_string(),
                status: "active".to_string(),
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
                total_sales: 0,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                published_at: None,
                stock: 0,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
                version,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);
        let found = super::find_by_id(&pool, p.id, None).await.unwrap().unwrap();
        assert_eq!(found.title, "New");
        assert_eq!(found.price, 2000);
        assert_eq!(found.status, ProductStatus::Active);
        assert_eq!(found.description.unwrap(), "desc");
        assert_eq!(found.version, version + 1);
    }

    #[tokio::test]
    async fn update_version_conflict() {
        let pool = setup_pool().await;
        let p = seed_product(&pool, "Conflicting", "draft").await;
        let ok = super::update(
            &pool,
            &UpdateProductCmd {
                id: p.id,
                category_id: None,
                title: "New".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 1000,
                currency: "CNY".to_string(),
                status: "draft".to_string(),
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
                total_sales: 0,
                virtual_sales: 0,
                meta_title: None,
                meta_description: None,
                published_at: None,
                stock: 0,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
                og_title: None,
                og_description: None,
                og_image: None,
                version: 999,
            },
            None,
        )
        .await
        .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn delete_removes_product() {
        let pool = setup_pool().await;
        let p = seed_product(&pool, "Bye", "draft").await;
        let ok = super::delete_by_id(&pool, p.id, None).await.unwrap();
        assert!(ok);
        assert!(
            super::find_by_id(&pool, p.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_not_found() {
        let pool = setup_pool().await;
        let ok = super::delete_by_id(&pool, SnowflakeId(99999), None)
            .await
            .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn find_active_paginated_filters_status() {
        let pool = setup_pool().await;
        for i in 0..5 {
            let p = seed_product(&pool, &format!("P{i}"), "draft").await;
            set_status(&pool, p.id, "active").await;
        }
        let p = seed_product(&pool, "Draft", "draft").await;
        set_status(&pool, p.id, "draft").await;

        let (items, total) = super::find_active_paginated(&pool, None, 1, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|p| p.status == ProductStatus::Active));
    }

    #[tokio::test]
    async fn find_active_paginated_page_two() {
        let pool = setup_pool().await;
        for i in 0..5 {
            let p = seed_product(&pool, &format!("P{i}"), "draft").await;
            set_status(&pool, p.id, "active").await;
        }
        let (items, total) = super::find_active_paginated(&pool, None, 2, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 2);
    }

    #[tokio::test]
    async fn find_all_admin_no_filter() {
        let pool = setup_pool().await;
        for i in 0..4 {
            seed_product(&pool, &format!("P{i}"), "draft").await;
        }
        let (items, total) = super::find_all_admin(&pool, None, 1, 10, None, None, None)
            .await
            .unwrap();
        assert_eq!(total, 4);
        assert_eq!(items.len(), 4);
    }

    #[tokio::test]
    async fn find_all_admin_with_status_filter() {
        let pool = setup_pool().await;
        for i in 0..3 {
            let p = seed_product(&pool, &format!("Active{i}"), "draft").await;
            set_status(&pool, p.id, "active").await;
        }
        seed_product(&pool, "Draft1", "draft").await;

        let (items, total) = super::find_all_admin(&pool, None, 1, 10, Some("active"), None, None)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|p| p.status == ProductStatus::Active));
    }

    #[tokio::test]
    async fn find_active_paginated_empty() {
        let pool = setup_pool().await;
        let (items, total) = super::find_active_paginated(&pool, None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(items.is_empty());
    }

    async fn seed_product_with_stock(pool: &crate::db::Pool, stock: i64) -> Product {
        insert(
            pool,
            &CreateProductCmd {
                category_id: None,
                title: "StockTest".to_string(),
                description: None,
                cover_url: None,
                product_type: "custom".to_string(),
                fulfillment_type: "digital".to_string(),
                delivery_hook: None,
                weight: None,
                price: 1000,
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
                og_title: None,
                og_description: None,
                og_image: None,
                stock,
                cost_price: None,
                sale_price: None,
                has_variants: false,
                tag_ids: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    async fn get_stock(pool: &crate::db::Pool, id: SnowflakeId) -> i64 {
        let (s,): (i64,) = sqlx::query_as(&format!(
            "SELECT stock FROM products WHERE id = {}",
            crate::db::Driver::ph(1)
        ))
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        s
    }

    async fn deduct(
        pool: &crate::db::Pool,
        pid: SnowflakeId,
        qty: i64,
    ) -> crate::errors::app_error::AppResult<()> {
        crate::in_transaction!(pool, tx, {
            super::tx_deduct_stock(&mut tx, pid, qty, None).await
        })
    }

    async fn replenish(
        pool: &crate::db::Pool,
        pid: SnowflakeId,
        qty: i64,
    ) -> crate::errors::app_error::AppResult<()> {
        crate::in_transaction!(pool, tx, {
            super::tx_replenish_stock(&mut tx, pid, qty, None).await
        })
    }

    #[tokio::test]
    async fn tx_deduct_stock_success() {
        let pool = setup_pool().await;
        let p = seed_product_with_stock(&pool, 10).await;
        deduct(&pool, p.id, 3).await.unwrap();
        assert_eq!(get_stock(&pool, p.id).await, 7);
    }

    #[tokio::test]
    async fn tx_deduct_stock_exact() {
        let pool = setup_pool().await;
        let p = seed_product_with_stock(&pool, 5).await;
        deduct(&pool, p.id, 5).await.unwrap();
        assert_eq!(get_stock(&pool, p.id).await, 0);
    }

    #[tokio::test]
    async fn tx_deduct_stock_insufficient() {
        let pool = setup_pool().await;
        let p = seed_product_with_stock(&pool, 3).await;
        let err = deduct(&pool, p.id, 5).await.unwrap_err();
        assert!(
            matches!(err, crate::errors::app_error::AppError::BadRequest(ref s) if s == "insufficient_stock")
        );
        assert_eq!(get_stock(&pool, p.id).await, 3);
    }

    #[tokio::test]
    async fn tx_deduct_stock_not_found() {
        let pool = setup_pool().await;
        let err = deduct(&pool, SnowflakeId(99999), 1).await.unwrap_err();
        assert!(
            matches!(err, crate::errors::app_error::AppError::BadRequest(ref s) if s == "insufficient_stock")
        );
    }

    #[tokio::test]
    async fn tx_replenish_stock_success() {
        let pool = setup_pool().await;
        let p = seed_product_with_stock(&pool, 5).await;
        replenish(&pool, p.id, 3).await.unwrap();
        assert_eq!(get_stock(&pool, p.id).await, 8);
    }

    #[tokio::test]
    async fn tx_replenish_stock_not_found() {
        let pool = setup_pool().await;
        let err = replenish(&pool, SnowflakeId(99999), 1).await.unwrap_err();
        assert!(matches!(
            err,
            crate::errors::app_error::AppError::Internal(_)
        ));
    }
}
