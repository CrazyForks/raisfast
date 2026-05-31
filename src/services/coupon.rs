use std::sync::Arc;

use async_trait::async_trait;

use crate::dto::coupon::{CouponResponse, CreateCouponRequest, UpdateCouponRequest};
use crate::errors::app_error::{AppError, AppResult};
use crate::middleware::auth::AuthUser;
use crate::models::coupon::{Coupon, CouponStatus, CouponType};
use crate::types::snowflake_id::SnowflakeId;

#[async_trait]
pub trait CouponService: Send + Sync {
    async fn create(&self, auth: &AuthUser, req: CreateCouponRequest) -> AppResult<Coupon>;
    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateCouponRequest,
    ) -> AppResult<Coupon>;
    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()>;
    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<Coupon>;
    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<CouponResponse>, i64)>;
    async fn validate_coupon(
        &self,
        coupon_id: Option<SnowflakeId>,
        coupon_code: Option<&str>,
        user_id: SnowflakeId,
        order_amount: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Coupon>;
    fn calculate_discount(&self, coupon: &Coupon, order_amount: i64) -> i64;
}

pub struct CouponServiceImpl {
    pool: Arc<crate::db::Pool>,
}

impl CouponServiceImpl {
    pub fn new(pool: Arc<crate::db::Pool>) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CouponService for CouponServiceImpl {
    async fn create(&self, auth: &AuthUser, req: CreateCouponRequest) -> AppResult<Coupon> {
        auth.ensure_admin()?;

        let existing =
            crate::models::coupon::find_by_code(&self.pool, &req.code, auth.tenant_id()).await?;
        if existing.is_some() {
            return Err(AppError::Conflict("coupon_code_exists".into()));
        }

        let coupon_type = req.coupon_type.as_deref().unwrap_or("percent");
        crate::models::coupon::insert(
            &self.pool,
            &crate::commands::CreateCouponCmd {
                code: req.code,
                title: req.title,
                coupon_type: coupon_type.to_string(),
                value: req.value,
                min_order: req.min_order.unwrap_or(0),
                max_uses: req.max_uses.unwrap_or(0),
                max_uses_per_user: req.max_uses_per_user.unwrap_or(1),
                starts_at: req.starts_at,
                expires_at: req.expires_at,
            },
            auth.tenant_id(),
        )
        .await
    }

    async fn update(
        &self,
        auth: &AuthUser,
        id: SnowflakeId,
        req: UpdateCouponRequest,
    ) -> AppResult<Coupon> {
        auth.ensure_admin()?;
        crate::models::coupon::update(
            &self.pool,
            &crate::commands::UpdateCouponCmd {
                id,
                title: req.title,
                value: req.value,
                min_order: req.min_order,
                max_uses: req.max_uses,
                max_uses_per_user: req.max_uses_per_user,
                starts_at: req.starts_at,
                expires_at: req.expires_at,
                status: req.status,
            },
            auth.tenant_id(),
        )
        .await?;
        crate::models::coupon::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("coupon"))
    }

    async fn delete(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<()> {
        auth.ensure_admin()?;
        crate::models::coupon::delete_by_id(&self.pool, id, auth.tenant_id()).await
    }

    async fn get(&self, auth: &AuthUser, id: SnowflakeId) -> AppResult<Coupon> {
        crate::models::coupon::find_by_id(&self.pool, id, auth.tenant_id())
            .await?
            .ok_or_else(|| AppError::not_found("coupon"))
    }

    async fn list(
        &self,
        auth: &AuthUser,
        page: i64,
        page_size: i64,
        status: Option<&str>,
    ) -> AppResult<(Vec<CouponResponse>, i64)> {
        let (items, total) = crate::models::coupon::find_all_paginated(
            &self.pool,
            auth.tenant_id(),
            page,
            page_size,
            status,
        )
        .await?;
        Ok((items.into_iter().map(CouponResponse::from).collect(), total))
    }

    async fn validate_coupon(
        &self,
        coupon_id: Option<SnowflakeId>,
        coupon_code: Option<&str>,
        user_id: SnowflakeId,
        order_amount: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<Coupon> {
        let coupon = match (coupon_id, coupon_code) {
            (Some(id), _) => crate::models::coupon::find_by_id(&self.pool, id, tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found("coupon"))?,
            (None, Some(code)) => crate::models::coupon::find_by_code(&self.pool, code, tenant_id)
                .await?
                .ok_or_else(|| AppError::not_found("coupon"))?,
            (None, None) => {
                return Err(AppError::BadRequest("coupon_id_or_code_required".into()));
            }
        };

        if coupon.status != CouponStatus::Active {
            return Err(AppError::BadRequest("coupon_not_active".into()));
        }

        let now = crate::utils::tz::now_utc();
        if let Some(ref starts) = coupon.starts_at
            && starts > &now
        {
            return Err(AppError::BadRequest("coupon_not_started".into()));
        }
        if let Some(ref expires) = coupon.expires_at
            && expires < &now
        {
            return Err(AppError::BadRequest("coupon_expired".into()));
        }

        if coupon.min_order > 0 && order_amount < coupon.min_order {
            return Err(AppError::BadRequest("coupon_min_order_not_met".into()));
        }

        if coupon.max_uses > 0 && coupon.used_count >= coupon.max_uses {
            return Err(AppError::BadRequest("coupon_max_uses_reached".into()));
        }

        if coupon.max_uses_per_user > 0 {
            let user_uses =
                crate::models::coupon::count_user_uses(&self.pool, coupon.id, user_id, tenant_id)
                    .await?;
            if user_uses >= coupon.max_uses_per_user {
                return Err(AppError::BadRequest("coupon_user_limit_reached".into()));
            }
        }

        Ok(coupon)
    }

    fn calculate_discount(&self, coupon: &Coupon, order_amount: i64) -> i64 {
        match coupon.coupon_type {
            CouponType::Percent => {
                let discount = order_amount * coupon.value / 100;
                discount.min(order_amount)
            }
            CouponType::Fixed => coupon.value.min(order_amount),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn make_service(pool: crate::db::Pool) -> CouponServiceImpl {
        CouponServiceImpl::new(Arc::new(pool))
    }

    fn auth_admin(tid: Option<&str>) -> AuthUser {
        AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            tid.map(|s| s.to_string()),
        )
    }

    fn auth_reader(user_int_id: i64) -> AuthUser {
        AuthUser::from_parts(
            Some(user_int_id),
            crate::models::user::UserRole::Reader,
            None,
        )
    }

    #[tokio::test]
    async fn create_coupon_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "SAVE10".into(),
                    title: "10% Off".into(),
                    coupon_type: Some("percent".into()),
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(c.code, "SAVE10");
        assert_eq!(c.value, 10);
    }

    #[tokio::test]
    async fn create_coupon_duplicate_code_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        svc.create(
            &a,
            CreateCouponRequest {
                code: "DUP".into(),
                title: "First".into(),
                coupon_type: None,
                value: 10,
                min_order: None,
                max_uses: None,
                max_uses_per_user: None,
                starts_at: None,
                expires_at: None,
            },
        )
        .await
        .unwrap();

        let err = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "DUP".into(),
                    title: "Second".into(),
                    coupon_type: None,
                    value: 20,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn create_coupon_requires_admin() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let a = auth_reader(1);
        let err = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "X".into(),
                    title: "T".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
    }

    #[tokio::test]
    async fn update_coupon_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "UPD".into(),
                    title: "Original".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let updated = svc
            .update(
                &a,
                c.id,
                UpdateCouponRequest {
                    title: Some("Updated".into()),
                    value: Some(20),
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                    status: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(updated.title, "Updated");
        assert_eq!(updated.value, 20);
    }

    #[tokio::test]
    async fn delete_coupon_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "DEL".into(),
                    title: "Delete Me".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        svc.delete(&a, c.id).await.unwrap();
        assert!(
            crate::models::coupon::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn validate_coupon_by_id_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "VAL".into(),
                    title: "Validate".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let validated = svc
            .validate_coupon(Some(c.id), None, SnowflakeId(1), 1000, None)
            .await
            .unwrap();
        assert_eq!(validated.code, "VAL");
    }

    #[tokio::test]
    async fn validate_coupon_by_code_success() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        svc.create(
            &a,
            CreateCouponRequest {
                code: "BYCODE".into(),
                title: "By Code".into(),
                coupon_type: None,
                value: 10,
                min_order: None,
                max_uses: None,
                max_uses_per_user: None,
                starts_at: None,
                expires_at: None,
            },
        )
        .await
        .unwrap();

        let validated = svc
            .validate_coupon(None, Some("BYCODE"), SnowflakeId(1), 1000, None)
            .await
            .unwrap();
        assert_eq!(validated.code, "BYCODE");
    }

    #[tokio::test]
    async fn validate_coupon_no_id_or_code_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let err = svc
            .validate_coupon(None, None, SnowflakeId(1), 1000, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn validate_coupon_min_order_not_met() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "MIN".into(),
                    title: "Min Order".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: Some(5000),
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let err = svc
            .validate_coupon(Some(c.id), None, SnowflakeId(1), 1000, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "coupon_min_order_not_met"));
    }

    #[tokio::test]
    async fn validate_coupon_inactive_fails() {
        let pool = setup_pool().await;
        let svc = make_service(pool.clone());
        let a = auth_admin(None);

        let c = svc
            .create(
                &a,
                CreateCouponRequest {
                    code: "INACT".into(),
                    title: "Inactive".into(),
                    coupon_type: None,
                    value: 10,
                    min_order: None,
                    max_uses: None,
                    max_uses_per_user: None,
                    starts_at: None,
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        sqlx::query("UPDATE coupons SET status = 'inactive' WHERE id = ?")
            .bind(c.id)
            .execute(&pool)
            .await
            .unwrap();

        let err = svc
            .validate_coupon(Some(c.id), None, SnowflakeId(1), 1000, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(ref s) if s == "coupon_not_active"));
    }

    #[tokio::test]
    async fn calculate_discount_percent() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let coupon = Coupon {
            id: SnowflakeId(1),
            tenant_id: None,
            code: "P10".into(),
            title: "10%".into(),
            coupon_type: CouponType::Percent,
            value: 10,
            min_order: 0,
            max_uses: 0,
            used_count: 0,
            max_uses_per_user: 1,
            starts_at: None,
            expires_at: None,
            status: CouponStatus::Active,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        };
        assert_eq!(svc.calculate_discount(&coupon, 1000), 100);
        assert_eq!(svc.calculate_discount(&coupon, 500), 50);
    }

    #[tokio::test]
    async fn calculate_discount_fixed() {
        let pool = setup_pool().await;
        let svc = make_service(pool);
        let coupon = Coupon {
            id: SnowflakeId(1),
            tenant_id: None,
            code: "F200".into(),
            title: "200 off".into(),
            coupon_type: CouponType::Fixed,
            value: 200,
            min_order: 0,
            max_uses: 0,
            used_count: 0,
            max_uses_per_user: 1,
            starts_at: None,
            expires_at: None,
            status: CouponStatus::Active,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        };
        assert_eq!(svc.calculate_discount(&coupon, 1000), 200);
        assert_eq!(svc.calculate_discount(&coupon, 100), 100);
    }
}
