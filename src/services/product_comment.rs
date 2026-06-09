use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::product_comment::{
    AdminProductCommentListQuery, CreateProductCommentRequest, ProductCommentResponse,
    UpdateProductCommentRequest,
};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::order::OrderStatus;
use crate::models::product_comment::{ProductComment, ProductCommentStats, ProductCommentStatus};
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait ProductCommentService: Send + Sync {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductCommentRequest,
    ) -> AppResult<ProductComment>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductCommentRequest,
    ) -> AppResult<ProductComment>;
    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ProductComment>;
    async fn list_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCommentResponse>, i64)>;
    async fn list_by_user(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCommentResponse>, i64)>;
    async fn get_stats(&self, auth: &AuthUser, product_id: &str) -> AppResult<ProductCommentStats>;
    async fn admin_list(
        &self,
        auth: &AuthUser,
        query: &AdminProductCommentListQuery,
    ) -> AppResult<(Vec<ProductComment>, i64)>;
    async fn admin_update_status(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        status: ProductCommentStatus,
    ) -> AppResult<()>;
    async fn admin_reply(&self, auth: &AuthUser, id: SnowflakeId, reply: &str) -> AppResult<()>;
    async fn admin_delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
}

pub struct ProductCommentServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl ProductCommentServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ProductCommentService for ProductCommentServiceImpl {
    async fn create(
        &self,
        auth: &AuthUser,
        req: CreateProductCommentRequest,
    ) -> AppResult<ProductComment> {
        let user_id = auth.ensure_snowflake_user_id()?;

        let product_id = crate::types::snowflake_id::parse_id(&req.product_id)?;
        crate::models::product::find_by_id(&self.pool, product_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product"))?;

        let order_id = crate::types::snowflake_id::parse_id(&req.order_id)?;
        let order = crate::models::order::find_by_id(&self.pool, order_id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("order"))?;

        if order.user_id != user_id {
            return Err(AppError::Forbidden);
        }
        if order.status != OrderStatus::Completed {
            return Err(AppError::BadRequest("only_completed_can_review".into()));
        }

        if let Some(_existing) = crate::models::product_comment::find_by_product_order_user(
            &self.pool,
            product_id,
            order_id,
            user_id,
            auth.tenant_id(),
        )
        .await?
        {
            return Err(AppError::Conflict("already_reviewed".into()));
        }

        let has_item =
            crate::models::order_item::find_by_order_id(&self.pool, order_id, auth.tenant_id())
                .await?
                .iter()
                .any(|item| item.product_id == Some(product_id));
        if !has_item {
            return Err(AppError::BadRequest("product_not_in_order".into()));
        }

        crate::models::product_comment::insert(
            &self.pool,
            &crate::commands::CreateProductCommentCmd {
                product_id,
                order_id,
                user_id,
                rating: req.rating,
                title: req.title,
                content: req.content,
                images: req.images,
            },
            auth.tenant_id(),
        )
        .await
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateProductCommentRequest,
    ) -> AppResult<ProductComment> {
        let user_id = auth.ensure_snowflake_user_id()?;
        let existing = crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))?;

        if existing.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        crate::models::product_comment::update(
            &self.pool,
            &crate::commands::UpdateProductCommentCmd {
                id: existing.id,
                rating: req.rating,
                title: req.title,
                content: req.content,
                images: req.images,
            },
            auth.tenant_id(),
        )
        .await?;

        crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))
    }

    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        let user_id = auth.ensure_snowflake_user_id()?;
        let existing = crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))?;

        if existing.user_id != user_id {
            return Err(AppError::Forbidden);
        }

        crate::models::product_comment::delete_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<ProductComment> {
        crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))
    }

    async fn list_by_product(
        &self,
        auth: &AuthUser,
        product_id: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCommentResponse>, i64)> {
        let pid = crate::types::snowflake_id::parse_id(product_id)?;
        let (items, total) = crate::models::product_comment::find_by_product_paginated(
            &self.pool,
            pid,
            auth.tenant_id(),
            page,
            page_size,
        )
        .await?;
        Ok((
            items
                .into_iter()
                .map(ProductCommentResponse::from)
                .collect(),
            total,
        ))
    }

    async fn list_by_user(
        &self,
        auth: &AuthUser,
        user_id: SnowflakeId,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<ProductCommentResponse>, i64)> {
        let (items, total) = crate::models::product_comment::find_by_user_paginated(
            &self.pool,
            user_id,
            auth.tenant_id(),
            page,
            page_size,
        )
        .await?;
        Ok((
            items
                .into_iter()
                .map(ProductCommentResponse::from)
                .collect(),
            total,
        ))
    }

    async fn get_stats(&self, auth: &AuthUser, product_id: &str) -> AppResult<ProductCommentStats> {
        let pid = crate::types::snowflake_id::parse_id(product_id)?;
        crate::models::product_comment::get_stats(&self.pool, pid, auth.tenant_id()).await
    }

    async fn admin_list(
        &self,
        auth: &AuthUser,
        query: &AdminProductCommentListQuery,
    ) -> AppResult<(Vec<ProductComment>, i64)> {
        crate::models::product_comment::find_all_admin_paginated(
            &self.pool,
            auth.tenant_id(),
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(20),
            query.status.as_deref(),
        )
        .await
    }

    async fn admin_update_status(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        status: ProductCommentStatus,
    ) -> AppResult<()> {
        crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))?;
        crate::models::product_comment::update_status(&self.pool, id, status, auth.tenant_id())
            .await
    }

    async fn admin_reply(&self, auth: &AuthUser, id: SnowflakeId, reply: &str) -> AppResult<()> {
        crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))?;
        crate::models::product_comment::admin_reply(&self.pool, id, reply, auth.tenant_id()).await
    }

    async fn admin_delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        crate::models::product_comment::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("product_comment"))?;
        crate::models::product_comment::delete_by_id(&self.pool, id, auth.tenant_id()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::product_comment::CreateProductCommentRequest;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn make_service(
        pool: crate::db::Pool,
    ) -> crate::services::product_comment::ProductCommentServiceImpl {
        crate::services::product_comment::ProductCommentServiceImpl::new(Arc::new(pool))
    }

    fn auth_admin(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    fn auth_user(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            None,
        )
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

    async fn seed_product(pool: &crate::db::Pool) -> SnowflakeId {
        let p = crate::models::product::insert(
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
        p.id
    }

    async fn seed_completed_order(
        pool: &crate::db::Pool,
        user_id: i64,
        product_id: SnowflakeId,
    ) -> SnowflakeId {
        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        let order = crate::models::order::insert(
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

        crate::models::order_item::insert_batch(
            pool,
            vec![crate::commands::CreateOrderItemCmd {
                order_id: order.id,
                product_id: Some(*product_id),
                variant_id: None,
                title: "Test".to_string(),
                description: None,
                sku: None,
                unit_price: 1000,
                quantity: 1,
                subtotal: 1000,
                tax_amount: 0,
                cover_url: None,
                attributes: None,
            }],
            None,
        )
        .await
        .unwrap();

        crate::models::order::update_status(pool, order.id, "paid", Some("paid_at"), None)
            .await
            .unwrap();
        crate::models::order::update_shipped(pool, order.id, Some("TRK"), Some("UPS"), None)
            .await
            .unwrap();
        crate::models::order::update_status(
            pool,
            order.id,
            "completed",
            Some("completed_at"),
            None,
        )
        .await
        .unwrap();

        order.id
    }

    #[tokio::test]
    async fn create_comment_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        let comment = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 5,
                    title: Some("Great!".into()),
                    content: "Love it".into(),
                    images: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(comment.rating, 5);
        assert_eq!(comment.content, "Love it");
        assert_eq!(comment.product_id, pid);
        assert_eq!(comment.user_id, SnowflakeId(uid));
    }

    #[tokio::test]
    async fn create_comment_non_completed_order_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;

        let order_no = format!("ORD-{}", uuid::Uuid::now_v7().to_string().replace('-', ""));
        let order = crate::models::order::insert(
            &pool,
            &crate::commands::CreateOrderCmd {
                user_id: SnowflakeId(uid),
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

        let a = auth_user(uid);
        let err = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: order.id.to_string(),
                    rating: 5,
                    title: None,
                    content: "Test".into(),
                    images: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "only_completed_can_review"));
    }

    #[tokio::test]
    async fn create_comment_wrong_user_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let uid2 = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;

        let a = auth_user(uid2);
        let err = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 4,
                    title: None,
                    content: "Test".into(),
                    images: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn create_comment_duplicate_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        svc.create(
            &a,
            CreateProductCommentRequest {
                product_id: pid.to_string(),
                order_id: oid.to_string(),
                rating: 5,
                title: None,
                content: "First".into(),
                images: None,
            },
        )
        .await
        .unwrap();

        let err = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 4,
                    title: None,
                    content: "Second".into(),
                    images: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn update_comment_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        let c = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 3,
                    title: None,
                    content: "OK".into(),
                    images: None,
                },
            )
            .await
            .unwrap();

        let updated = svc
            .update(
                &a,
                c.id,
                crate::dto::product_comment::UpdateProductCommentRequest {
                    rating: Some(5),
                    title: Some("Updated!".into()),
                    content: Some("Actually great".into()),
                    images: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.rating, 5);
        assert_eq!(updated.content, "Actually great");
    }

    #[tokio::test]
    async fn delete_comment_by_owner() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        let c = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 4,
                    title: None,
                    content: "Nice".into(),
                    images: None,
                },
            )
            .await
            .unwrap();

        svc.delete(&a, c.id).await.unwrap();
        assert!(
            crate::models::product_comment::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn admin_update_status() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        let c = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 4,
                    title: None,
                    content: "Nice".into(),
                    images: None,
                },
            )
            .await
            .unwrap();

        let admin = auth_admin(None);
        svc.admin_update_status(&admin, c.id, ProductCommentStatus::Rejected)
            .await
            .unwrap();

        let found = crate::models::product_comment::find_by_id(&pool, c.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.status, ProductCommentStatus::Rejected);
    }

    #[tokio::test]
    async fn admin_reply_to_comment() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let uid = seed_user(&pool).await;
        let pid = seed_product(&pool).await;
        let oid = seed_completed_order(&pool, uid, pid).await;
        let a = auth_user(uid);

        let c = svc
            .create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating: 4,
                    title: None,
                    content: "Nice".into(),
                    images: None,
                },
            )
            .await
            .unwrap();

        let admin = auth_admin(None);
        svc.admin_reply(&admin, c.id, "Thanks for your review!")
            .await
            .unwrap();

        let found = crate::models::product_comment::find_by_id(&pool, c.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.admin_reply.unwrap(), "Thanks for your review!");
    }

    #[tokio::test]
    async fn get_stats_from_service() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let pid = seed_product(&pool).await;

        for rating in [5, 4, 3, 5, 5] {
            let uid = seed_user(&pool).await;
            let oid = seed_completed_order(&pool, uid, pid).await;
            let a = auth_user(uid);
            svc.create(
                &a,
                CreateProductCommentRequest {
                    product_id: pid.to_string(),
                    order_id: oid.to_string(),
                    rating,
                    title: None,
                    content: format!("Rating {rating}"),
                    images: None,
                },
            )
            .await
            .unwrap();
        }

        let admin = auth_admin(None);
        let stats = svc.get_stats(&admin, &pid.to_string()).await.unwrap();
        assert_eq!(stats.total_count, 5);
        assert!((stats.average_rating - 4.4).abs() < 0.01);
    }
}
