//! `llm_channels` model — upstream channel with embedded key pool (design §5.1).

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LlmChannelStatus {
        Enabled = "enabled",
        ManualDisabled = "manual_disabled",
        AutoDisabled = "auto_disabled",
    }
);

define_enum!(
    LlmKeyMode {
        Polling = "polling",
        Random = "random",
    }
);

define_enum!(
    LlmKeyStatus {
        Active = "active",
        Disabled = "disabled",
    }
);

// Upstream cost model (pricing.md §7): `usage` bills per token via a discount
// on the directory price; `fixed` is a subscription upstream whose per-token
// cost is undefined — cost lives in `monthly_cost` and per-request
// `cost_quota` stays 0.
define_enum!(
    LlmCostMode {
        Usage = "usage",
        Fixed = "fixed",
    }
);

/// One key inside the channel pool (`keys` JSONB array element, design §5.1).
/// `key` holds the AES ciphertext (`enc:v1:…`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmKeyEntry {
    pub key: String,
    pub status: LlmKeyStatus,
    #[serde(default)]
    pub disabled_reason: Option<String>,
    #[serde(default)]
    pub disabled_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i32>,
}

/// Parse the `keys` JSONB column into entries (empty on malformed input).
pub fn parse_keys(channel: &LlmChannel) -> Vec<LlmKeyEntry> {
    serde_json::from_value(channel.api_keys.clone()).unwrap_or_default()
}

/// Serialize entries back into the `keys` JSONB value.
pub fn keys_value(entries: &[LlmKeyEntry]) -> serde_json::Value {
    serde_json::to_value(entries).unwrap_or(serde_json::Value::Array(vec![]))
}

/// One upstream channel row.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmChannel {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_keys: serde_json::Value,
    pub key_mode: LlmKeyMode,
    pub status: LlmChannelStatus,
    pub models: String,
    pub model_mapping: Option<serde_json::Value>,
    pub priority: i64,
    pub weight: i32,
    pub channel_groups: String,
    pub auto_ban: bool,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
    pub config: Option<serde_json::Value>,
    pub used_quota: i64,
    pub cost_mode: LlmCostMode,
    pub cost_discount: f64,
    pub monthly_cost: Option<f64>,
    pub test_model: Option<String>,
    pub test_time: Option<Timestamp>,
    pub response_time: Option<i32>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Payload for creating a channel (keys arrive pre-encrypted).
#[derive(Debug)]
pub struct NewChannel {
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub api_keys: serde_json::Value,
    pub key_mode: LlmKeyMode,
    pub models: String,
    pub model_mapping: Option<serde_json::Value>,
    pub priority: i64,
    pub weight: i32,
    pub channel_groups: String,
    pub auto_ban: bool,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
    pub config: Option<serde_json::Value>,
    pub cost_mode: LlmCostMode,
    pub cost_discount: f64,
    pub monthly_cost: Option<f64>,
    pub test_model: Option<String>,
}

/// Editable (non-key) columns for `update_channel`.
#[derive(Debug)]
pub struct ChannelChanges {
    pub name: String,
    pub provider: String,
    pub base_url: String,
    pub key_mode: LlmKeyMode,
    pub models: String,
    pub model_mapping: Option<serde_json::Value>,
    pub priority: i64,
    pub weight: i32,
    pub channel_groups: String,
    pub auto_ban: bool,
    pub param_override: Option<serde_json::Value>,
    pub header_override: Option<serde_json::Value>,
    pub config: Option<serde_json::Value>,
    pub cost_mode: LlmCostMode,
    pub cost_discount: f64,
    pub monthly_cost: Option<f64>,
    pub test_model: Option<String>,
}

/// Create a channel and return it.
pub async fn create_channel(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    n: NewChannel,
) -> AppResult<LlmChannel> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "llm_channels",
        [
            "id" => id,
            "name" => n.name,
            "provider" => n.provider,
            "base_url" => n.base_url,
            "api_keys" => n.api_keys,
            "key_mode" => n.key_mode.as_str(),
            "status" => LlmChannelStatus::Enabled.as_str(),
            "models" => n.models,
            "model_mapping" => n.model_mapping,
            "priority" => n.priority,
            "weight" => n.weight,
            "channel_groups" => n.channel_groups,
            "auto_ban" => n.auto_ban,
            "param_override" => n.param_override,
            "header_override" => n.header_override,
            "config" => n.config,
            "cost_mode" => n.cost_mode.as_str(),
            "cost_discount" => n.cost_discount,
            "monthly_cost" => n.monthly_cost,
            "test_model" => n.test_model,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("llm_channel insert vanished")))
}

/// Find a channel by id (tenant-scoped).
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<LlmChannel>> {
    let result: Option<LlmChannel> = raisfast_derive::crud_find!(
        pool,
        "llm_channels",
        LlmChannel,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// List channels of a tenant, newest first.
pub async fn list_channels(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
) -> AppResult<Vec<LlmChannel>> {
    let result: Vec<LlmChannel> = raisfast_derive::crud_list!(
        pool,
        "llm_channels",
        LlmChannel,
        order_by: "id DESC",
        tenant: tenant_id
    )?;
    Ok(result)
}

/// Update editable (non-key) columns.
pub async fn update_channel(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    c: ChannelChanges,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "llm_channels",
        bind: [
            "name" => c.name,
            "provider" => c.provider,
            "base_url" => c.base_url,
            "key_mode" => c.key_mode.as_str(),
            "models" => c.models,
            "model_mapping" => c.model_mapping,
            "priority" => c.priority,
            "weight" => c.weight,
            "channel_groups" => c.channel_groups,
            "auto_ban" => c.auto_ban,
            "param_override" => c.param_override,
            "header_override" => c.header_override,
            "config" => c.config,
            "cost_mode" => c.cost_mode.as_str(),
            "cost_discount" => c.cost_discount,
            "monthly_cost" => c.monthly_cost,
            "test_model" => c.test_model,
            "updated_at" => &now
        ],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_channel")
}

/// Delete a channel by id (tenant-scoped).
pub async fn delete_channel(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "llm_channels",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_channel")
}

/// Replace the whole key pool (write-only credential sub-resource, §11.5).
pub async fn replace_keys(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    entries: &[LlmKeyEntry],
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "llm_channels",
        bind: ["api_keys" => keys_value(entries), "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_channel")
}

/// Set one key's status inside the pool (read-modify-write in a transaction;
/// per-channel write serialization is enforced by the service layer, §6.2).
pub async fn update_key_status(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    key_index: usize,
    status: LlmKeyStatus,
    reason: Option<&str>,
) -> AppResult<Vec<LlmKeyEntry>> {
    crate::in_transaction!(pool, tx, {
        let row: LlmChannel = raisfast_derive::crud_find_one!(
            &mut *tx,
            "llm_channels",
            LlmChannel,
            where: ("id", id),
            tenant: tenant_id
        )?;
        let mut entries = parse_keys(&row);
        if key_index >= entries.len() {
            return Err(AppError::NotFound("llm_channel_key".to_owned()));
        }
        entries[key_index].status = status;
        if status == LlmKeyStatus::Active {
            // Enabling clears the disable bookkeeping — an active key must
            // not carry a stale reason/timestamp (§6.3 recovery semantics).
            entries[key_index].disabled_reason = None;
            entries[key_index].disabled_at = None;
        } else {
            entries[key_index].disabled_reason = reason.map(|r| r.to_owned());
            entries[key_index].disabled_at = Some(crate::utils::tz::now_str());
        }
        let now = now_utc();
        let result = raisfast_derive::crud_update!(
            &mut *tx,
            "llm_channels",
            bind: ["api_keys" => keys_value(&entries), "updated_at" => &now],
            where: ("id", id),
            tenant: tenant_id
        )?;
        AppError::expect_affected(&result, "llm_channel")?;
        Ok::<_, AppError>(entries)
    })
}

/// Set the channel-level status (manual enable/disable, auto ban).
pub async fn update_status(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    status: LlmChannelStatus,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "llm_channels",
        bind: ["status" => status.as_str(), "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_channel")
}
