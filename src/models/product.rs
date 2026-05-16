use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::commands::{CreateProductCmd, UpdateProductCmd};
use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::{AppError, AppResult};
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
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Product {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub category_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub product_type: ProductType,
    pub fulfillment_type: FulfillmentType,
    pub delivery_hook: Option<String>,
    pub weight: Option<i64>,
    pub shipping_template_id: Option<i64>,
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
    pub published_at: Option<Timestamp>,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

crate::impl_from_row_opt_tenant!(Product {
    required { id, document_id, title, product_type, fulfillment_type, price, currency, status, sort_order, unit, min_purchase, total_sales, virtual_sales, version, created_at, updated_at }
    optional { category_id, description, cover_url, delivery_hook, weight, shipping_template_id, attributes, slug, content, image_ids, original_price, specs, max_purchase, meta_title, meta_description, published_at }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<Product>> {
    let sql = format!(
        "SELECT * FROM products WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Product>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Product>> {
    let sql = format!(
        "SELECT * FROM products WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, Product>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_active_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Product>, i64)> {
    let offset = (page - 1) * page_size;
    let count_sql = format!(
        "SELECT COUNT(*) as count FROM products WHERE status = 'active'{}",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut cq = sqlx::query_as::<_, (i64,)>(&count_sql);
    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
    }
    let (total,): (i64,) = cq.fetch_one(pool).await?;
    let base = usize::from(tenant_id.is_some()) + 1;
    let sql = format!(
        "SELECT * FROM products WHERE status = 'active'{} ORDER BY sort_order, created_at DESC LIMIT {} OFFSET {}",
        tenant_filter_ph(tenant_id, 1),
        ph(base),
        ph(base + 1)
    );
    let mut dq = sqlx::query_as::<_, Product>(&sql);
    if let Some(tid) = tenant_id {
        dq = dq.bind(tid);
    }
    let rows = dq.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

pub async fn find_all_admin(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<Product>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 1);
    let has_tenant = tenant_id.is_some();
    let status_ph_idx = if has_tenant { 2 } else { 1 };
    let (count_sql, data_sql_base) = if let Some(_s) = status {
        (
            format!(
                "SELECT COUNT(*) as count FROM products WHERE status = {}{}",
                ph(status_ph_idx),
                tenant_ph
            ),
            format!(
                "SELECT * FROM products WHERE status = {}{} ORDER BY sort_order, created_at DESC",
                ph(status_ph_idx),
                tenant_ph
            ),
        )
    } else {
        (
            format!(
                "SELECT COUNT(*) as count FROM products WHERE 1=1{}",
                tenant_ph
            ),
            format!(
                "SELECT * FROM products WHERE 1=1{} ORDER BY sort_order, created_at DESC",
                tenant_ph
            ),
        )
    };
    let mut q = sqlx::query_as::<_, (i64,)>(&count_sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(ref s) = status {
        q = q.bind(s);
    }
    let (total,): (i64,) = q.fetch_one(pool).await?;
    let limit_base = status_ph_idx + usize::from(status.is_some());
    let sql = format!(
        "{} LIMIT {} OFFSET {}",
        data_sql_base,
        ph(limit_base + 1),
        ph(limit_base + 2)
    );
    let mut q2 = sqlx::query_as::<_, Product>(&sql);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    if let Some(s) = status {
        q2 = q2.bind(s);
    }
    let rows = q2.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &CreateProductCmd,
    tenant_id: Option<&str>,
) -> AppResult<Product> {
    let document_id = uuid::Uuid::now_v7().to_string();
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO products (document_id, tenant_id, category_id, title, description, cover_url, product_type, fulfillment_type, delivery_hook, weight, price, currency, attributes, sort_order, slug, content, image_ids, original_price, specs, unit, min_purchase, max_purchase, virtual_sales, meta_title, meta_description, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11),
                ph(12),
                ph(13),
                ph(14),
                ph(15),
                ph(16),
                ph(17),
                ph(18),
                ph(19),
                ph(20),
                ph(21),
                ph(22),
                ph(23),
                ph(24),
                ph(25)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(tid)
                .bind(cmd.category_id)
                .bind(&cmd.title)
                .bind(&cmd.description)
                .bind(&cmd.cover_url)
                .bind(&cmd.product_type)
                .bind(&cmd.fulfillment_type)
                .bind(&cmd.delivery_hook)
                .bind(cmd.weight)
                .bind(cmd.price)
                .bind(&cmd.currency)
                .bind(&cmd.attributes)
                .bind(cmd.sort_order)
                .bind(&cmd.slug)
                .bind(&cmd.content)
                .bind(&cmd.image_ids)
                .bind(cmd.original_price)
                .bind(&cmd.specs)
                .bind(&cmd.unit)
                .bind(cmd.min_purchase)
                .bind(cmd.max_purchase)
                .bind(cmd.virtual_sales)
                .bind(&cmd.meta_title)
                .bind(&cmd.meta_description)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO products (document_id, category_id, title, description, cover_url, product_type, fulfillment_type, delivery_hook, weight, price, currency, attributes, sort_order, slug, content, image_ids, original_price, specs, unit, min_purchase, max_purchase, virtual_sales, meta_title, meta_description, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10),
                ph(11),
                ph(12),
                ph(13),
                ph(14),
                ph(15),
                ph(16),
                ph(17),
                ph(18),
                ph(19),
                ph(20),
                ph(21),
                ph(22),
                ph(23),
                ph(24)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(cmd.category_id)
                .bind(&cmd.title)
                .bind(&cmd.description)
                .bind(&cmd.cover_url)
                .bind(&cmd.product_type)
                .bind(&cmd.fulfillment_type)
                .bind(&cmd.delivery_hook)
                .bind(cmd.weight)
                .bind(cmd.price)
                .bind(&cmd.currency)
                .bind(&cmd.attributes)
                .bind(cmd.sort_order)
                .bind(&cmd.slug)
                .bind(&cmd.content)
                .bind(&cmd.image_ids)
                .bind(cmd.original_price)
                .bind(&cmd.specs)
                .bind(&cmd.unit)
                .bind(cmd.min_purchase)
                .bind(cmd.max_purchase)
                .bind(cmd.virtual_sales)
                .bind(&cmd.meta_title)
                .bind(&cmd.meta_description)
                .execute(pool)
                .await?;
        }
    }
    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "product not found after insert: {document_id}"
            ))
        })
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &UpdateProductCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let sql = format!(
        "UPDATE products SET category_id={}, title={}, description={}, cover_url={}, product_type={}, fulfillment_type={}, delivery_hook={}, weight={}, price={}, currency={}, status={}, attributes={}, sort_order={}, slug={}, content={}, image_ids={}, original_price={}, specs={}, unit={}, min_purchase={}, max_purchase={}, total_sales={}, virtual_sales={}, meta_title={}, meta_description={}, published_at={}, updated_at=datetime('now'), version=version+1 WHERE id={} AND version={}{}",
        ph(1),
        ph(2),
        ph(3),
        ph(4),
        ph(5),
        ph(6),
        ph(7),
        ph(8),
        ph(9),
        ph(10),
        ph(11),
        ph(12),
        ph(13),
        ph(14),
        ph(15),
        ph(16),
        ph(17),
        ph(18),
        ph(19),
        ph(20),
        ph(21),
        ph(22),
        ph(23),
        ph(24),
        ph(25),
        ph(26),
        ph(27),
        ph(28),
        tenant_filter_ph(tenant_id, 29)
    );
    let mut q = sqlx::query(&sql)
        .bind(cmd.category_id)
        .bind(&cmd.title)
        .bind(&cmd.description)
        .bind(&cmd.cover_url)
        .bind(&cmd.product_type)
        .bind(&cmd.fulfillment_type)
        .bind(&cmd.delivery_hook)
        .bind(cmd.weight)
        .bind(cmd.price)
        .bind(&cmd.currency)
        .bind(&cmd.status)
        .bind(&cmd.attributes)
        .bind(cmd.sort_order)
        .bind(&cmd.slug)
        .bind(&cmd.content)
        .bind(&cmd.image_ids)
        .bind(cmd.original_price)
        .bind(&cmd.specs)
        .bind(&cmd.unit)
        .bind(cmd.min_purchase)
        .bind(cmd.max_purchase)
        .bind(cmd.total_sales)
        .bind(cmd.virtual_sales)
        .bind(&cmd.meta_title)
        .bind(&cmd.meta_description)
        .bind(&cmd.published_at)
        .bind(cmd.id)
        .bind(cmd.version);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let affected = q.execute(pool).await?.rows_affected();
    Ok(affected > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let sql = format!(
        "DELETE FROM products WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let affected = q.execute(pool).await?.rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            },
            None,
        )
        .await
        .unwrap()
    }

    async fn set_status(pool: &crate::db::Pool, id: i64, status: &str) {
        sqlx::query("UPDATE products SET status = ? WHERE id = ?")
            .bind(status)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn get_version(pool: &crate::db::Pool, id: i64) -> i64 {
        let (v,): (i64,) = sqlx::query_as("SELECT version FROM products WHERE id = ?")
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
    async fn find_by_document_id() {
        let pool = setup_pool().await;
        let p = seed_product(&pool, "Gadget", "draft").await;
        let found = super::find_by_document_id(&pool, &p.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, p.id);
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_id(&pool, 99999, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_document_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_document_id(&pool, "nonexistent", None)
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
        assert!(p.tenant_id.is_none());
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
        let ok = super::delete_by_id(&pool, 99999, None).await.unwrap();
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
        let (items, total) = super::find_all_admin(&pool, None, 1, 10, None)
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

        let (items, total) = super::find_all_admin(&pool, None, 1, 10, Some("active"))
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
}
