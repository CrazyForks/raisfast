//! Webhook 订阅数据模型与数据库查询

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};

/// Webhook 订阅完整数据库行
#[derive(Debug, FromRow, Serialize, Deserialize, Clone, TS)]
pub struct WebhookSubscription {
    pub id: String,
    pub tenant_id: String,
    pub url: String,
    pub secret: String,
    pub events: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

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
#[derive(Debug, Serialize, TS)]
pub struct WebhookPayload {
    pub event: String,
    #[ts(type = "unknown")]
    pub data: serde_json::Value,
    pub timestamp: String,
}

/// 插入一条 webhook 订阅
pub async fn insert(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "INSERT INTO webhook_subscriptions (id, tenant_id, url, secret, events, enabled, description, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    );
    sqlx::query(&sql)
        .bind(&sub.id)
        .bind(&sub.tenant_id)
        .bind(&sub.url)
        .bind(&sub.secret)
        .bind(&sub.events)
        .bind(sub.enabled)
        .bind(&sub.description)
        .bind(&sub.created_at)
        .bind(&sub.updated_at)
        .execute(pool)
        .await?;
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

    let mut where_parts = vec!["1=1".to_string()];
    if tenant_id.is_some() {
        where_parts.push("tenant_id = ?".to_string());
    }

    let where_str = where_parts.join(" AND ");
    let count_sql_raw = format!("SELECT COUNT(*) FROM webhook_subscriptions WHERE {where_str}");
    let data_sql_raw = format!(
        "SELECT * FROM webhook_subscriptions WHERE {where_str} ORDER BY created_at DESC LIMIT ? OFFSET ?"
    );
    let count_sql = crate::db::dialect::translate(&count_sql_raw);
    let data_sql = crate::db::dialect::translate(&data_sql_raw);

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
    let sql = crate::db::dialect::translate("SELECT * FROM webhook_subscriptions WHERE id = ?");
    sqlx::query_as::<_, WebhookSubscription>(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

/// 根据 ID 更新订阅
pub async fn update(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let sql = crate::db::dialect::translate(
        "UPDATE webhook_subscriptions SET url = ?, secret = ?, events = ?, enabled = ?, description = ?, updated_at = ? WHERE id = ?",
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
    let sql = crate::db::dialect::translate("DELETE FROM webhook_subscriptions WHERE id = ?");
    let result = sqlx::query(&sql).bind(id).execute(pool).await?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

/// 查找所有启用的订阅（用于事件投递）
pub async fn find_enabled_by_tenant(
    pool: &crate::db::Pool,
    tenant_id: &str,
) -> AppResult<Vec<WebhookSubscription>> {
    let sql = crate::db::dialect::translate(
        "SELECT * FROM webhook_subscriptions WHERE tenant_id = ? AND enabled = 1",
    );
    let items = sqlx::query_as::<_, WebhookSubscription>(&sql)
        .bind(tenant_id)
        .fetch_all(pool)
        .await?;
    Ok(items)
}
