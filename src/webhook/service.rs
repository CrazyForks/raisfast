//! Webhook 订阅服务

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::utils::id::new_id_and_timestamp;
use crate::webhook::model;

/// Webhook 订阅服务
pub struct WebhookService {
    pool: Pool,
}

impl WebhookService {
    /// 创建服务实例
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 创建 webhook 订阅
    pub async fn create(
        &self,
        tenant_id: &str,
        url: String,
        events: Vec<String>,
        description: Option<String>,
        enabled: bool,
    ) -> AppResult<model::WebhookSubscription> {
        let (id, now) = new_id_and_timestamp();
        let secret = Self::generate_secret();
        let sub = model::WebhookSubscription {
            id,
            tenant_id: tenant_id.to_string(),
            url,
            secret,
            events: serde_json::to_string(&events).unwrap_or_default(),
            enabled,
            description,
            created_at: now.clone(),
            updated_at: now,
        };
        model::insert(&self.pool, &sub).await?;
        Ok(sub)
    }

    /// 分页查询订阅
    pub async fn list(
        &self,
        tenant_id: Option<&str>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<model::WebhookSubscription>, i64)> {
        model::find_paginated(&self.pool, tenant_id, page, page_size).await
    }

    /// 获取单个订阅
    pub async fn get(&self, id: &str) -> AppResult<model::WebhookSubscription> {
        model::find_by_id(&self.pool, id).await
    }

    /// 更新订阅
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        &self,
        id: &str,
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

    /// 删除订阅
    pub async fn delete(&self, id: &str) -> AppResult<()> {
        model::delete_by_id(&self.pool, id).await
    }

    /// 查找启用的订阅（供事件投递使用）
    pub async fn find_enabled(
        &self,
        tenant_id: &str,
    ) -> AppResult<Vec<model::WebhookSubscription>> {
        model::find_enabled_by_tenant(&self.pool, tenant_id).await
    }

    /// 生成随机 secret（32 字节 hex）
    fn generate_secret() -> String {
        use getrandom::getrandom;
        let mut buf = [0u8; 32];
        getrandom(&mut buf).unwrap_or_else(|e| {
            tracing::error!("failed to generate webhook secret: {e}");
            panic!("rng failure");
        });
        hex::encode(buf)
    }

    /// 用 HMAC-SHA256 对 payload 签名
    pub fn sign_payload(secret: &str, body: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;
        type HmacSha256 = Hmac<Sha256>;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap_or_else(|e| {
            tracing::error!("hmac init failed: {e}");
            panic!("hmac init failure");
        });
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_payload_deterministic() {
        let secret = "my-secret-key";
        let body = b"{\"event\":\"post.created\"}";
        let sig1 = WebhookService::sign_payload(secret, body);
        let sig2 = WebhookService::sign_payload(secret, body);
        assert_eq!(sig1, sig2, "same input should produce same signature");
    }

    #[test]
    fn sign_payload_different_secrets() {
        let body = b"test payload";
        let sig1 = WebhookService::sign_payload("secret-a", body);
        let sig2 = WebhookService::sign_payload("secret-b", body);
        assert_ne!(
            sig1, sig2,
            "different secrets should produce different signatures"
        );
    }

    #[test]
    fn sign_payload_different_bodies() {
        let secret = "shared-secret";
        let sig1 = WebhookService::sign_payload(secret, b"body-1");
        let sig2 = WebhookService::sign_payload(secret, b"body-2");
        assert_ne!(
            sig1, sig2,
            "different bodies should produce different signatures"
        );
    }

    #[test]
    fn sign_payload_is_hex_encoded_sha256_hmac() {
        let secret = "test-secret";
        let body = b"hello world";
        let sig = WebhookService::sign_payload(secret, body);
        assert_eq!(sig.len(), 64, "SHA256 HMAC hex should be 64 chars");
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_secret_length_and_format() {
        let s = WebhookService::generate_secret();
        assert_eq!(s.len(), 64, "32 bytes = 64 hex chars");
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn generate_secret_unique() {
        let s1 = WebhookService::generate_secret();
        let s2 = WebhookService::generate_secret();
        assert_ne!(s1, s2, "each secret should be unique");
    }
}
