use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ProductVariant {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub product_id: SnowflakeId,
    pub sku: Option<String>,
    pub title: String,
    pub price: i64,
    pub original_price: Option<i64>,
    pub stock: i64,
    pub attributes: Option<String>,
    pub image_url: Option<String>,
    pub weight: Option<i64>,
    pub sort_order: i64,
    pub is_active: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ProductVariant>> {
    let result: Option<ProductVariant> = raisfast_derive::crud_find!(
        pool,
        "product_variants",
        ProductVariant,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_by_sku(
    pool: &crate::db::Pool,
    sku: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<ProductVariant>> {
    let result: Option<ProductVariant> = raisfast_derive::crud_find!(
        pool,
        "product_variants",
        ProductVariant,
        where: ("sku", sku),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_by_product_id(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ProductVariant>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "product_variants",
        ProductVariant,
        where: ("product_id", product_id),
        order_by: "sort_order, created_at",
        tenant: tenant_id
    )?)
}

pub async fn find_active_by_product_id(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<ProductVariant>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "product_variants",
        ProductVariant,
        where: AND(("product_id", product_id), ("is_active", true)),
        order_by: "sort_order, created_at",
        tenant: tenant_id
    )?)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateProductVariantCmd,
    tenant_id: Option<&str>,
) -> AppResult<ProductVariant> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "product_variants",
        [
            "id" => id,
            "product_id" => cmd.product_id,
            "sku" => &cmd.sku,
            "title" => &cmd.title,
            "price" => cmd.price,
            "original_price" => cmd.original_price,
            "stock" => cmd.stock,
            "attributes" => &cmd.attributes,
            "image_url" => &cmd.image_url,
            "weight" => cmd.weight,
            "sort_order" => cmd.sort_order,
            "is_active" => cmd.is_active,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id).await?.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("product_variant not found after insert"))
    })
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateProductVariantCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(
        pool,
        "product_variants",
        bind: [
            "sku" => &cmd.sku,
            "title" => &cmd.title,
            "price" => cmd.price,
            "original_price" => cmd.original_price,
            "stock" => cmd.stock,
            "attributes" => &cmd.attributes,
            "image_url" => &cmd.image_url,
            "weight" => cmd.weight,
            "sort_order" => cmd.sort_order,
            "is_active" => cmd.is_active,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", cmd.id),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let result: crate::db::DbQueryResult = raisfast_derive::crud_delete!(pool, "product_variants", where: ("id", id), tenant: tenant_id)?;
    Ok(result.rows_affected() > 0)
}

pub async fn delete_by_product_id(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "product_variants",
        where: ("product_id", product_id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "product_variant")
}

pub async fn count_by_product(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let count = raisfast_derive::crud_count!(
        pool,
        "product_variants",
        where: ("product_id", product_id),
        tenant: tenant_id
    )?;
    Ok(count)
}

pub async fn tx_deduct_stock(
    tx: &mut crate::db::pool::DbConnection,
    variant_id: SnowflakeId,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!("product_variants", "stock", "updated_at", "id", "tenant_id");
    let sql = if tenant_id.is_some() {
        format!(
            "UPDATE product_variants SET stock = stock - {}, updated_at = {} WHERE id = {} AND stock >= {} AND tenant_id = {} AND is_active = 1",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3),
            crate::db::Driver::ph(4)
        )
    } else {
        format!(
            "UPDATE product_variants SET stock = stock - {}, updated_at = {} WHERE id = {} AND stock >= {} AND is_active = 1",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3)
        )
    };
    let mut q = sqlx::query(&sql)
        .bind(quantity)
        .bind(variant_id)
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
    variant_id: SnowflakeId,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!("product_variants", "stock", "updated_at", "id", "tenant_id");
    let sql = if tenant_id.is_some() {
        format!(
            "UPDATE product_variants SET stock = stock + {}, updated_at = {} WHERE id = {} AND tenant_id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2),
            crate::db::Driver::ph(3)
        )
    } else {
        format!(
            "UPDATE product_variants SET stock = stock + {}, updated_at = {} WHERE id = {}",
            crate::db::Driver::ph(1),
            crate::db::Driver::now_fn(),
            crate::db::Driver::ph(2)
        )
    };
    let mut q = sqlx::query(&sql).bind(quantity).bind(variant_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.execute(&mut *tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_product(pool: &crate::db::Pool) -> i64 {
        crate::models::product::insert(
            pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Test Product".to_string(),
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
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM products ORDER BY id DESC LIMIT 1")
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_variant(
        pool: &crate::db::Pool,
        product_id: i64,
        title: &str,
        sku: &str,
    ) -> ProductVariant {
        insert(
            pool,
            &crate::commands::CreateProductVariantCmd {
                product_id: SnowflakeId(product_id),
                sku: Some(sku.to_string()),
                title: title.to_string(),
                price: 1000,
                original_price: None,
                stock: 50,
                attributes: Some(r#"{"color":"red"}"#.to_string()),
                image_url: None,
                weight: None,
                sort_order: 0,
                is_active: true,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        let v = seed_variant(&pool, pid, "Red Shirt", "SKU-RED-001").await;

        let found = super::find_by_id(&pool, v.id, None).await.unwrap().unwrap();
        assert_eq!(found.id, v.id);
        assert_eq!(found.title, "Red Shirt");
        assert_eq!(found.sku.unwrap(), "SKU-RED-001");
        assert_eq!(found.price, 1000);
        assert_eq!(found.stock, 50);
        assert!(found.is_active);
    }

    #[tokio::test]
    async fn find_by_sku() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        seed_variant(&pool, pid, "Green", "SKU-GRN").await;
        let found = super::find_by_sku(&pool, "SKU-GRN", None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.title, "Green");
    }

    #[tokio::test]
    async fn find_by_product_id() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        seed_variant(&pool, pid, "V1", "SKU1").await;
        seed_variant(&pool, pid, "V2", "SKU2").await;

        let variants = super::find_by_product_id(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        assert_eq!(variants.len(), 2);
    }

    #[tokio::test]
    async fn find_active_by_product_id_filters_inactive() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;

        insert(
            &pool,
            &crate::commands::CreateProductVariantCmd {
                product_id: SnowflakeId(pid),
                sku: Some("SKU-ACTIVE".to_string()),
                title: "Active".to_string(),
                price: 100,
                original_price: None,
                stock: 10,
                attributes: None,
                image_url: None,
                weight: None,
                sort_order: 0,
                is_active: true,
            },
            None,
        )
        .await
        .unwrap();
        insert(
            &pool,
            &crate::commands::CreateProductVariantCmd {
                product_id: SnowflakeId(pid),
                sku: Some("SKU-INACTIVE".to_string()),
                title: "Inactive".to_string(),
                price: 100,
                original_price: None,
                stock: 10,
                attributes: None,
                image_url: None,
                weight: None,
                sort_order: 1,
                is_active: false,
            },
            None,
        )
        .await
        .unwrap();

        let active = super::find_active_by_product_id(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].title, "Active");
    }

    #[tokio::test]
    async fn update_changes_title_and_price() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        let v = seed_variant(&pool, pid, "Old", "SKU-OLD").await;

        let ok = super::update(
            &pool,
            &crate::commands::UpdateProductVariantCmd {
                id: v.id,
                sku: Some("SKU-NEW".to_string()),
                title: "New".to_string(),
                price: 2000,
                original_price: Some(2500),
                stock: 99,
                attributes: None,
                image_url: None,
                weight: None,
                sort_order: 1,
                is_active: true,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);

        let found = super::find_by_id(&pool, v.id, None).await.unwrap().unwrap();
        assert_eq!(found.title, "New");
        assert_eq!(found.sku.unwrap(), "SKU-NEW");
        assert_eq!(found.price, 2000);
        assert_eq!(found.original_price.unwrap(), 2500);
        assert_eq!(found.stock, 99);
    }

    #[tokio::test]
    async fn update_not_found() {
        let pool = setup_pool().await;
        let ok = super::update(
            &pool,
            &crate::commands::UpdateProductVariantCmd {
                id: SnowflakeId(99999),
                sku: None,
                title: "X".to_string(),
                price: 100,
                original_price: None,
                stock: 0,
                attributes: None,
                image_url: None,
                weight: None,
                sort_order: 0,
                is_active: true,
            },
            None,
        )
        .await
        .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn delete_by_id() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        let v = seed_variant(&pool, pid, "Bye", "SKU-BYE").await;
        let ok = super::delete_by_id(&pool, v.id, None).await.unwrap();
        assert!(ok);
        assert!(
            super::find_by_id(&pool, v.id, None)
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
    async fn delete_by_product_id() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        seed_variant(&pool, pid, "V1", "SKU-D1").await;
        seed_variant(&pool, pid, "V2", "SKU-D2").await;

        super::delete_by_product_id(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        let variants = super::find_by_product_id(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        assert!(variants.is_empty());
    }

    #[tokio::test]
    async fn count_by_product() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        assert_eq!(
            super::count_by_product(&pool, SnowflakeId(pid), None)
                .await
                .unwrap(),
            0
        );

        seed_variant(&pool, pid, "V1", "SKU-C1").await;
        seed_variant(&pool, pid, "V2", "SKU-C2").await;
        assert_eq!(
            super::count_by_product(&pool, SnowflakeId(pid), None)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;

        insert(
            &pool,
            &crate::commands::CreateProductVariantCmd {
                product_id: SnowflakeId(pid),
                sku: Some("SKU-TENANT".to_string()),
                title: "TenantVariant".to_string(),
                price: 500,
                original_price: None,
                stock: 10,
                attributes: None,
                image_url: None,
                weight: None,
                sort_order: 0,
                is_active: true,
            },
            Some("tenant_a"),
        )
        .await
        .unwrap();

        let a_variants = super::find_by_product_id(&pool, SnowflakeId(pid), Some("tenant_a"))
            .await
            .unwrap();
        assert_eq!(a_variants.len(), 1);

        let b_variants = super::find_by_product_id(&pool, SnowflakeId(pid), Some("tenant_b"))
            .await
            .unwrap();
        assert!(b_variants.is_empty());

        let all_variants = super::find_by_product_id(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        assert_eq!(all_variants.len(), 1);
    }
}
