use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::utils::id::{SnowflakeId, new_id_and_timestamp};
use crate::webhook::model;

pub struct WebhookService {
    pool: Pool,
}

impl WebhookService {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        tenant_id: Option<&str>,
        url: String,
        events: Vec<String>,
        description: Option<String>,
        enabled: bool,
        custom_secret: Option<String>,
    ) -> AppResult<model::WebhookSubscription> {
        let (id, now) = new_id_and_timestamp();
        let secret = custom_secret.unwrap_or_else(Self::generate_secret);
        let sub = model::WebhookSubscription {
            id: SnowflakeId(id),
            tenant_id: tenant_id.map(|t| t.to_string()),
            url,
            secret,
            events: serde_json::to_string(&events).unwrap_or_default(),
            enabled,
            description,
            created_at: now,
            updated_at: now,
        };
        model::insert(&self.pool, &sub).await?;
        let inserted = model::find_by_id(&self.pool, *sub.id).await?;
        Ok(inserted)
    }

    pub async fn list(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<model::WebhookSubscription>, i64)> {
        model::find_paginated(&self.pool, tenant_id, page, page_size).await
    }

    pub async fn get(&self, id: i64) -> AppResult<model::WebhookSubscription> {
        model::find_by_id(&self.pool, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: i64,
        url: Option<String>,
        events: Option<Vec<String>>,
        description: Option<String>,
        enabled: Option<bool>,
    ) -> AppResult<model::WebhookSubscription> {
        let mut sub = model::find_by_id(&self.pool, id).await?;
        let (_, now) = new_id_and_timestamp();
        if let Some(u) = url {
            sub.url = u;
        }
        if let Some(e) = events {
            sub.events = serde_json::to_string(&e).unwrap_or_default();
        }
        if description.is_some() {
            sub.description = description;
        }
        if let Some(en) = enabled {
            sub.enabled = en;
        }
        sub.updated_at = now;
        model::update(&self.pool, &sub).await?;
        Ok(sub)
    }

    pub async fn delete(&self, id: i64) -> AppResult<()> {
        model::delete_by_id(&self.pool, id).await
    }

    pub async fn find_enabled(
        &self,
        tenant_id: Option<&str>,
    ) -> AppResult<Vec<model::WebhookSubscription>> {
        model::find_enabled_by_tenant(&self.pool, tenant_id).await
    }

    fn generate_secret() -> String {
        crate::utils::id::random_hex(32)
    }

    pub fn sign_payload(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        type HmacSha256 = Hmac<sha2::Sha256>;
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_secret_length() {
        let secret = WebhookService::generate_secret();
        assert_eq!(secret.len(), 64);
    }
}
