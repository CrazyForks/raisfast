//! `llm_tokens` model — downstream sk- keys (design §5.2). Relay-facing
//! auth/billing lands in P3; P1 ships the row + CRUD essentials.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LlmTokenStatus {
        Enabled = "enabled",
        Disabled = "disabled",
        Expired = "expired",
        Exhausted = "exhausted",
    }
);

/// One downstream sk- token row (key stored as sha256 hash only).
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmToken {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub name: String,
    pub key_hash: String,
    pub status: LlmTokenStatus,
    pub remain_quota: i64,
    pub used_quota: i64,
    pub unlimited_quota: bool,
    pub expired_at: Option<Timestamp>,
    pub allowed_models: Option<String>,
    pub allowed_ips: Option<String>,
    pub token_group: Option<String>,
    pub created_at: Timestamp,
    pub accessed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// Create a token row (key_hash pre-computed by the caller).
#[allow(clippy::too_many_arguments)]
pub async fn create_token(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: SnowflakeId,
    name: &str,
    key_hash: &str,
    remain_quota: i64,
    unlimited_quota: bool,
    expired_at: Option<Timestamp>,
    allowed_models: Option<&str>,
    allowed_ips: Option<&str>,
) -> AppResult<LlmToken> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "llm_tokens",
        [
            "id" => id,
            "user_id" => user_id,
            "name" => name,
            "key_hash" => key_hash,
            "status" => LlmTokenStatus::Enabled.as_str(),
            "remain_quota" => remain_quota,
            "unlimited_quota" => unlimited_quota,
            "expired_at" => expired_at,
            "allowed_models" => allowed_models,
            "allowed_ips" => allowed_ips,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("llm_token insert vanished")))
}

/// Find a token by id (tenant-scoped).
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<LlmToken>> {
    let result: Option<LlmToken> = raisfast_derive::crud_find!(
        pool,
        "llm_tokens",
        LlmToken,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// Find a token by key hash (auth lookup, global unique).
pub async fn find_by_hash(pool: &crate::db::Pool, key_hash: &str) -> AppResult<Option<LlmToken>> {
    let result: Option<LlmToken> = raisfast_derive::crud_find!(
        pool,
        "llm_tokens",
        LlmToken,
        where: ("key_hash", key_hash)
    )?;
    Ok(result)
}

/// List a user's tokens.
pub async fn list_by_user(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: SnowflakeId,
) -> AppResult<Vec<LlmToken>> {
    let result: Vec<LlmToken> = raisfast_derive::crud_find_all!(
        pool,
        "llm_tokens",
        LlmToken,
        where: ("user_id", user_id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// Update token status.
pub async fn update_status(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    status: LlmTokenStatus,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "llm_tokens",
        bind: ["status" => status.as_str(), "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_token")
}

/// Delete a token by id (tenant-scoped).
pub async fn delete_token(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "llm_tokens",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_token")
}
