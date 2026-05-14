use serde::{Deserialize, Serialize};

use crate::db::dialect::ph;
use crate::db::tenant::tenant_filter_ph;
use crate::errors::app_error::AppResult;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaymentChannel {
    pub id: i64,
    pub document_id: String,
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

crate::impl_from_row_opt_tenant!(PaymentChannel {
    required { id, document_id, provider, name, is_live, credentials, is_active, sort_order, version, created_at, updated_at }
    optional { webhook_secret, settings }
});

pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentChannel>> {
    let sql = format!(
        "SELECT * FROM payment_channels WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentChannel>(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_by_document_id(
    pool: &crate::db::Pool,
    document_id: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<PaymentChannel>> {
    let sql = format!(
        "SELECT * FROM payment_channels WHERE document_id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, PaymentChannel>(&sql).bind(document_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_optional(pool).await.map_err(Into::into)
}

pub async fn find_all_active(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<PaymentChannel>> {
    let sql = format!(
        "SELECT * FROM payment_channels WHERE is_active = 1{} ORDER BY sort_order, created_at DESC",
        tenant_filter_ph(tenant_id, 1)
    );
    let mut q = sqlx::query_as::<_, PaymentChannel>(&sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    q.fetch_all(pool).await.map_err(Into::into)
}

pub async fn find_all_admin_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
    is_active: Option<bool>,
) -> AppResult<(Vec<PaymentChannel>, i64)> {
    let offset = (page - 1) * page_size;
    let tenant_ph = tenant_filter_ph(tenant_id, 1);
    let has_tenant = tenant_id.is_some();
    let status_ph_idx = if has_tenant { 2 } else { 1 };
    let (count_sql, data_sql_base) = if let Some(active) = is_active {
        let val = if active { 1 } else { 0 };
        let _ = val;
        (
            format!(
                "SELECT COUNT(*) as count FROM payment_channels WHERE is_active = {}{}",
                ph(status_ph_idx),
                tenant_ph
            ),
            format!(
                "SELECT * FROM payment_channels WHERE is_active = {}{} ORDER BY sort_order, created_at DESC",
                ph(status_ph_idx),
                tenant_ph
            ),
        )
    } else {
        (
            format!(
                "SELECT COUNT(*) as count FROM payment_channels WHERE 1=1{}",
                tenant_ph
            ),
            format!(
                "SELECT * FROM payment_channels WHERE 1=1{} ORDER BY sort_order, created_at DESC",
                tenant_ph
            ),
        )
    };
    let mut q = sqlx::query_as::<_, (i64,)>(&count_sql);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(active) = is_active {
        q = q.bind(if active { 1_i64 } else { 0_i64 });
    }
    let (total,): (i64,) = q.fetch_one(pool).await?;
    let limit_base = status_ph_idx + usize::from(is_active.is_some());
    let sql = format!(
        "{} LIMIT {} OFFSET {}",
        data_sql_base,
        ph(limit_base + 1),
        ph(limit_base + 2)
    );
    let mut q2 = sqlx::query_as::<_, PaymentChannel>(&sql);
    if let Some(tid) = tenant_id {
        q2 = q2.bind(tid);
    }
    if let Some(active) = is_active {
        q2 = q2.bind(if active { 1_i64 } else { 0_i64 });
    }
    let rows = q2.bind(page_size).bind(offset).fetch_all(pool).await?;
    Ok((rows, total))
}

pub async fn insert(
    pool: &crate::db::Pool,
    cmd: &crate::commands::CreatePaymentChannelCmd,
    tenant_id: Option<&str>,
) -> AppResult<PaymentChannel> {
    let document_id = uuid::Uuid::now_v7().to_string();
    let is_live_val = if cmd.is_live { 1_i64 } else { 0_i64 };
    let is_active_val = if cmd.is_active { 1_i64 } else { 0_i64 };
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO payment_channels (document_id, tenant_id, provider, name, is_live, credentials, webhook_secret, settings, is_active, sort_order, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
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
                .bind(&document_id)
                .bind(tid)
                .bind(&cmd.provider)
                .bind(&cmd.name)
                .bind(is_live_val)
                .bind(&cmd.credentials)
                .bind(&cmd.webhook_secret)
                .bind(&cmd.settings)
                .bind(is_active_val)
                .bind(cmd.sort_order)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO payment_channels (document_id, provider, name, is_live, credentials, webhook_secret, settings, is_active, sort_order, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, datetime('now'), datetime('now'))",
                ph(1),
                ph(2),
                ph(3),
                ph(4),
                ph(5),
                ph(6),
                ph(7),
                ph(8),
                ph(9)
            );
            sqlx::query(&sql)
                .bind(&document_id)
                .bind(&cmd.provider)
                .bind(&cmd.name)
                .bind(is_live_val)
                .bind(&cmd.credentials)
                .bind(&cmd.webhook_secret)
                .bind(&cmd.settings)
                .bind(is_active_val)
                .bind(cmd.sort_order)
                .execute(pool)
                .await?;
        }
    }
    find_by_document_id(pool, &document_id, tenant_id)
        .await?
        .ok_or_else(|| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(
                "inserted row not found: {document_id}"
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
    let sql = format!(
        "UPDATE payment_channels SET provider={}, name={}, is_live={}, credentials={}, webhook_secret={}, settings={}, is_active={}, sort_order={}, updated_at=datetime('now'), version=version+1 WHERE id={} AND version={}{}",
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
        tenant_filter_ph(tenant_id, 11)
    );
    let mut q = sqlx::query(&sql)
        .bind(&cmd.provider)
        .bind(&cmd.name)
        .bind(is_live_val)
        .bind(&cmd.credentials)
        .bind(&cmd.webhook_secret)
        .bind(&cmd.settings)
        .bind(is_active_val)
        .bind(cmd.sort_order)
        .bind(cmd.id)
        .bind(cmd.version);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let affected = q.execute(pool).await?.rows_affected();
    Ok(affected > 0)
}

pub async fn delete_by_id(
    pool: &crate::db::Pool,
    id: i64,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let sql = format!(
        "DELETE FROM payment_channels WHERE id = {}{}",
        ph(1),
        tenant_filter_ph(tenant_id, 2)
    );
    let mut q = sqlx::query(&sql).bind(id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let affected = q.execute(pool).await?.rows_affected();
    Ok(affected > 0)
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
    async fn find_by_document_id_works() {
        let pool = setup_pool().await;
        let ch = seed_channel(&pool, "alipay").await;
        let found = super::find_by_document_id(&pool, &ch.document_id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, ch.id);
    }

    #[tokio::test]
    async fn find_by_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_id(&pool, 99999, None)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn find_by_document_id_not_found() {
        let pool = setup_pool().await;
        assert!(
            super::find_by_document_id(&pool, "nonexistent", None)
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
        let ok = super::delete_by_id(&pool, 99999, None).await.unwrap();
        assert!(!ok);
    }
}
