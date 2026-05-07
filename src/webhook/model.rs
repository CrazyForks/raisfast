//! Webhook 订阅数据模型与数据库查询

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};

/// Webhook 订阅完整数据库行
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebhookSubscription {
    pub id: String,
    pub tenant_id: Option<String>,
    pub url: String,
    pub secret: String,
    pub events: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

crate::impl_from_row_opt_tenant!(WebhookSubscription {
    required { id, url, secret, events, enabled, created_at, updated_at }
    optional { description }
});

/// 创建订阅请求体
#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub url: String,
    pub events: Vec<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// 更新订阅请求体
#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// 投递到 webhook 的 payload
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub event: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub data: serde_json::Value,
    pub timestamp: String,
}

/// 插入一条 webhook 订阅
pub async fn insert(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    match &sub.tenant_id {
        Some(tid) => {
            let sql = format!(
                "INSERT INTO webhook_subscriptions (id, tenant_id, url, secret, events, enabled, description, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {})",
                crate::db::dialect::ph(1),
                crate::db::dialect::ph(2),
                crate::db::dialect::ph(3),
                crate::db::dialect::ph(4),
                crate::db::dialect::ph(5),
                crate::db::dialect::ph(6),
                crate::db::dialect::ph(7),
                crate::db::dialect::ph(8),
                crate::db::dialect::ph(9)
            );
            sqlx::query(&sql)
                .bind(&sub.id)
                .bind(tid)
                .bind(&sub.url)
                .bind(&sub.secret)
                .bind(&sub.events)
                .bind(sub.enabled)
                .bind(&sub.description)
                .bind(&sub.created_at)
                .bind(&sub.updated_at)
                .execute(pool)
                .await?;
        }
        None => {
            let sql = format!(
                "INSERT INTO webhook_subscriptions (id, url, secret, events, enabled, description, created_at, updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {})",
                crate::db::dialect::ph(1),
                crate::db::dialect::ph(2),
                crate::db::dialect::ph(3),
                crate::db::dialect::ph(4),
                crate::db::dialect::ph(5),
                crate::db::dialect::ph(6),
                crate::db::dialect::ph(7),
                crate::db::dialect::ph(8)
            );
            sqlx::query(&sql)
                .bind(&sub.id)
                .bind(&sub.url)
                .bind(&sub.secret)
                .bind(&sub.events)
                .bind(sub.enabled)
                .bind(&sub.description)
                .bind(&sub.created_at)
                .bind(&sub.updated_at)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// 分页查询 webhook 订阅
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WebhookSubscription>, i64)> {
    let offset = (page - 1).max(0) * page_size;

    let mut ph_idx = 1usize;
    let mut where_parts = vec!["1=1".to_string()];
    if tenant_id.is_some() {
        where_parts.push(format!("tenant_id = {}", crate::db::dialect::ph(ph_idx)));
        ph_idx += 1;
    }

    let where_str = where_parts.join(" AND ");
    let count_sql = format!("SELECT COUNT(*) FROM webhook_subscriptions WHERE {where_str}");
    let data_sql = format!(
        "SELECT * FROM webhook_subscriptions WHERE {where_str} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        crate::db::dialect::ph(ph_idx),
        crate::db::dialect::ph(ph_idx + 1)
    );

    let mut cq = sqlx::query_scalar::<_, i64>(&count_sql);
    let mut dq = sqlx::query_as::<_, WebhookSubscription>(&data_sql);

    if let Some(tid) = tenant_id {
        cq = cq.bind(tid);
        dq = dq.bind(tid);
    }

    let total = cq.fetch_one(pool).await?;
    dq = dq.bind(page_size).bind(offset);
    let items = dq.fetch_all(pool).await?;

    Ok((items, total))
}

/// 根据 ID 查找订阅
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<WebhookSubscription> {
    let sql = format!(
        "SELECT * FROM webhook_subscriptions WHERE id = {}",
        crate::db::dialect::ph(1)
    );
    sqlx::query_as::<_, WebhookSubscription>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 根据 ID 更新订阅
pub async fn update(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let sql = format!(
        "UPDATE webhook_subscriptions SET url = {}, secret = {}, events = {}, enabled = {}, description = {}, updated_at = {} WHERE id = {}",
        crate::db::dialect::ph(1),
        crate::db::dialect::ph(2),
        crate::db::dialect::ph(3),
        crate::db::dialect::ph(4),
        crate::db::dialect::ph(5),
        crate::db::dialect::ph(6),
        crate::db::dialect::ph(7)
    );
    let result = sqlx::query(&sql)
        .bind(&sub.url)
        .bind(&sub.secret)
        .bind(&sub.events)
        .bind(sub.enabled)
        .bind(&sub.description)
        .bind(&sub.updated_at)
        .bind(&sub.id)
        .execute(pool)
        .await?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

/// 根据 ID 删除订阅
pub async fn delete_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM webhook_subscriptions WHERE id = {}",
        crate::db::dialect::ph(1)
    );
    let result = sqlx::query(&sql).bind(id).execute(pool).await?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

/// 查找所有启用的订阅（用于事件投递）
pub async fn find_enabled_by_tenant(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<WebhookSubscription>> {
    match tenant_id {
        Some(tid) => {
            let sql = format!(
                "SELECT * FROM webhook_subscriptions WHERE tenant_id = {} AND enabled = 1",
                crate::db::dialect::ph(1)
            );
            let items = sqlx::query_as::<_, WebhookSubscription>(&sql)
                .bind(tid)
                .fetch_all(pool)
                .await?;
            Ok(items)
        }
        None => {
            let sql = "SELECT * FROM webhook_subscriptions WHERE enabled = 1".to_string();
            let items = sqlx::query_as::<_, WebhookSubscription>(&sql)
                .fetch_all(pool)
                .await?;
            Ok(items)
        }
    }
}
