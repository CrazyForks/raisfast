use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct CartItem {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub user_id: i64,
    pub product_id: i64,
    pub variant_id: Option<i64>,
    pub quantity: i64,
    pub attributes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<CartItem>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "cart_items",
        CartItem,
        "user_id" => user_id,
        order_by: "created_at DESC",
        tenant: tenant_id
    )?)
}

pub async fn find_by_user_and_product(
    pool: &crate::db::Pool,
    user_id: i64,
    product_id: i64,
    variant_id: Option<i64>,
    tenant_id: Option<&str>,
) -> AppResult<Option<CartItem>> {
    if let Some(vid) = variant_id {
        raisfast_derive::crud_find!(
            pool,
            "cart_items",
            CartItem,
            "user_id" => user_id,
            and: ["product_id" => product_id, "variant_id" => vid],
            tenant: tenant_id
        )
        .map_err(Into::into)
    } else {
        raisfast_derive::crud_find!(
            pool,
            "cart_items",
            CartItem,
            "user_id" => user_id,
            and: ["product_id" => product_id],
            and_null: ["variant_id"],
            tenant: tenant_id
        )
        .map_err(Into::into)
    }
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<CartItem> {
    raisfast_derive::crud_find_one!(pool, "cart_items", CartItem, "document_id" => document_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_document_id_opt(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<CartItem>> {
    raisfast_derive::crud_find!(pool, "cart_items", CartItem, "document_id" => document_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn insert(
    pool: &crate::db::Pool,
    user_id: i64,
    product_id: i64,
    variant_id: Option<i64>,
    quantity: i64,
    attributes: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<CartItem> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "cart_items",
        [
            "document_id" => &document_id,
            "user_id" => user_id,
            "product_id" => product_id,
            "variant_id" => variant_id,
            "quantity" => quantity,
            "attributes" => attributes,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_document_id(pool, &document_id, tenant_id).await
}

pub async fn update_quantity(
    pool: &crate::db::Pool,
    id: i64,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "cart_items",
        bind: ["quantity" => quantity, "updated_at" => &now],
        where: "id" => id,
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn delete_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "cart_items", "document_id" => document_id, tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn delete_by_user_id(
    pool: &crate::db::Pool,
    user_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result =
        raisfast_derive::crud_delete!(pool, "cart_items", "user_id" => user_id, tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn tx_delete_by_user_id(
    tx: &mut crate::db::pool::DbConnection,
    user_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(&mut *tx, "cart_items", "user_id" => user_id, tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn count_by_user(
    pool: &crate::db::Pool,
    user_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let count =
        raisfast_derive::crud_count!(pool, "cart_items", "user_id" => user_id, tenant: tenant_id)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let username = format!("testuser_{doc_id}");
        sqlx::query(
            "INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')",
        )
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

    #[tokio::test]
    async fn insert_and_find_by_document_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(&pool, uid, pid, None, 2, Some(r#"{"color":"red"}"#), None)
            .await
            .unwrap();

        assert_eq!(item.user_id, uid);
        assert_eq!(item.product_id, pid);
        assert_eq!(item.quantity, 2);
        assert_eq!(item.attributes.as_deref(), Some(r#"{"color":"red"}"#));
        assert!(!item.document_id.is_empty());
    }

    #[tokio::test]
    async fn find_by_user_id_returns_items() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(&pool, uid, p1, None, 1, None, None)
            .await
            .unwrap();
        super::insert(&pool, uid, p2, None, 3, None, None)
            .await
            .unwrap();

        let items = super::find_by_user_id(&pool, uid, None).await.unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|it| it.user_id == uid));
    }

    #[tokio::test]
    async fn find_by_user_id_empty() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let items = super::find_by_user_id(&pool, uid, None).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn find_by_user_and_product_found() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(&pool, uid, pid, None, 5, None, None)
            .await
            .unwrap();

        let found = super::find_by_user_and_product(&pool, uid, pid, None, None)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().quantity, 5);
    }

    #[tokio::test]
    async fn find_by_user_and_product_not_found() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let found = super::find_by_user_and_product(&pool, uid, 99999, None, None)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_document_id_opt() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(&pool, uid, pid, None, 1, None, None)
            .await
            .unwrap();

        let found = super::find_by_document_id_opt(&pool, &item.document_id, None)
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, item.id);

        let not_found = super::find_by_document_id_opt(&pool, "nonexistent", None)
            .await
            .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn update_quantity() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(&pool, uid, pid, None, 1, None, None)
            .await
            .unwrap();

        super::update_quantity(&pool, item.id, 10, None)
            .await
            .unwrap();

        let updated = super::find_by_document_id(&pool, &item.document_id, None)
            .await
            .unwrap();
        assert_eq!(updated.quantity, 10);
    }

    #[tokio::test]
    async fn update_quantity_nonexistent() {
        let pool = setup_pool().await;
        let result = super::update_quantity(&pool, 99999, 10, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_by_document_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(&pool, uid, pid, None, 1, None, None)
            .await
            .unwrap();

        super::delete_by_document_id(&pool, &item.document_id, None)
            .await
            .unwrap();

        let found = super::find_by_document_id_opt(&pool, &item.document_id, None)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_by_document_id_nonexistent() {
        let pool = setup_pool().await;
        let result = super::delete_by_document_id(&pool, "nonexistent", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_by_user_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(&pool, uid, p1, None, 1, None, None)
            .await
            .unwrap();
        super::insert(&pool, uid, p2, None, 2, None, None)
            .await
            .unwrap();

        super::delete_by_user_id(&pool, uid, None).await.unwrap();

        let items = super::find_by_user_id(&pool, uid, None).await.unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn delete_by_user_id_does_not_affect_other_user() {
        let pool = setup_pool().await;
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(&pool, uid1, pid, None, 1, None, None)
            .await
            .unwrap();
        super::insert(&pool, uid2, pid, None, 2, None, None)
            .await
            .unwrap();

        super::delete_by_user_id(&pool, uid1, None).await.unwrap();

        let items2 = super::find_by_user_id(&pool, uid2, None).await.unwrap();
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].quantity, 2);
    }

    #[tokio::test]
    async fn count_by_user() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;

        assert_eq!(super::count_by_user(&pool, uid, None).await.unwrap(), 0);

        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;
        super::insert(&pool, uid, p1, None, 1, None, None)
            .await
            .unwrap();
        super::insert(&pool, uid, p2, None, 1, None, None)
            .await
            .unwrap();

        assert_eq!(super::count_by_user(&pool, uid, None).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn tx_delete_by_user_id() -> Result<(), Box<dyn std::error::Error>> {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(&pool, uid, pid, None, 1, None, None).await?;

        crate::in_transaction!(&pool, tx, {
            super::tx_delete_by_user_id(&mut tx, uid, None).await?;
            Ok(())
        })?;

        let items = super::find_by_user_id(&pool, uid, None).await?;
        assert!(items.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(&pool, uid, p1, None, 1, None, Some("tenant_a"))
            .await
            .unwrap();
        super::insert(&pool, uid, p2, None, 2, None, None)
            .await
            .unwrap();

        let items_a = super::find_by_user_id(&pool, uid, Some("tenant_a"))
            .await
            .unwrap();
        assert_eq!(items_a.len(), 1);
        assert_eq!(items_a[0].product_id, p1);

        let items_b = super::find_by_user_id(&pool, uid, Some("tenant_b"))
            .await
            .unwrap();
        assert!(items_b.is_empty());

        let items_none = super::find_by_user_id(&pool, uid, None).await.unwrap();
        assert_eq!(items_none.len(), 2);
    }
}
