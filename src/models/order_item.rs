use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
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

crate::impl_from_row_opt_tenant!(OrderItem {
    required { id, document_id, order_id, title, unit_price, quantity, subtotal, created_at }
    optional { product_id, description, cover_url, attributes }
});

pub async fn find_by_order_id(
    pool: &crate::db::Pool,
    order_id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Vec<OrderItem>> {
    let sql = format!(
        "SELECT * FROM order_items WHERE order_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, OrderItem>(&sql).bind(order_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert(
    pool: &crate::db::Pool,
    document_id: &str,
    order_id: i64,
    product_id: Option<i64>,
    title: &str,
    description: Option<&str>,
    unit_price: i64,
    quantity: i64,
    subtotal: i64,
    cover_url: Option<&str>,
    attributes: Option<&str>,
    tenant_id: Option<&str>,
) -> AppResult<OrderItem> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO order_items (document_id, tenant_id, order_id, product_id, title, description, unit_price, quantity, subtotal, cover_url, attributes, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
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
                ph(11)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(tid)
                .bind(order_id)
                .bind(product_id)
                .bind(title)
                .bind(description)
                .bind(unit_price)
                .bind(quantity)
                .bind(subtotal)
                .bind(cover_url)
                .bind(attributes)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO order_items (document_id, order_id, product_id, title, description, unit_price, quantity, subtotal, cover_url, attributes, created_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9),
                ph(10)
            );
            sqlx::query(&sql)
                .bind(document_id)
                .bind(order_id)
                .bind(product_id)
                .bind(title)
                .bind(description)
                .bind(unit_price)
                .bind(quantity)
                .bind(subtotal)
                .bind(cover_url)
                .bind(attributes)
                .execute(pool)
                .await?;
        }
    }
    let sql2 = format!(
        "SELECT * FROM order_items WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, OrderItem>(&sql2).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_one(pool).await.map_err(Into::into)
}

pub async fn insert_batch(
    pool: &crate::db::Pool,
    items: Vec<InsertOrderItem>,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    for item in &items {
        insert(
            pool,
            &item.document_id,
            item.order_id,
            item.product_id,
            &item.title,
            item.description.as_deref(),
            item.unit_price,
            item.quantity,
            item.subtotal,
            item.cover_url.as_deref(),
            item.attributes.as_deref(),
            tenant_id,
        )
        .await?;
    }
    Ok(())
}

pub struct InsertOrderItem {
    pub document_id: String,
    pub order_id: i64,
    pub product_id: Option<i64>,
    pub title: String,
    pub description: Option<String>,
    pub unit_price: i64,
    pub quantity: i64,
    pub subtotal: i64,
    pub cover_url: Option<String>,
    pub attributes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
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
        let doc_id = uuid::Uuid::now_v7().to_string();
        let order_no = format!(
            "ORD-{}",
            &uuid::Uuid::now_v7().to_string().replace('-', "")[..16]
        );
        crate::models::order::insert(
            pool, &doc_id, user_id, &order_no, 1000, 0, 0, 1000, "CNY", None, None, None, None,
            None, None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM orders WHERE document_id = ?")
            .bind(&doc_id)
            .fetch_one(pool)
            .await
            .unwrap();
        id
    }

    async fn seed_product(pool: &crate::db::Pool) -> i64 {
        let doc_id = uuid::Uuid::now_v7().to_string();
        crate::models::product::insert(
            pool,
            &doc_id,
            None,
            "Test Product",
            None,
            None,
            "custom",
            "digital",
            None,
            None,
            1000,
            "CNY",
            None,
            0,
            None,
            None,
            None,
            None,
            None,
            "piece",
            1,
            None,
            0,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM products WHERE document_id = ?")
            .bind(&doc_id)
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
        let doc_id = uuid::Uuid::now_v7().to_string();

        let item = super::insert(
            &pool,
            &doc_id,
            order_id,
            Some(pid),
            "Widget",
            Some("A nice widget"),
            1000,
            2,
            2000,
            Some("https://img.test/widget.jpg"),
            Some(r#"{"color":"red"}"#),
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
            let doc_id = uuid::Uuid::now_v7().to_string();
            super::insert(
                &pool,
                &doc_id,
                order_id,
                None,
                &format!("Item{i}"),
                None,
                100 * (i + 1),
                i + 1,
                100 * (i + 1) * (i + 1),
                None,
                None,
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

        let doc1 = uuid::Uuid::now_v7().to_string();
        super::insert(
            &pool, &doc1, order1, None, "Item1", None, 100, 1, 100, None, None, None,
        )
        .await
        .unwrap();
        let doc2 = uuid::Uuid::now_v7().to_string();
        super::insert(
            &pool, &doc2, order2, None, "Item2", None, 200, 1, 200, None, None, None,
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
            InsertOrderItem {
                document_id: uuid::Uuid::now_v7().to_string(),
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
            InsertOrderItem {
                document_id: uuid::Uuid::now_v7().to_string(),
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
