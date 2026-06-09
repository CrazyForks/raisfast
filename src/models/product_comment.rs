use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    ProductCommentStatus {
        Pending = "pending",
        Approved = "approved",
        Rejected = "rejected",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct ProductComment {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub product_id: SnowflakeId,
    pub order_id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub rating: i64,
    pub title: Option<String>,
    pub content: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub images: Option<String>,
    pub status: ProductCommentStatus,
    pub admin_reply: Option<String>,
    pub admin_replied_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
pub struct ProductCommentStats {
    pub average_rating: f64,
    pub total_count: i64,
    pub rating_distribution: Vec<RatingBucket>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Clone)]
pub struct RatingBucket {
    pub rating: i64,
    pub count: i64,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ProductComment>> {
    let result: Option<ProductComment> = raisfast_derive::crud_find!(
        pool,
        "product_comments",
        ProductComment,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_by_product_order_user(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    order_id: SnowflakeId,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<ProductComment>> {
    let result: Option<ProductComment> = raisfast_derive::crud_find!(
        pool,
        "product_comments",
        ProductComment,
        where: AND(("product_id", product_id), ("order_id", order_id), ("user_id", user_id)),
        tenant: tenant_id
    )?;
    Ok(result)
}

pub async fn find_by_product_paginated(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<ProductComment>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool,
        ProductComment,
        table: "product_comments",
        where: AND(("product_id", product_id), ("status", ProductCommentStatus::Approved)),
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_by_user_paginated(
    pool: &crate::db::Pool,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<ProductComment>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool,
        ProductComment,
        table: "product_comments",
        where: ("user_id", user_id),
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<ProductComment>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool,
        ProductComment,
        table: "product_comments",
        where: ["status" => status],
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreateProductCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<ProductComment> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "product_comments",
        [
            "id" => id,
            "product_id" => cmd.product_id,
            "order_id" => cmd.order_id,
            "user_id" => cmd.user_id,
            "rating" => cmd.rating,
            "title" => &cmd.title,
            "content" => &cmd.content,
            "images" => &cmd.images,
            "status" => ProductCommentStatus::Approved,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id).await?.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("product_comment not found after insert"))
    })
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateProductCommentCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let existing = find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("product_comment"))?;

    let rating = cmd.rating.unwrap_or(existing.rating);
    let title = cmd.title.as_deref().or(existing.title.as_deref());
    let content = cmd.content.as_deref().unwrap_or(&existing.content);
    let images = cmd.images.as_deref().or(existing.images.as_deref());

    let result: crate::db::pool::DbQueryResult = raisfast_derive::crud_update!(
        pool,
        "product_comments",
        bind: [
            "rating" => rating,
            "title" => title,
            "content" => content,
            "images" => images,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", cmd.id),
        tenant: tenant_id
    )?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: ProductCommentStatus,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_update!(
        pool,
        "product_comments",
        bind: ["status" => status],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "product_comment")
}

pub async fn admin_reply(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    reply: &str,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "product_comments",
        bind: ["admin_reply" => reply, "admin_replied_at" => &now],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "product_comment")
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "product_comments",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "product_comment")
}

pub async fn get_stats(
    pool: &crate::db::Pool,
    product_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<ProductCommentStats> {
    let avg_sql = format!(
        "SELECT COALESCE(AVG(rating), 0.0), COUNT(*) FROM product_comments WHERE product_id = {} AND status = 'approved'{}",
        crate::db::Driver::ph(1),
        crate::db::tenant::tenant_filter_ph(tenant_id, 2)
    );
    let (avg, total): (f64, i64) = raisfast_derive::crud_query!(
        pool,
        (f64, i64),
        &avg_sql,
        [product_id],
        fetch_one,
        tenant: tenant_id
    )?;

    let dist_sql = format!(
        "SELECT rating, COUNT(*) as cnt FROM product_comments WHERE product_id = {} AND status = 'approved'{} GROUP BY rating ORDER BY rating DESC",
        crate::db::Driver::ph(1),
        crate::db::tenant::tenant_filter_ph(tenant_id, 2)
    );
    let rows: Vec<(i64, i64)> = raisfast_derive::crud_query!(
        pool,
        (i64, i64),
        &dist_sql,
        [product_id],
        fetch_all,
        tenant: tenant_id
    )?;

    let mut distribution = Vec::new();
    let row_map: std::collections::HashMap<i64, i64> = rows.into_iter().collect();
    for r in 1..=5 {
        distribution.push(RatingBucket {
            rating: r,
            count: *row_map.get(&r).unwrap_or(&0),
        });
    }

    Ok(ProductCommentStats {
        average_rating: (avg * 10.0).round() / 10.0,
        total_count: total,
        rating_distribution: distribution,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CreateProductCommentCmd;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_user(pool: &crate::db::Pool) -> i64 {
        let id = crate::utils::id::new_id();
        let username = format!("testuser_{id}");
        sqlx::query("INSERT INTO users (id, username, role, status, registered_via) VALUES (?, ?, 'reader', 'active', 'email')")
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
                stock: 100,
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

    async fn seed_order(pool: &crate::db::Pool, user_id: i64) -> i64 {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        crate::models::order::insert(
            pool,
            &crate::commands::CreateOrderCmd {
                user_id: SnowflakeId(user_id),
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
                tax_amount: 0,
                coupon_id: None,
                shipping_address_id: None,
                billing_address_id: None,
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

    async fn seed_comment(
        pool: &crate::db::Pool,
        product_id: i64,
        order_id: i64,
        user_id: i64,
        rating: i64,
    ) -> ProductComment {
        insert(
            pool,
            &CreateProductCommentCmd {
                product_id: SnowflakeId(product_id),
                order_id: SnowflakeId(order_id),
                user_id: SnowflakeId(user_id),
                rating,
                title: Some("Great product".into()),
                content: "Really enjoyed it".into(),
                images: None,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        let c = seed_comment(&pool, pid, oid, uid, 5).await;

        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.id, c.id);
        assert_eq!(found.product_id, SnowflakeId(pid));
        assert_eq!(found.order_id, SnowflakeId(oid));
        assert_eq!(found.user_id, SnowflakeId(uid));
        assert_eq!(found.rating, 5);
        assert_eq!(found.content, "Really enjoyed it");
        assert_eq!(found.status, ProductCommentStatus::Approved);
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
    async fn find_by_product_order_user_found() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        seed_comment(&pool, pid, oid, uid, 4).await;

        let found = super::find_by_product_order_user(
            &pool,
            SnowflakeId(pid),
            SnowflakeId(oid),
            SnowflakeId(uid),
            None,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found.rating, 4);
    }

    #[tokio::test]
    async fn find_by_product_order_user_not_found() {
        let pool = setup_pool().await;
        let found = super::find_by_product_order_user(
            &pool,
            SnowflakeId(1),
            SnowflakeId(1),
            SnowflakeId(1),
            None,
        )
        .await
        .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_by_product_paginated() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        for i in 0..5 {
            let uid = seed_user(&pool).await;
            let oid = seed_order(&pool, uid).await;
            seed_comment(&pool, pid, oid, uid, 3 + (i % 3)).await;
        }

        let (items, total) = super::find_by_product_paginated(&pool, SnowflakeId(pid), None, 1, 3)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn find_by_user_paginated() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        for _ in 0..3 {
            let pid = seed_product(&pool).await;
            let oid = seed_order(&pool, uid).await;
            seed_comment(&pool, pid, oid, uid, 5).await;
        }

        let (items, total) = super::find_by_user_paginated(&pool, SnowflakeId(uid), None, 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|c| c.user_id == SnowflakeId(uid)));
    }

    #[tokio::test]
    async fn update_changes_rating_and_content() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        let c = seed_comment(&pool, pid, oid, uid, 3).await;

        let ok = super::update(
            &pool,
            &crate::commands::UpdateProductCommentCmd {
                id: c.id,
                rating: Some(5),
                title: None,
                content: Some("Updated review".into()),
                images: None,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);

        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.rating, 5);
        assert_eq!(found.content, "Updated review");
    }

    #[tokio::test]
    async fn update_status() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        let c = seed_comment(&pool, pid, oid, uid, 4).await;

        super::update_status(&pool, c.id, ProductCommentStatus::Rejected, None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, ProductCommentStatus::Rejected);
    }

    #[tokio::test]
    async fn admin_reply_sets_reply() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        let c = seed_comment(&pool, pid, oid, uid, 4).await;

        super::admin_reply(&pool, c.id, "Thank you!", None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.admin_reply.unwrap(), "Thank you!");
        assert!(found.admin_replied_at.is_some());
    }

    #[tokio::test]
    async fn delete_removes_comment() {
        let pool = setup_pool().await;
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_order(&pool, uid).await;
        let c = seed_comment(&pool, pid, oid, uid, 4).await;

        super::delete_by_id(&pool, c.id, None).await.unwrap();
        assert!(
            super::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn get_stats_aggregates_correctly() {
        let pool = setup_pool().await;
        let pid = seed_product(&pool).await;
        let ratings = [5, 4, 5, 3, 5];
        for &r in &ratings {
            let uid = seed_user(&pool).await;
            let oid = seed_order(&pool, uid).await;
            seed_comment(&pool, pid, oid, uid, r).await;
        }

        let stats = super::get_stats(&pool, SnowflakeId(pid), None)
            .await
            .unwrap();
        assert_eq!(stats.total_count, 5);
        assert!((stats.average_rating - 4.4).abs() < 0.01);
        assert_eq!(stats.rating_distribution.len(), 5);
        assert_eq!(
            stats
                .rating_distribution
                .iter()
                .find(|b| b.rating == 5)
                .unwrap()
                .count,
            3
        );
        assert_eq!(
            stats
                .rating_distribution
                .iter()
                .find(|b| b.rating == 4)
                .unwrap()
                .count,
            1
        );
        assert_eq!(
            stats
                .rating_distribution
                .iter()
                .find(|b| b.rating == 3)
                .unwrap()
                .count,
            1
        );
    }

    #[tokio::test]
    async fn get_stats_empty_product() {
        let pool = setup_pool().await;
        let _pid = seed_product(&pool).await;
        let (id,): (i64,) = sqlx::query_as("SELECT id FROM products ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

        let stats = super::get_stats(&pool, SnowflakeId(id), None)
            .await
            .unwrap();
        assert_eq!(stats.total_count, 0);
        assert_eq!(stats.average_rating, 0.0);
    }
}
