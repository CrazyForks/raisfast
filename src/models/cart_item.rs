use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct CartItem {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub product_id: SnowflakeId,
    pub variant_id: Option<SnowflakeId>,
    pub quantity: i64,
    pub attributes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Vec<CartItem>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "cart_items",
        CartItem,
        where: ("user_id", user_id),
        order_by: "created_at DESC",
        tenant: tenant_id
    )?)
}

pub async fn find_by_user_and_product(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    product_id: SnowflakeId,
    variant_id: Option<i64>,
    tenant_id: Option<&str>,
) -> AppResult<Option<CartItem>> {
    if let Some(vid) = variant_id {
        raisfast_derive::crud_find!(
            pool,
            "cart_items",
            CartItem,
            where: AND(("user_id", user_id), ("product_id", product_id), ("variant_id", vid)),
            tenant: tenant_id
        )
        .map_err(Into::into)
    } else {
        raisfast_derive::crud_find!(
            pool,
            "cart_items",
            CartItem,
            where: AND(("user_id", user_id), ("product_id", product_id), ("variant_id", IS_NULL)),
            tenant: tenant_id
        )
        .map_err(Into::into)
    }
}

pub async fn insert(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    product_id: SnowflakeId,
    variant_id: Option<i64>,
    quantity: i64,
    attributes: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<CartItem> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "cart_items",
        [
            "id" => id,
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
    raisfast_derive::crud_find_one!(pool, "cart_items", CartItem, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn update_quantity(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    quantity: i64,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "cart_items",
        bind: ["quantity" => quantity, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<CartItem>> {
    Ok(
        raisfast_derive::crud_find!(pool, "cart_items", CartItem, where: ("id", id), tenant: tenant_id)?,
    )
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result =
        raisfast_derive::crud_delete!(pool, "cart_items", where: ("id", id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn delete_by_user_id(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "cart_items", where: ("user_id", user_id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn tx_delete_by_user_id(
    tx: &mut crate::db::pool::DbConnection,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(&mut *tx, "cart_items", where: ("user_id", user_id), tenant: tenant_id)?;
    AppError::expect_affected(&result, "cart_item")
}

pub async fn count_by_user(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let count = raisfast_derive::crud_count!(pool, "cart_items", where: ("user_id", user_id), tenant: tenant_id)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use crate::types::snowflake_id::SnowflakeId;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("testuser_{id}");
        sqlx::query(
            "INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')",
        )
        .bind(id)
        .bind(&username)
        .execute(pool)
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

    #[tokio::test]
    async fn insert_and_find() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            2,
            Some(r#"{"color":"red"}"#),
            None,
        )
        .await
        .unwrap();

        assert_eq!(item.user_id, SnowflakeId(uid));
        assert_eq!(item.product_id, SnowflakeId(pid));
        assert_eq!(item.quantity, 2);
        assert_eq!(item.attributes.as_deref(), Some(r#"{"color":"red"}"#));
        assert!(*item.id > 0);
    }

    #[tokio::test]
    async fn find_by_user_id_returns_items() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p1),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p2),
            None,
            3,
            None,
            None,
        )
        .await
        .unwrap();

        let items = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|it| it.user_id == SnowflakeId(uid)));
    }

    #[tokio::test]
    async fn find_by_user_id_empty() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let items = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn find_by_user_and_product_found() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            5,
            None,
            None,
        )
        .await
        .unwrap();

        let found =
            super::find_by_user_and_product(&pool, SnowflakeId(uid), SnowflakeId(pid), None, None)
                .await
                .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().quantity, 5);
    }

    #[tokio::test]
    async fn find_by_user_and_product_not_found() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let found = super::find_by_user_and_product(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(99999),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_id_opt() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();

        let found =
            super::find_by_user_and_product(&pool, SnowflakeId(uid), SnowflakeId(pid), None, None)
                .await
                .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, item.id);

        let not_found = super::find_by_user_and_product(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(99999),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn update_quantity() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();

        super::update_quantity(&pool, item.id, 10, None)
            .await
            .unwrap();

        let updated =
            super::find_by_user_and_product(&pool, SnowflakeId(uid), SnowflakeId(pid), None, None)
                .await
                .unwrap()
                .unwrap();
        assert_eq!(updated.quantity, 10);
    }

    #[tokio::test]
    async fn update_quantity_nonexistent() {
        let pool = setup_pool().await;
        let result = super::update_quantity(&pool, SnowflakeId(99999), 10, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();

        super::delete_by_id(&pool, item.id, None).await.unwrap();

        let found =
            super::find_by_user_and_product(&pool, SnowflakeId(uid), SnowflakeId(pid), None, None)
                .await
                .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_by_id_nonexistent() {
        let pool = setup_pool().await;
        let result = super::delete_by_id(&pool, SnowflakeId(99999), None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_by_user_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p1),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p2),
            None,
            2,
            None,
            None,
        )
        .await
        .unwrap();

        super::delete_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();

        let items = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn delete_by_user_id_does_not_affect_other_user() {
        let pool = setup_pool().await;
        let uid1 = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid1),
            SnowflakeId(pid),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            SnowflakeId(uid2),
            SnowflakeId(pid),
            None,
            2,
            None,
            None,
        )
        .await
        .unwrap();

        super::delete_by_user_id(&pool, SnowflakeId(uid1), None)
            .await
            .unwrap();

        let items2 = super::find_by_user_id(&pool, SnowflakeId(uid2), None)
            .await
            .unwrap();
        assert_eq!(items2.len(), 1);
        assert_eq!(items2[0].quantity, 2);
    }

    #[tokio::test]
    async fn count_by_user() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;

        assert_eq!(
            super::count_by_user(&pool, SnowflakeId(uid), None)
                .await
                .unwrap(),
            0
        );

        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;
        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p1),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p2),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            super::count_by_user(&pool, SnowflakeId(uid), None)
                .await
                .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn tx_delete_by_user_id() -> Result<(), crate::errors::app_error::AppError> {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(pid),
            None,
            1,
            None,
            None,
        )
        .await
        .unwrap();

        crate::in_transaction!(&pool, tx, {
            super::tx_delete_by_user_id(&mut tx, SnowflakeId(uid), None).await?;
            Ok(())
        })?;

        let items = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert!(items.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let p1 = seed_product(&pool).await;
        let p2 = seed_product(&pool).await;

        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p1),
            None,
            1,
            None,
            Some("tenant_a"),
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            SnowflakeId(uid),
            SnowflakeId(p2),
            None,
            2,
            None,
            None,
        )
        .await
        .unwrap();

        let items_a = super::find_by_user_id(&pool, SnowflakeId(uid), Some("tenant_a"))
            .await
            .unwrap();
        assert_eq!(items_a.len(), 1);
        assert_eq!(items_a[0].product_id, SnowflakeId(p1));

        let items_b = super::find_by_user_id(&pool, SnowflakeId(uid), Some("tenant_b"))
            .await
            .unwrap();
        assert!(items_b.is_empty());

        let items_none = super::find_by_user_id(&pool, SnowflakeId(uid), None)
            .await
            .unwrap();
        assert_eq!(items_none.len(), 2);
    }
}
