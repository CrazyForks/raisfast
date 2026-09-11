//! `llm_logs` model — usage/billing log rows (design §5.3). Write path is
//! shared by relay (P3) and internal `execute` (P2); P1 ships row + insert.

use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LogSource {
        Relay = "relay",
        Agent = "agent",
        Kb = "kb",
        Flow = "flow",
        Test = "test",
    }
);

/// One usage-log row (`quota` stays 0 for internal calls, design §9.3).
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmLog {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub request_id: Option<String>,
    pub user_id: Option<SnowflakeId>,
    pub token_id: Option<SnowflakeId>,
    pub source: LogSource,
    pub channel_id: Option<SnowflakeId>,
    pub key_index: Option<i32>,
    pub model_name: String,
    pub is_stream: bool,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_write_tokens: i32,
    pub quota: i64,
    pub detail: Option<serde_json::Value>,
    pub elapsed_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
    pub created_at: Timestamp,
}

/// Payload for inserting a log row (best-effort: caller logs failures).
#[derive(Debug)]
pub struct NewLog {
    pub tenant_id: Option<String>,
    pub request_id: Option<String>,
    pub user_id: Option<SnowflakeId>,
    pub token_id: Option<SnowflakeId>,
    pub source: LogSource,
    pub channel_id: Option<SnowflakeId>,
    pub key_index: Option<i32>,
    pub model_name: String,
    pub is_stream: bool,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cache_read_tokens: i32,
    pub cache_write_tokens: i32,
    pub quota: i64,
    pub detail: Option<serde_json::Value>,
    pub elapsed_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error_message: Option<String>,
}

impl Default for NewLog {
    fn default() -> Self {
        Self {
            tenant_id: None,
            request_id: None,
            user_id: None,
            token_id: None,
            source: LogSource::Relay,
            channel_id: None,
            key_index: None,
            model_name: String::new(),
            is_stream: false,
            prompt_tokens: 0,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            quota: 0,
            detail: None,
            elapsed_ms: None,
            status_code: None,
            error_message: None,
        }
    }
}

/// Insert a usage-log row.
pub async fn insert_log(pool: &crate::db::Pool, l: NewLog) -> AppResult<()> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "llm_logs",
        [
            "id" => id,
            "request_id" => l.request_id,
            "user_id" => l.user_id,
            "token_id" => l.token_id,
            "source" => l.source.as_str(),
            "channel_id" => l.channel_id,
            "key_index" => l.key_index,
            "model_name" => l.model_name,
            "is_stream" => l.is_stream,
            "prompt_tokens" => l.prompt_tokens,
            "completion_tokens" => l.completion_tokens,
            "cache_read_tokens" => l.cache_read_tokens,
            "cache_write_tokens" => l.cache_write_tokens,
            "quota" => l.quota,
            "detail" => l.detail,
            "elapsed_ms" => l.elapsed_ms,
            "status_code" => l.status_code,
            "error_message" => l.error_message,
            "created_at" => &now
        ],
        tenant: l.tenant_id.as_deref()
    )?;
    Ok(())
}

/// Admin log query filters (§12).
#[derive(Debug, Default, serde::Deserialize)]
pub struct LogFilters {
    pub channel_id: Option<String>,
    pub token_id: Option<String>,
    pub model_name: Option<String>,
}

/// Paged admin log query.
pub async fn query_paged(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    filters: &LogFilters,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<LlmLog>, i64)> {
    use crate::types::snowflake_id::{SnowflakeId, parse_id};
    let channel: Option<SnowflakeId> = filters.channel_id.as_deref().and_then(|s| parse_id(s).ok());
    let token: Option<SnowflakeId> = filters.token_id.as_deref().and_then(|s| parse_id(s).ok());
    let model = filters.model_name.clone().filter(|m| !m.is_empty());

    Ok(raisfast_derive::crud_query_paged!(
        pool,
        LlmLog,
        table: "llm_logs",
        where: [
            "channel_id" => channel,
            "token_id" => token,
            "model_name" => model
        ],
        order_by: "id DESC",
        page: page,
        page_size: page_size,
        tenant: tenant_id
    ))
}
