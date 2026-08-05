use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::db::driver::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WebhookSubscription {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    pub url: String,
    pub secret: String,
    pub events: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: Option<String>,
    pub url: String,
    pub events: Vec<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub name: Option<String>,
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub secret: Option<String>,
}

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub event: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub data: serde_json::Value,
    pub timestamp: Timestamp,
}

pub async fn insert(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "webhook_subscriptions",
        [
            "id" => sub.id,
            "name" => &sub.name,
            "url" => &sub.url,
            "secret" => &sub.secret,
            "events" => &sub.events,
            "enabled" => sub.enabled,
            "description" => sub.description.as_deref(),
            "created_at" => now,
            "updated_at" => now
        ],
        tenant: sub.tenant_id.as_deref()
    )?;
    Ok(())
}

pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WebhookSubscription>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WebhookSubscription,
        table: "webhook_subscriptions",
        order_by: "created_at DESC",
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn find_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<WebhookSubscription> {
    let result: WebhookSubscription = raisfast_derive::crud_find_one!(pool, "webhook_subscriptions", WebhookSubscription, where: ("id", id))?;
    Ok(result)
}

pub async fn update(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(
        pool, "webhook_subscriptions",
        bind: ["name" => &sub.name, "url" => &sub.url, "secret" => &sub.secret, "events" => &sub.events, "enabled" => sub.enabled, "description" => &sub.description, "updated_at" => now],
        where: ("id", sub.id)
    )?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

pub async fn delete_by_id(pool: &crate::db::Pool, id: SnowflakeId) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "webhook_subscriptions", where: ("id", id))?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

pub async fn find_enabled_by_tenant(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<WebhookSubscription>> {
    Ok(
        raisfast_derive::crud_find_all!(pool, "webhook_subscriptions", WebhookSubscription, where: ("enabled", true), tenant: tenant_id)?,
    )
}

// ── Delivery log ──

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct WebhookDelivery {
    pub id: SnowflakeId,
    pub webhook_id: SnowflakeId,
    pub event: String,
    pub status: String,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at: Timestamp,
}

pub async fn insert_delivery(
    pool: &crate::db::Pool,
    webhook_id: SnowflakeId,
    event: &str,
    status: &str,
    status_code: Option<i32>,
    error: Option<&str>,
    duration_ms: Option<i64>,
) -> AppResult<()> {
    let id = crate::utils::id::new_snowflake_id();
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "webhook_deliveries",
        [
            "id" => id,
            "webhook_id" => webhook_id,
            "event" => event,
            "status" => status,
            "status_code" => status_code,
            "error" => error,
            "duration_ms" => duration_ms,
            "created_at" => now
        ]
    )?;
    Ok(())
}

pub async fn find_deliveries_by_webhook(
    pool: &crate::db::Pool,
    webhook_id: SnowflakeId,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WebhookDelivery>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WebhookDelivery,
        table: "webhook_deliveries",
        where: ("webhook_id", webhook_id),
        order_by: "created_at DESC",
        page: page,
        page_size: page_size
    );
    Ok(result)
}

pub async fn delete_deliveries_before(pool: &crate::db::Pool, before: Timestamp) -> AppResult<u64> {
    let sql = format!(
        "DELETE FROM webhook_deliveries WHERE created_at < {}",
        crate::db::Driver::ph(1)
    );
    let result = sqlx::query(&sql)
        .bind(before)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}
