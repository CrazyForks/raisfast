//! Webhook subscription data models and database queries

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::errors::app_error::{AppError, AppResult};
use crate::utils::tz::Timestamp;

/// Complete database row for a webhook subscription
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct WebhookSubscription {
    pub id: i64,
    pub document_id: String,
    pub tenant_id: Option<String>,
    pub url: String,
    pub secret: String,
    pub events: String,
    pub enabled: bool,
    pub description: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create subscription request body
#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub url: String,
    pub events: Vec<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub secret: Option<String>,
}

/// Update subscription request body
#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// Payload delivered to a webhook
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub event: String,
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub data: serde_json::Value,
    pub timestamp: Timestamp,
}

/// Inserts a webhook subscription
pub async fn insert(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "webhook_subscriptions",
        [
            "document_id" => &sub.document_id,
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

/// Paginated query for webhook subscriptions
pub async fn find_paginated(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<WebhookSubscription>, i64)> {
    let result = raisfast_derive::crud_query_paged!(
        pool, WebhookSubscription,
        data_sql: "SELECT * FROM webhook_subscriptions WHERE 1=1{tenant} ORDER BY created_at DESC",
        count_sql: "SELECT COUNT(*) FROM webhook_subscriptions WHERE 1=1{tenant}",
        binds: [],
        tenant: tenant_id,
        page: page,
        page_size: page_size
    );
    Ok(result)
}

/// Finds a subscription by ID
pub async fn find_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<WebhookSubscription> {
    raisfast_derive::crud_find_one!(pool, "webhook_subscriptions", WebhookSubscription, "document_id" => id)
        .map_err(Into::into)
}

/// Updates a subscription by ID
pub async fn update(pool: &crate::db::Pool, sub: &WebhookSubscription) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    let result = raisfast_derive::crud_update!(
        pool, "webhook_subscriptions",
        bind: ["url" => &sub.url, "secret" => &sub.secret, "events" => &sub.events, "enabled" => sub.enabled, "description" => &sub.description, "updated_at" => now],
        where: "document_id" => &sub.document_id
    )?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

pub async fn delete_by_id(pool: &crate::db::Pool, id: &str) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(pool, "webhook_subscriptions", "document_id" => id)?;
    AppError::expect_affected(&result, "webhook_subscription")?;
    Ok(())
}

/// Finds all enabled subscriptions (for event delivery)
pub async fn find_enabled_by_tenant(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<WebhookSubscription>> {
    Ok(
        raisfast_derive::crud_find_all!(pool, "webhook_subscriptions", WebhookSubscription, "enabled" => true, tenant: tenant_id)?,
    )
}
