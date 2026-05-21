use serde::{Deserialize, Serialize};

use crate::db::DbDriver;
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct PaymentChannel {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub provider: String,
    pub name: String,
    pub is_live: i64,
    pub credentials: String,
    pub webhook_secret: Option<String>,
    pub settings: Option<String>,
    pub is_active: i64,
    pub sort_order: i64,
    pub version: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentChannel>> {
    raisfast_derive::crud_find!(pool, "payment_channels", PaymentChannel, where: ("id", id), tenant: tenant_id)
        .map_err(Into::into)
}

pub async fn find_all_active(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentChannel>> {
    raisfast_derive::crud_find_all!(pool, "payment_channels", PaymentChannel, where: ("is_active", 1_i64), tenant: tenant_id, order_by: "sort_order, created_at DESC")
        .map_err(Into::into)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    is_active: Option<bool>,
) -> AppResult<(Vec<PaymentChannel>, i64)> {
    let active_val = is_active.map(|a| if a { 1_i64 } else { 0_i64 });
    let result = raisfast_derive::crud_query_paged!(
        pool, PaymentChannel,
        table: "payment_channels",
        where: ["is_active" => active_val],
        order_by: "sort_order, created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePaymentChannelCmd,
    tenant_id: Option<&str>,
) -> AppResult<PaymentChannel> {
    let id = crate::utils::id::new_id();
    let is_live_val = if cmd.is_live { 1_i64 } else { 0_i64 };
    let is_active_val = if cmd.is_active { 1_i64 } else { 0_i64 };
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "payment_channels",
        [
            "id" => id,
            "provider" => &cmd.provider,
            "name" => &cmd.name,
            "is_live" => is_live_val,
            "credentials" => &cmd.credentials,
            "webhook_secret" => &cmd.webhook_secret,
            "settings" => &cmd.settings,
            "is_active" => is_active_val,
            "sort_order" => cmd.sort_order,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, SnowflakeId(id), tenant_id)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "inserted row not found: {id}"
            ))
        })
}

pub async fn update(
    pool: &crate::db::Pool,
    cmd: &crate::commands::UpdatePaymentChannelCmd,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let is_live_val = if cmd.is_live { 1_i64 } else { 0_i64 };
    let is_active_val = if cmd.is_active { 1_i64 } else { 0_i64 };
    let affected = raisfast_derive::crud_update!(
        pool, "payment_channels",
        bind: [
            "provider" => &cmd.provider,
            "name" => &cmd.name,
            "is_live" => is_live_val,
            "credentials" => &cmd.credentials,
            "webhook_secret" => &cmd.webhook_secret,
            "settings" => &cmd.settings,
            "is_active" => is_active_val,
            "sort_order" => cmd.sort_order,
        ],
        raw: ["updated_at" => crate::db::Driver::now_fn(), "version" => "version + 1"],
        where: AND(("id", cmd.id), ("version", cmd.version)),
        tenant: tenant_id
    )?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let affected =
        raisfast_derive::crud_delete!(pool, "payment_channels", where: ("id", id), tenant: tenant_id)?
            .rows_affected();
    Ok(affected > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    async fn seed_channel(pool: &crate::db::Pool, provider: &str) -> PaymentChannel {
        super::insert(
            pool,
            &crate::commands::CreatePaymentChannelCmd {
                provider: provider.into(),
                name: format!("{}-channel-{}", provider, uuid::Uuid::now_v7()),
                is_live: false,
                credentials: r#"{"api_key":"test"}"#.into(),
                webhook_secret: None,
                settings: None,
                is_active: true,
                sort_order: 0,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn insert_and_find_by_id() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "stripe").await;
        let found = super::find_by_id(&pool, ch.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, ch.id);
        assert_eq!(found.provider, "stripe");
        assert_eq!(found.is_live, 0);
        assert_eq!(found.is_active, 1);
        assert_eq!(found.version, 1);
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
    async fn find_all_active_channels() {
        let pool = setup_pool().await;
        let ch1 = seed_channel(&pool, "stripe").await;
        let _ch2 = seed_channel(&pool, "alipay").await;
        sqlx::query("UPDATE payment_channels SET is_active = 0 WHERE id = ?")
            .bind(ch1.id)
            .execute(&pool)
            .await
            .unwrap();
        let active = super::find_all_active(&pool, None).await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].provider, "alipay");
    }

    #[tokio::test]
    async fn find_all_admin_paginated_no_filter() {
        let pool = setup_pool().await;
        for _ in 0..3 {
            seed_channel(&pool, "stripe").await;
        }
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, None)
            .await
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn find_all_admin_paginated_status_filter() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "wxpay").await;
        sqlx::query("UPDATE payment_channels SET is_active = 0 WHERE id = ?")
            .bind(ch.id)
            .execute(&pool)
            .await
            .unwrap();
        let (items, total) = super::find_all_admin_paginated(&pool, None, 1, 10, Some(true))
            .await
            .unwrap();
        assert_eq!(total, 0);
        assert!(items.is_empty());
    }

    #[tokio::test]
    async fn update_changes_fields() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "stripe").await;
        let ok = super::update(
            &pool,
            &crate::commands::UpdatePaymentChannelCmd {
                id: ch.id,
                provider: "paypal".into(),
                name: "PayPal Live".into(),
                is_live: true,
                credentials: r#"{"client_id":"new"}"#.into(),
                webhook_secret: Some("secret123".into()),
                settings: Some(r#"{"currencies":["USD"]}"#.into()),
                is_active: false,
                sort_order: 5,
                version: ch.version,
            },
            None,
        )
        .await
        .unwrap();
        assert!(ok);
        let found = super::find_by_id(&pool, ch.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.provider, "paypal");
        assert_eq!(found.name, "PayPal Live");
        assert_eq!(found.is_live, 1);
        assert_eq!(found.is_active, 0);
        assert_eq!(found.sort_order, 5);
        assert_eq!(found.version, ch.version + 1);
    }

    #[tokio::test]
    async fn update_version_conflict() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "stripe").await;
        let ok = super::update(
            &pool,
            &crate::commands::UpdatePaymentChannelCmd {
                id: ch.id,
                provider: "stripe".into(),
                name: "name".into(),
                is_live: false,
                credentials: "{}".into(),
                webhook_secret: None,
                settings: None,
                is_active: true,
                sort_order: 0,
                version: 999,
            },
            None,
        )
        .await
        .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn delete_removes_channel() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "stripe").await;
        let ok = super::delete_by_id(&pool, ch.id, None).await.unwrap();
        assert!(ok);
        assert!(
            super::find_by_id(&pool, ch.id, None)
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
}
