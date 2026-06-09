use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::{DbDriver, Driver, Pool};
use crate::errors::app_error::AppResult;
use crate::models::order::OrderStatus;
use crate::worker::{Job, JobHandler};

pub struct ExpireOrdersHandler {
    pool: Pool,
    config: Arc<AppConfig>,
}

impl ExpireOrdersHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>) -> Self {
        Self { pool, config }
    }
}

#[async_trait::async_trait]
impl JobHandler for ExpireOrdersHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::ExpireOrders = job else {
            return Ok(());
        };

        let minutes = self.config.order_expire_minutes.max(1);
        let cutoff = chrono::Utc::now() - chrono::Duration::minutes(minutes);

        let sql = format!(
            "SELECT * FROM orders WHERE status = 'pending' AND created_at < {} LIMIT 500",
            Driver::ph(1)
        );
        let orders: Vec<crate::models::order::Order> = sqlx::query_as(&sql)
            .bind(cutoff.format("%Y-%m-%d %H:%M:%S").to_string())
            .fetch_all(&self.pool)
            .await?;

        let mut expired = 0u64;
        let mut failed = 0u64;

        for order in &orders {
            match self.expire_one(order).await {
                Ok(()) => expired += 1,
                Err(e) => {
                    tracing::warn!(
                        "[expire_orders] failed to expire order {}: {e}",
                        order.id.to_string()
                    );
                    failed += 1;
                }
            }
        }

        if expired > 0 || failed > 0 {
            tracing::info!("[expire_orders] expired {expired} order(s), failed {failed}");
        }
        Ok(())
    }
}

impl ExpireOrdersHandler {
    async fn expire_one(&self, order: &crate::models::order::Order) -> AppResult<()> {
        crate::in_transaction!(&self.pool, tx, {
            let rows = crate::models::order::tx_update_status_cas(
                &mut tx,
                order.id,
                OrderStatus::Expired,
                Some("expired_at"),
                OrderStatus::Pending,
            )
            .await?;
            if rows == 0 {
                return Ok(());
            }

            let items = crate::models::order_item::find_by_order_id(
                &self.pool,
                order.id,
                order.tenant_id.as_deref(),
            )
            .await?;
            for item in &items {
                if let Some(vid) = item.variant_id {
                    crate::models::product_variant::tx_replenish_stock(
                        &mut tx,
                        vid,
                        item.quantity,
                        order.tenant_id.as_deref(),
                    )
                    .await?;
                } else if let Some(pid) = item.product_id {
                    crate::models::product::tx_replenish_stock(
                        &mut tx,
                        pid,
                        item.quantity,
                        order.tenant_id.as_deref(),
                    )
                    .await?;
                }
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> Pool {
        let pool = Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn seed_user(pool: &Pool) -> i64 {
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

    async fn seed_product(pool: &Pool, stock: i64) -> crate::types::snowflake_id::SnowflakeId {
        let p = crate::models::product::insert(
            pool,
            &crate::commands::CreateProductCmd {
                category_id: None,
                title: "Widget".to_string(),
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
                stock,
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
        p.id
    }

    async fn seed_pending_order_with_items(
        pool: &Pool,
        user_id: i64,
        product_id: crate::types::snowflake_id::SnowflakeId,
        qty: i64,
        created_at_offset_minutes: i64,
    ) -> crate::types::snowflake_id::SnowflakeId {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        let order = crate::models::order::insert(
            pool,
            &crate::commands::CreateOrderCmd {
                user_id: crate::types::snowflake_id::SnowflakeId(user_id),
                order_no,
                subtotal: 1000 * qty,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: 1000 * qty,
                currency: "CNY".into(),
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
            },
            None,
        )
        .await
        .unwrap();

        crate::models::order_item::insert_batch(
            pool,
            vec![crate::commands::CreateOrderItemCmd {
                order_id: order.id,
                product_id: Some(*product_id),
                variant_id: None,
                title: "Widget".to_string(),
                description: None,
                sku: None,
                unit_price: 1000,
                quantity: qty,
                subtotal: 1000 * qty,
                tax_amount: 0,
                cover_url: None,
                attributes: None,
            }],
            None,
        )
        .await
        .unwrap();

        if created_at_offset_minutes != 0 {
            let offset = chrono::Duration::minutes(created_at_offset_minutes);
            let past = (chrono::Utc::now() + offset)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string();
            sqlx::query("UPDATE orders SET created_at = ? WHERE id = ?")
                .bind(&past)
                .bind(order.id)
                .execute(pool)
                .await
                .unwrap();
        }

        order.id
    }

    async fn get_product_stock(pool: &Pool, id: crate::types::snowflake_id::SnowflakeId) -> i64 {
        let (s,): (i64,) = sqlx::query_as("SELECT stock FROM products WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        s
    }

    async fn get_order_status(pool: &Pool, id: crate::types::snowflake_id::SnowflakeId) -> String {
        let (s,): (String,) = sqlx::query_as("SELECT status FROM orders WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        s
    }

    #[tokio::test]
    async fn ignores_wrong_job_type() {
        let pool = setup_pool().await;
        let config = Arc::new(AppConfig::test_defaults());
        let handler = ExpireOrdersHandler::new(pool, config);
        let job = Job::GenerateSitemap;
        assert!(handler.handle(&job).await.is_ok());
    }

    #[tokio::test]
    async fn expires_old_pending_order() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool, 100).await;

        sqlx::query("UPDATE products SET stock = ? WHERE id = ?")
            .bind(97i64)
            .bind(pid)
            .execute(&pool)
            .await
            .unwrap();

        let oid = seed_pending_order_with_items(&pool, uid, pid, 3, -60).await;
        assert_eq!(get_product_stock(&pool, pid).await, 97);

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);
        handler.handle(&Job::ExpireOrders).await.unwrap();

        let status = get_order_status(&pool, oid).await;
        assert_eq!(status, "expired");
        assert_eq!(get_product_stock(&pool, pid).await, 100);
    }

    #[tokio::test]
    async fn skips_recent_pending_order() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool, 100).await;

        let oid = seed_pending_order_with_items(&pool, uid, pid, 2, -5).await;

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);
        handler.handle(&Job::ExpireOrders).await.unwrap();

        let status = get_order_status(&pool, oid).await;
        assert_eq!(status, "pending");
        assert_eq!(get_product_stock(&pool, pid).await, 100);
    }

    #[tokio::test]
    async fn skips_non_pending_order() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool, 100).await;

        let oid = seed_pending_order_with_items(&pool, uid, pid, 2, -60).await;

        crate::models::order::update_status(&pool, oid, "paid", Some("paid_at"), None)
            .await
            .unwrap();

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);
        handler.handle(&Job::ExpireOrders).await.unwrap();

        let status = get_order_status(&pool, oid).await;
        assert_eq!(status, "paid");
        assert_eq!(get_product_stock(&pool, pid).await, 100);
    }

    #[tokio::test]
    async fn expires_multiple_old_orders() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool, 100).await;

        sqlx::query("UPDATE products SET stock = ? WHERE id = ?")
            .bind(95i64)
            .bind(pid)
            .execute(&pool)
            .await
            .unwrap();

        let oid1 = seed_pending_order_with_items(&pool, uid, pid, 2, -60).await;
        let oid2 = seed_pending_order_with_items(&pool, uid, pid, 3, -120).await;

        assert_eq!(get_product_stock(&pool, pid).await, 95);

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);
        handler.handle(&Job::ExpireOrders).await.unwrap();

        assert_eq!(get_order_status(&pool, oid1).await, "expired");
        assert_eq!(get_order_status(&pool, oid2).await, "expired");
        assert_eq!(get_product_stock(&pool, pid).await, 100);
    }

    #[tokio::test]
    async fn handles_order_without_items() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;

        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        let order = crate::models::order::insert(
            &pool,
            &crate::commands::CreateOrderCmd {
                user_id: crate::types::snowflake_id::SnowflakeId(uid),
                order_no,
                subtotal: 0,
                discount_amount: 0,
                shipping_amount: 0,
                total_amount: 0,
                currency: "CNY".into(),
                buyer_name: None,
                buyer_phone: None,
                buyer_email: None,
                shipping_address: None,
                remark: None,
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
            },
            None,
        )
        .await
        .unwrap();

        let past = (chrono::Utc::now() - chrono::Duration::minutes(60))
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        sqlx::query("UPDATE orders SET created_at = ? WHERE id = ?")
            .bind(&past)
            .bind(order.id)
            .execute(&pool)
            .await
            .unwrap();

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);
        handler.handle(&Job::ExpireOrders).await.unwrap();

        let status = get_order_status(&pool, order.id).await;
        assert_eq!(status, "expired");
    }

    #[tokio::test]
    async fn partial_expire_does_not_double_replenish() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool, 100).await;

        sqlx::query("UPDATE products SET stock = ? WHERE id = ?")
            .bind(95i64)
            .bind(pid)
            .execute(&pool)
            .await
            .unwrap();

        let oid = seed_pending_order_with_items(&pool, uid, pid, 5, -60).await;
        assert_eq!(get_product_stock(&pool, pid).await, 95);

        let config = Arc::new(AppConfig {
            order_expire_minutes: 30,
            ..AppConfig::test_defaults()
        });
        let handler = ExpireOrdersHandler::new(pool.clone(), config);

        handler.handle(&Job::ExpireOrders).await.unwrap();
        assert_eq!(get_order_status(&pool, oid).await, "expired");
        assert_eq!(get_product_stock(&pool, pid).await, 100);

        handler.handle(&Job::ExpireOrders).await.unwrap();
        assert_eq!(get_product_stock(&pool, pid).await, 100);
    }
}
