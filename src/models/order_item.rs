use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct OrderItem {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub order_id: i64,
    pub product_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub unit_price: i64,
    pub quantity: i64,
    pub subtotal: i64,
    pub cover_url: Option<String>,
    pub attributes: Option<String>,
    pub created_at: Timestamp,
}

pub async fn find_by_order_id(
    pool: &crate::db::Pool,
    order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<OrderItem>> {
    raisfast_derive::crud_find_all!(pool, "order_items", OrderItem, "order_id" => order_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateOrderItemCmd,
    tenant_id: Option<&str>,
) -> AppResult<OrderItem> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "order_items",
        [
            "document_id" => &document_id,
            "order_id" => cmd.order_id,
            "product_id" => cmd.product_id,
            "title" => &cmd.title,
            "description" => &cmd.description,
            "unit_price" => cmd.unit_price,
            "quantity" => cmd.quantity,
            "subtotal" => cmd.subtotal,
            "cover_url" => &cmd.cover_url,
            "attributes" => &cmd.attributes,
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    raisfast_derive::crud_find_one!(pool, "order_items", OrderItem, "document_id" => &document_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn insert_batch(
    pool: &crate::db::Pool,
    items: Vec<crate::commands::CreateOrderItemCmd>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    for item in &items {
        insert(pool, item, tenant_id).await?;
    }
    Ok(())
}

pub async fn tx_insert(
    tx: &mut crate::db::pool::DbConnection,
    cmd: &crate::commands::CreateOrderItemCmd,
    tenant_id: Option<&str>,
) -> AppResult<OrderItem> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        &mut *tx,
        "order_items",
        [
            "document_id" => &document_id,
            "order_id" => cmd.order_id,
            "product_id" => cmd.product_id,
            "title" => &cmd.title,
            "description" => &cmd.description,
            "unit_price" => cmd.unit_price,
            "quantity" => cmd.quantity,
            "subtotal" => cmd.subtotal,
            "cover_url" => &cmd.cover_url,
            "attributes" => &cmd.attributes,
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    raisfast_derive::crud_find_one!(&mut *tx, "order_items", OrderItem, "document_id" => &document_id, tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn tx_insert_batch(
    tx: &mut crate::db::pool::DbConnection,
    items: Vec<crate::commands::CreateOrderItemCmd>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    for item in &items {
        tx_insert(tx, item, tenant_id).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        let username = format!("testuser_{doc_id}");
        sqlx::query("INSERT INTO users (document_id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')")
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

    async fn seed_order(pool: &crate::db::Pool, user_id: i64) -> i64 {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        crate::models::order::insert(
            pool,
            &crate::commands::CreateOrderCmd {
                user_id,
                order_no,
                subtotal: 1000,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: 1000,
                currency: "CNY".into(),
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
            },
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM orders ORDER BY id DESC LIMIT 1")
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
        let order_id = seed_order(&pool, uid).await;
        let pid = seed_product(&pool).await;

        let item = super::insert(
            &pool,
            &crate::commands::CreateOrderItemCmd {
                order_id,
                product_id: Some(pid),
                title: "Widget".into(),
                description: Some("A nice widget".into()),
                unit_price: 1000,
                quantity: 2,
                subtotal: 2000,
                cover_url: Some("https://img.test/widget.jpg".into()),
                attributes: Some(r#"{"color":"red"}"#.into()),
            },
            None,
        )
        .await
        .unwrap();

        assert_eq!(item.order_id, order_id);
        assert_eq!(item.product_id, Some(pid));
        assert_eq!(item.title, "Widget");
        assert_eq!(item.unit_price, 1000);
        assert_eq!(item.quantity, 2);
        assert_eq!(item.subtotal, 2000);
        assert_eq!(item.description.unwrap(), "A nice widget");
        assert_eq!(item.cover_url.unwrap(), "https://img.test/widget.jpg");
        assert_eq!(item.attributes.unwrap(), r#"{"color":"red"}"#);
    }

    #[tokio::test]
    async fn find_by_order_id_returns_items() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let order_id = seed_order(&pool, uid).await;

        for i in 0..3 {
            super::insert(
                &pool,
                &crate::commands::CreateOrderItemCmd {
                    order_id,
                    product_id: None,
                    title: format!("Item{i}"),
                    description: None,
                    unit_price: 100 * (i + 1),
                    quantity: i + 1,
                    subtotal: 100 * (i + 1) * (i + 1),
                    cover_url: None,
                    attributes: None,
                },
                None,
            )
            .await
            .unwrap();
        }

        let items = super::find_by_order_id(&pool, order_id, None)
            .await
            .unwrap();
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|it| it.order_id == order_id));
    }

    #[tokio::test]
    async fn find_by_order_id_empty() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let order_id = seed_order(&pool, uid).await;
        let items = super::find_by_order_id(&pool, order_id, None)
            .await
            .unwrap();
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn find_by_order_id_different_orders() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let order1 = seed_order(&pool, uid).await;
        let order2 = seed_order(&pool, uid).await;

        super::insert(
            &pool,
            &crate::commands::CreateOrderItemCmd {
                order_id: order1,
                product_id: None,
                title: "Item1".into(),
                description: None,
                unit_price: 100,
                quantity: 1,
                subtotal: 100,
                cover_url: None,
                attributes: None,
            },
            None,
        )
        .await
        .unwrap();
        super::insert(
            &pool,
            &crate::commands::CreateOrderItemCmd {
                order_id: order2,
                product_id: None,
                title: "Item2".into(),
                description: None,
                unit_price: 200,
                quantity: 1,
                subtotal: 200,
                cover_url: None,
                attributes: None,
            },
            None,
        )
        .await
        .unwrap();

        let items1 = super::find_by_order_id(&pool, order1, None).await.unwrap();
        let items2 = super::find_by_order_id(&pool, order2, None).await.unwrap();
        assert_eq!(items1.len(), 1);
        assert_eq!(items2.len(), 1);
        assert_eq!(items1[0].title, "Item1");
        assert_eq!(items2[0].title, "Item2");
    }

    #[tokio::test]
    async fn insert_batch() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let order_id = seed_order(&pool, uid).await;

        let items = vec![
            crate::commands::CreateOrderItemCmd {
                order_id,
                product_id: None,
                title: "Batch1".into(),
                description: None,
                unit_price: 100,
                quantity: 2,
                subtotal: 200,
                cover_url: None,
                attributes: None,
            },
            crate::commands::CreateOrderItemCmd {
                order_id,
                product_id: None,
                title: "Batch2".into(),
                description: None,
                unit_price: 300,
                quantity: 1,
                subtotal: 300,
                cover_url: None,
                attributes: None,
            },
        ];

        super::insert_batch(&pool, items, None).await.unwrap();
        let found = super::find_by_order_id(&pool, order_id, None)
            .await
            .unwrap();
        assert_eq!(found.len(), 2);
    }

    #[tokio::test]
    async fn insert_batch_empty() {
        let pool = setup_pool().await;
        super::insert_batch(&pool, vec![], None).await.unwrap();
    }
}
