use crate::types::price::Price;
use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

define_enum!(
    CouponType {
        Percent = "percent",
        Fixed = "fixed",
    }
);

define_enum!(
    CouponStatus {
        Active = "active",
        Inactive = "inactive",
    }
);

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct Coupon {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub code: String,
    pub title: String,
    pub coupon_type: CouponType,
    pub value: Price,
    pub min_order: Price,
    pub max_uses: i64,
    pub used_count: i64,
    pub max_uses_per_user: i64,
    pub starts_at: Option<Timestamp>,
    pub expires_at: Option<Timestamp>,
    pub status: CouponStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<Coupon>> {
    raisfast_derive::crud_find!(pool, "coupons", Coupon, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_by_code(
    pool: &crate::db::Pool,
    code: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<Coupon>> {
    raisfast_derive::crud_find!(
        pool,
        "coupons",
        Coupon,
        where: ("code", code),
        tenant: tenant_id
    )
    .map_err(Into::into)
}

pub async fn find_all_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    status: Option<&str>,
) -> AppResult<(Vec<Coupon>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool,
        Coupon,
        table: "coupons",
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
    cmd: &crate::commands::CreateCouponCmd,
    tenant_id: Option<&str>,
) -> AppResult<Coupon> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "coupons",
        [
            "id" => id,
            "code" => &cmd.code,
            "title" => &cmd.title,
            "coupon_type" => &cmd.coupon_type,
            "value" => cmd.value,
            "min_order" => cmd.min_order,
            "max_uses" => cmd.max_uses,
            "used_count" => 0i64,
            "max_uses_per_user" => cmd.max_uses_per_user,
            "starts_at" => crate::utils::tz::parse_rfc3339_opt(cmd.starts_at.as_deref()),
            "expires_at" => crate::utils::tz::parse_rfc3339_opt(cmd.expires_at.as_deref()),
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("coupon not found after insert")))
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdateCouponCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let existing = find_by_id(pool, cmd.id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("coupon"))?;

    let starts_at_ts = match cmd.starts_at.as_deref() {
        Some(s) => crate::utils::tz::parse_rfc3339_opt(Some(s)),
        None => existing.starts_at,
    };
    let expires_at_ts = match cmd.expires_at.as_deref() {
        Some(s) => crate::utils::tz::parse_rfc3339_opt(Some(s)),
        None => existing.expires_at,
    };

    let affected = raisfast_derive::crud_update!(
        pool,
        "coupons",
        bind: [
            "title" => cmd.title.as_deref().unwrap_or(&existing.title),
            "value" => cmd.value.unwrap_or(existing.value),
            "min_order" => cmd.min_order.unwrap_or(existing.min_order),
            "max_uses" => cmd.max_uses.unwrap_or(existing.max_uses),
            "max_uses_per_user" => cmd.max_uses_per_user.unwrap_or(existing.max_uses_per_user),
            "starts_at" => starts_at_ts,
            "expires_at" => expires_at_ts,
            "status" => cmd.status.as_deref().unwrap_or(existing.status.as_str()),
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn()],
        where: ("id", cmd.id),
        tenant: tenant_id
    )?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn increment_used(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!("coupons", "used_count", "updated_at", "id", "tenant_id");
    let sql = format!(
        "UPDATE coupons SET used_count = used_count + 1, updated_at = {} WHERE id = {}{}",
        crate::db::Driver::now_fn(),
        crate::db::Driver::ph(1),
        crate::db::tenant::tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql)).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(pool).await?;
    AppError::expect_affected(&result, "coupon")
}

pub async fn tx_increment_used(
    tx: &mut crate::db::pool::DbConnection,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    raisfast_derive::check_schema!("coupons", "used_count", "updated_at", "id", "tenant_id");
    let sql = format!(
        "UPDATE coupons SET used_count = used_count + 1, updated_at = {} WHERE id = {}{}",
        crate::db::Driver::now_fn(),
        crate::db::Driver::ph(1),
        crate::db::tenant::tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql)).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let result = q.execute(&mut *tx).await?;
    AppError::expect_affected(&result, "coupon")
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "coupons",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "coupon")
}

pub async fn count_user_uses(
    pool: &crate::db::Pool,
    coupon_id: SnowflakeId,
    user_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    raisfast_derive::check_schema!("orders", "coupon_id", "user_id", "tenant_id");
    let sql = format!(
        "SELECT COUNT(*) FROM orders WHERE coupon_id = {} AND user_id = {}{}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        crate::db::tenant::tenant_filter_ph(tenant_id, 3)
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql))
        .bind(coupon_id)
        .bind(user_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let count = q.fetch_one(pool).await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbDriver;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn seed_cmd(code: &str, value: Price) -> crate::commands::CreateCouponCmd {
        crate::commands::CreateCouponCmd {
            code: code.to_string(),
            title: format!("Coupon {code}"),
            coupon_type: "percent".to_string(),
            value,
            min_order: Price(0),
            max_uses: 0,
            max_uses_per_user: 1,
            starts_at: None,
            expires_at: None,
        }
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let code = format!("SAVE10_{}", crate::utils::id::new_id());
        let c = super::insert(&pool, &seed_cmd(&code, Price(10)), None)
            .await
            .unwrap();
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.code, code);
        assert_eq!(found.value.0, 10);
        assert_eq!(found.coupon_type, CouponType::Percent);
        assert_eq!(found.status, CouponStatus::Active);
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
    async fn find_by_code() {
        let pool = setup_pool().await;
        let code = format!("CODE20_{}", crate::utils::id::new_id());
        super::insert(&pool, &seed_cmd(&code, Price(20)), None)
            .await
            .unwrap();
        let found = super::find_by_code(&pool, &code, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.value.0, 20);
    }

    #[tokio::test]
    async fn find_by_code_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_code(&pool, "NOPE", None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_all_paginated() {
        let pool = setup_pool().await;
        let tenant = format!("t_{}", crate::utils::id::new_id());
        for i in 0..5 {
            super::insert(
                &pool,
                &seed_cmd(
                    &format!("C{i}_{}", crate::utils::id::new_id()),
                    Price(i as i64 * 10),
                ),
                Some(&tenant),
            )
            .await
            .unwrap();
        }
        let (items, total) = super::find_all_paginated(&pool, Some(&tenant), 1, 3, None)
            .await
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn find_all_paginated_status_filter() {
        let pool = setup_pool().await;
        let tenant = format!("t_{}", crate::utils::id::new_id());
        let a1_code = format!("A1_{}", crate::utils::id::new_id());
        super::insert(&pool, &seed_cmd(&a1_code, Price(10)), Some(&tenant))
            .await
            .unwrap();
        let c2 = super::insert(
            &pool,
            &seed_cmd(&format!("A2_{}", crate::utils::id::new_id()), Price(20)),
            Some(&tenant),
        )
        .await
        .unwrap();
        sqlx::query(crate::db::safe_sql(&format!(
            "UPDATE coupons SET status = 'inactive' WHERE id = {}",
            crate::db::Driver::ph(1)
        )))
        .bind(c2.id)
        .execute(&pool)
        .await
        .unwrap();

        let (items, total) = super::find_all_paginated(&pool, Some(&tenant), 1, 10, Some("active"))
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(items[0].code, a1_code);
    }

    #[tokio::test]
    async fn update_changes_title_and_value() {
        let pool = setup_pool().await;
        let c = super::insert(
            &pool,
            &seed_cmd(&format!("UPD_{}", crate::utils::id::new_id()), Price(10)),
            None,
        )
        .await
        .unwrap();
        let ok = super::update(
            &pool,
            &crate::commands::UpdateCouponCmd {
                id: c.id,
                title: Some("Updated Title".into()),
                value: Some(Price(20)),
                min_order: None,
                max_uses: None,
                max_uses_per_user: None,
                starts_at: None,
                expires_at: None,
                status: None,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.title, "Updated Title");
        assert_eq!(found.value.0, 20);
    }

    #[tokio::test]
    async fn update_status_to_inactive() {
        let pool = setup_pool().await;
        let c = super::insert(
            &pool,
            &seed_cmd(&format!("STAT_{}", crate::utils::id::new_id()), Price(10)),
            None,
        )
        .await
        .unwrap();
        super::update(
            &pool,
            &crate::commands::UpdateCouponCmd {
                id: c.id,
                title: None,
                value: None,
                min_order: None,
                max_uses: None,
                max_uses_per_user: None,
                starts_at: None,
                expires_at: None,
                status: Some("inactive".into()),
            },
            None,
        )
        .await
        .unwrap();
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.status, CouponStatus::Inactive);
    }

    #[tokio::test]
    async fn increment_used_increases_count() {
        let pool = setup_pool().await;
        let c = super::insert(
            &pool,
            &seed_cmd(&format!("USE_{}", crate::utils::id::new_id()), Price(10)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(c.used_count, 0);
        super::increment_used(&pool, c.id, None).await.unwrap();
        let found = super::find_by_id(&pool, c.id, None).await.unwrap().unwrap();
        assert_eq!(found.used_count, 1);
    }

    #[tokio::test]
    async fn delete_removes_coupon() {
        let pool = setup_pool().await;
        let c = super::insert(
            &pool,
            &seed_cmd(&format!("DEL_{}", crate::utils::id::new_id()), Price(10)),
            None,
        )
        .await
        .unwrap();
        super::delete_by_id(&pool, c.id, None).await.unwrap();
        assert!(
            super::find_by_id(&pool, c.id, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_not_found() {
        let pool = setup_pool().await;
        let err = super::delete_by_id(&pool, SnowflakeId(99999), None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::errors::app_error::AppError::NotFound(_)
        ));
    }

    #[tokio::test]
    async fn count_user_uses_returns_zero() {
        let pool = setup_pool().await;
        let c = super::insert(
            &pool,
            &seed_cmd(&format!("CU_{}", crate::utils::id::new_id()), Price(10)),
            None,
        )
        .await
        .unwrap();
        let count = super::count_user_uses(&pool, c.id, SnowflakeId(1), None)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
