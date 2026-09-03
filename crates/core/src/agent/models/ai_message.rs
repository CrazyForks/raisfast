//! Append-only conversation event log (`ai_messages`).
//!
//! One row = one `ConversationMessage` projection. `role` includes `meta` rows
//! (`turn:meta`, `context:summary`, `context:reset`) that are skipped on replay;
//! `usage` is carried on every assistant row (per LLM call); tool rows carry
//! `tool_success/error/elapsed_ms/truncated`. Multi-tenant (tenant_id filter).

use serde::{Deserialize, Serialize};

use crate::db::DbDriver;
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// Message/row column list used by hand-written reads (kept in one place).
const MESSAGE_COLS: &str = "id, tenant_id, session_id, seq, role, kind, content, tool_calls, \
    tool_call_id, tool_name, tool_success, tool_error, tool_elapsed_ms, tool_truncated, \
    reasoning_content, call_usage, created_at";

/// One append-only message row.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AiMessage {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub session_id: SnowflakeId,
    pub seq: i64,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_success: Option<bool>,
    pub tool_error: Option<String>,
    pub tool_elapsed_ms: Option<i64>,
    pub tool_truncated: Option<bool>,
    pub reasoning_content: Option<String>,
    #[sqlx(rename = "call_usage")]
    pub usage: Option<serde_json::Value>,
    pub created_at: Timestamp,
}

/// Payload for one append-only message row.
#[derive(Debug, Clone)]
pub struct AiMessageIn {
    pub session_id: SnowflakeId,
    pub seq: i64,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub tool_success: Option<bool>,
    pub tool_error: Option<String>,
    pub tool_elapsed_ms: Option<i64>,
    pub tool_truncated: Option<bool>,
    pub reasoning_content: Option<String>,
    pub usage: Option<serde_json::Value>,
}

/// Append one message row (tenant-scoped).
pub async fn append_message(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    msg: &AiMessageIn,
) -> AppResult<()> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "ai_messages",
        [
            "id" => id,
            "session_id" => msg.session_id,
            "seq" => msg.seq,
            "role" => &msg.role,
            "kind" => &msg.kind,
            "content" => &msg.content,
            "tool_calls" => msg.tool_calls.clone(),
            "tool_call_id" => msg.tool_call_id.as_deref(),
            "tool_name" => msg.tool_name.as_deref(),
            "tool_success" => msg.tool_success,
            "tool_error" => msg.tool_error.as_deref(),
            "tool_elapsed_ms" => msg.tool_elapsed_ms,
            "tool_truncated" => msg.tool_truncated,
            "reasoning_content" => msg.reasoning_content.as_deref(),
            "call_usage" => msg.usage.clone(),
            "created_at" => &now
        ],
        tenant: tenant_id
    )?;
    Ok(())
}

/// Next `seq` for a session (`MAX(seq)+1`); new sessions start at 1.
pub async fn next_seq(
    pool: &crate::db::Pool,
    session_id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<i64> {
    let sql = format!(
        "SELECT COALESCE(MAX(seq), 0) FROM ai_messages WHERE session_id = {}{}",
        crate::db::Driver::ph(1),
        tenant_filter(tenant_id, 2)
    );
    let mut q = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql)).bind(session_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    let max: i64 = q.fetch_one(pool).await?;
    Ok(max + 1)
}

/// Positional slice of the session log (`seq > since`), ordered ascending.
/// `since = None` returns the whole log; `meta` rows are included for auditing
/// and filtered out by the caller when replaying to the model.
pub async fn list_messages_after(
    pool: &crate::db::Pool,
    session_id: SnowflakeId,
    tenant_id: Option<&str>,
    since: Option<i64>,
    limit: i64,
) -> AppResult<Vec<AiMessage>> {
    let mut sql = format!(
        "SELECT {MESSAGE_COLS} FROM ai_messages WHERE session_id = {}",
        crate::db::Driver::ph(1)
    );
    let mut n = 2;
    if tenant_id.is_some() {
        sql.push_str(&format!(" AND tenant_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    if since.is_some() {
        sql.push_str(&format!(" AND seq > {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    sql.push_str(&format!(
        " ORDER BY seq ASC LIMIT {}",
        crate::db::Driver::ph(n)
    ));
    let mut q = sqlx::query_as::<_, AiMessage>(crate::db::safe_sql(&sql)).bind(session_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(since) = since {
        q = q.bind(since);
    }
    Ok(q.bind(limit).fetch_all(pool).await?)
}

/// `" AND tenant_id = {ph}"` for `Some`, `""` for `None`.
fn tenant_filter(tenant_id: Option<&str>, start_index: usize) -> String {
    tenant_id
        .map(|_| format!(" AND tenant_id = {}", crate::db::Driver::ph(start_index)))
        .unwrap_or_default()
}
