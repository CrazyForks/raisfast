use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WebhookSubscription {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
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
    pub url: String,
    pub events: Vec<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
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
            "url" => &sub.url,
            "secret" => &sub.secret,
            "events" => &sub.events,
            "enabled" => sub.enabled,
            "description" => &sub.description,
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
        bind: ["url" => &sub.url, "secret" => &sub.secret, "events" => &sub.events, "enabled" => sub.enabled, "description" => &sub.description, "updated_at" => now],
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
