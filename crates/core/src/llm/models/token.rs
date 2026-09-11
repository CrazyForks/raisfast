//! `llm_tokens` model — downstream sk- keys (design §5.2). Relay-facing
//! auth/billing lands in P3; P1 ships the row + CRUD essentials.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::quota::Quota;
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

/// One downstream sk- token row. Auth looks up by `key_hash` (sha256);
/// `key_enc` holds the plaintext reversibly encrypted with APP_KEY for
/// later reveal/copy in the UI — the same pattern as `api_token`.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmToken {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub user_id: SnowflakeId,
    pub name: String,
    pub key_hash: String,
    pub key_enc: Option<String>,
    pub status: LlmTokenStatus,
    pub remain_quota: Quota,
    pub used_quota: Quota,
    pub unlimited_quota: bool,
    pub expired_at: Option<Timestamp>,
    pub allowed_models: Option<String>,
    pub allowed_ips: Option<String>,
    pub token_group: Option<String>,
    pub created_at: Timestamp,
    pub accessed_at: Option<Timestamp>,
    pub updated_at: Timestamp,
}

/// Create a token row from the plaintext key: sha256 for auth lookup +
/// APP_KEY-encrypted copy for reveal.
#[allow(clippy::too_many_arguments)]
pub async fn create_token(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    user_id: SnowflakeId,
    name: &str,
    key_plain: &str,
    remain_quota: Quota,
    unlimited_quota: bool,
    expired_at: Option<Timestamp>,
    allowed_models: Option<&str>,
    allowed_ips: Option<&str>,
    token_group: Option<&str>,
) -> AppResult<LlmToken> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    let key_hash = crate::services::api_token::hash_token(key_plain);
    let key_enc = crate::llm::crypto::encrypt(key_plain)?;
    raisfast_derive::crud_insert!(
        pool,
        "llm_tokens",
        [
            "id" => id,
            "user_id" => user_id,
            "name" => name,
            "key_hash" => key_hash,
            "key_enc" => key_enc,
            "status" => LlmTokenStatus::Enabled.as_str(),
            "remain_quota" => remain_quota,
            "unlimited_quota" => unlimited_quota,
            "expired_at" => expired_at,
            "allowed_models" => allowed_models,
            "allowed_ips" => allowed_ips,
            "token_group" => token_group,
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

/// List all tokens (admin view, tenant-scoped, newest first). `NOT_NULL` on
/// the primary key is a tautology — the macro requires a `where:` section.
pub async fn list_all(pool: &crate::db::Pool, tenant_id: Option<&str>) -> AppResult<Vec<LlmToken>> {
    let result: Vec<LlmToken> = raisfast_derive::crud_find_all!(
        pool,
        "llm_tokens",
        LlmToken,
        where: ("id", NOT_NULL),
        order_by: "created_at DESC",
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
