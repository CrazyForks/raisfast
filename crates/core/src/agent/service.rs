//! AgentService: glue between `raisfast-agent` (engine) and the `ai_*` tables.
//!
//! One turn = two-phase persistence (loop-engine §2 落库时序契约):
//!   running 置位 → 先落 user 行 → 引擎跑回合 → 落 assistant/tool 行 +
//!   `turn:meta` → 幂等推进 `last_seq` → 回 `open`。
//! Provider/base-url/key resolution is MVP env-driven (`RAISFAST_AI_*`) until the
//! `[ai]` config section lands.

use std::sync::Arc;

use raisfast_agent::provider::openai::OpenAiCompatProvider;
use raisfast_agent::{
    CancellationToken, ChatMessage, ChatRole, ModelProvider, TokenUsage, ToolCall, ToolRegistry,
    TurnConfig, TurnEngine, TurnError, TurnEvent, register_memory_tools,
};
use serde_json::json;

use crate::agent::memory_sql::ScopedMemory;
use crate::agent::models::ai_agent::AiAgent;
use crate::agent::models::ai_message::AiMessage;
use crate::agent::models::ai_session::AiSession;
use crate::agent::models::{ai_message, ai_session};
use crate::config::app::AiConfig;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;

/// Outcome of a service-level turn.
#[derive(Debug)]
pub struct AgentTurnResult {
    pub text: String,
    pub iterations: usize,
    pub tool_calls_made: usize,
    pub usage: Option<TokenUsage>,
    pub messages_appended: usize,
}

/// Create the model provider for an agent from the `[ai]` config section.
fn provider_for(agent: &AiAgent, ai: &AiConfig) -> AppResult<Arc<dyn ModelProvider>> {
    let default_base = match agent.provider.as_str() {
        "ollama" => "http://localhost:11434/v1",
        _ => "https://api.openai.com/v1",
    };
    let base = ai
        .base_url
        .clone()
        .unwrap_or_else(|| default_base.to_string());
    Ok(Arc::new(OpenAiCompatProvider::new(
        base,
        ai.api_key.clone(),
    )))
}

fn turn_error(e: TurnError) -> AppError {
    AppError::Internal(anyhow::anyhow!(e.to_string()))
}

/// Map a stored row back to the flat engine message (meta/system skipped).
fn row_to_chat_message(row: &AiMessage) -> Option<ChatMessage> {
    let role = match row.role.as_str() {
        "user" => ChatRole::User,
        "assistant" => ChatRole::Assistant,
        "tool" => ChatRole::Tool,
        _ => return None,
    };
    let tool_calls = row.tool_calls.as_ref().and_then(|v| {
        serde_json::from_value(v.clone())
            .ok()
            .filter(|calls: &Vec<ToolCall>| !calls.is_empty())
    });
    Some(ChatMessage {
        role,
        content: (!row.content.is_empty()).then_some(row.content.clone()),
        tool_calls,
        tool_call_id: row.tool_call_id.clone(),
    })
}

fn base_message_in(
    session_id: SnowflakeId,
    seq: i64,
    role: &str,
    kind: &str,
    content: &str,
) -> ai_message::AiMessageIn {
    ai_message::AiMessageIn {
        session_id,
        seq,
        role: role.to_string(),
        kind: kind.to_string(),
        content: content.to_string(),
        tool_calls: None,
        tool_call_id: None,
        tool_name: None,
        tool_success: None,
        tool_error: None,
        tool_elapsed_ms: None,
        tool_truncated: None,
        reasoning_content: None,
        usage: None,
    }
}

/// Persist the new messages the engine appended to `history`, one row each.
#[allow(clippy::too_many_arguments)]
async fn persist_delta(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    session_id: SnowflakeId,
    history: &[ChatMessage],
    appended_start: usize,
    seq: &mut i64,
    per_call_usage: &[TokenUsage],
    messages_appended: &mut usize,
) -> AppResult<()> {
    let mut usage_idx = 0usize;
    let mut last_tool_calls: Option<&Vec<ToolCall>> = None;

    for message in &history[appended_start..] {
        // The user message was persisted up-front by the caller.
        if message.role == ChatRole::User {
            continue;
        }
        let role_str = message.role.as_wire();
        let kind = match (&message.role, message.tool_calls.as_ref()) {
            (ChatRole::Assistant, Some(calls)) if !calls.is_empty() => "assistant_tool_calls",
            (ChatRole::Tool, _) => "tool_result",
            _ => "chat",
        };

        let mut row = base_message_in(
            session_id,
            *seq,
            role_str,
            kind,
            message.content.as_deref().unwrap_or(""),
        );
        if message.role == ChatRole::Assistant {
            if let Some(calls) = &message.tool_calls
                && !calls.is_empty()
            {
                row.tool_calls = Some(serde_json::to_value(calls).unwrap_or_default());
            }
            // Per-iteration usage, aligned with assistant rows in order.
            if let Some(u) = per_call_usage.get(usage_idx) {
                row.usage = Some(json!({
                    "input": u.input_tokens,
                    "output": u.output_tokens,
                }));
            }
            usage_idx += 1;
            last_tool_calls = message.tool_calls.as_ref();
        } else if message.role == ChatRole::Tool {
            row.tool_call_id = message.tool_call_id.clone();
            row.tool_name = last_tool_calls
                .and_then(|calls| {
                    message
                        .tool_call_id
                        .as_ref()
                        .and_then(|id| calls.iter().find(|c| &c.id == id))
                })
                .map(|c| c.name.clone());
            let output = message.content.as_deref().unwrap_or("");
            row.tool_success =
                Some(!output.starts_with("工具执行失败") && !output.starts_with("工具不存在"));
        }

        ai_message::append_message(pool, tenant_id, &row).await?;
        *seq += 1;
        *messages_appended += 1;
    }
    Ok(())
}

/// Run one turn for an existing session and persist its transcript.
pub async fn run_turn(
    pool: &crate::db::Pool,
    ai: &AiConfig,
    agent: &AiAgent,
    session_id: SnowflakeId,
    user: &str,
) -> AppResult<AgentTurnResult> {
    let tenant_id = agent.tenant_id.as_deref();
    let session = ai_session::find_session_by_id(pool, session_id, tenant_id).await?;
    if session.status == "running" {
        return Err(AppError::Conflict("session_busy".into()));
    }

    ai_session::set_session_status(pool, session_id, tenant_id, "running").await?;
    let executed = run_turn_inner(
        pool, ai, agent, session_id, tenant_id, user, None, None, None,
    )
    .await;
    // Always release the busy flag before propagating errors.
    let result = match executed {
        Ok(r) => r,
        Err(e) => {
            let _ = ai_session::set_session_status(pool, session_id, tenant_id, "open").await;
            return Err(e);
        }
    };
    ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
    Ok(result)
}

/// Streamed variant of [`run_turn`]: live `TurnEvent`s are pushed to
/// `on_event` (SSE sink) while the turn runs and persists. `extra_tools` are
/// domain tools bound to this session's actor (see `agent::tools`).
#[allow(clippy::too_many_arguments)]
pub async fn run_turn_streamed(
    pool: &crate::db::Pool,
    ai: &AiConfig,
    agent: &AiAgent,
    session_id: SnowflakeId,
    user: &str,
    extra_tools: ToolRegistry,
    cancel: Option<CancellationToken>,
    on_event: &mut (dyn FnMut(TurnEvent) + Send),
) -> AppResult<AgentTurnResult> {
    let tenant_id = agent.tenant_id.as_deref();
    let session = ai_session::find_session_by_id(pool, session_id, tenant_id).await?;
    if session.status == "running" {
        return Err(AppError::Conflict("session_busy".into()));
    }

    ai_session::set_session_status(pool, session_id, tenant_id, "running").await?;
    let executed = run_turn_inner(
        pool,
        ai,
        agent,
        session_id,
        tenant_id,
        user,
        Some(on_event),
        Some(extra_tools),
        cancel,
    )
    .await;
    let result = match executed {
        Ok(r) => r,
        Err(e) => {
            let _ = ai_session::set_session_status(pool, session_id, tenant_id, "open").await;
            return Err(e);
        }
    };
    ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
async fn run_turn_inner(
    pool: &crate::db::Pool,
    ai: &AiConfig,
    agent: &AiAgent,
    session_id: SnowflakeId,
    tenant_id: Option<&str>,
    user: &str,
    mut emitter: Option<&mut (dyn FnMut(TurnEvent) + Send)>,
    extra_tools: Option<ToolRegistry>,
    cancel: Option<CancellationToken>,
) -> AppResult<AgentTurnResult> {
    // Load existing transcript (meta/system rows skipped) for continuity.
    let existing =
        ai_message::list_messages_after(pool, session_id, tenant_id, None, 10_000).await?;
    let mut history: Vec<ChatMessage> = existing.iter().filter_map(row_to_chat_message).collect();
    let old_len = history.len();
    let had_system = !agent.system_prompt.is_empty();
    let system_opt = had_system.then_some(agent.system_prompt.as_str());

    // Two-phase (1): durable user row before any model call.
    let mut seq = ai_message::next_seq(pool, session_id, tenant_id).await?;
    ai_message::append_message(
        pool,
        tenant_id,
        &base_message_in(session_id, seq, "user", "chat", user),
    )
    .await?;
    seq += 1;

    // Build the engine with a scoped memory handle + its tools.
    let memory = ScopedMemory::new(pool.clone(), agent.tenant_id.clone(), agent.id);
    let mut tools = extra_tools.unwrap_or_default();
    register_memory_tools(&mut tools, memory.clone());
    let provider = provider_for(agent, ai)?;
    let engine = TurnEngine::new(
        provider,
        agent.model.clone(),
        Arc::new(tools),
        TurnConfig {
            max_iterations: agent.max_iterations.clamp(1, 50) as usize,
            temperature: agent.temperature,
        },
    )
    .with_memory(memory);
    let mut engine = engine;
    if let Some(c) = cancel {
        engine = engine.with_cancel(c);
    }

    let outcome = match emitter.take() {
        Some(mut cb) => engine
            .run_streamed(&mut history, system_opt, user, &mut cb)
            .await
            .map_err(turn_error)?,
        None => engine
            .run(&mut history, system_opt, user)
            .await
            .map_err(turn_error)?,
    };

    // Two-phase (2): persist assistant/tool rows appended by the engine.
    let appended_start = old_len + usize::from(had_system);
    let mut messages_appended = 0usize;
    persist_delta(
        pool,
        tenant_id,
        session_id,
        &history,
        appended_start,
        &mut seq,
        &outcome.per_call_usage,
        &mut messages_appended,
    )
    .await?;

    // turn:meta terminal row, then the idempotent cursor advance.
    let stop_reason = if outcome.cancelled {
        "cancelled"
    } else {
        "completed"
    };
    let meta = json!({
        "stop_reason": stop_reason,
        "iterations": outcome.iterations,
        "tool_calls_made": outcome.tool_calls_made,
        "usage_total": outcome.usage.as_ref().map(|u| json!({
            "input": u.input_tokens,
            "output": u.output_tokens,
        })),
    });
    ai_message::append_message(
        pool,
        tenant_id,
        &base_message_in(session_id, seq, "meta", "turn:meta", &meta.to_string()),
    )
    .await?;
    ai_session::advance_last_seq(pool, session_id, tenant_id, seq).await?;

    Ok(AgentTurnResult {
        text: outcome.text,
        iterations: outcome.iterations,
        tool_calls_made: outcome.tool_calls_made,
        usage: outcome.usage,
        messages_appended,
    })
}

// ── thin service wrappers for handlers ──────────────────────────────────────

/// Create an agent (admin).
#[allow(clippy::too_many_arguments)]
pub async fn create_agent(
    pool: &crate::db::Pool,
    tenant: Option<String>,
    owner: Option<SnowflakeId>,
    name: String,
    system_prompt: String,
    provider: String,
    model: String,
    temperature: Option<f64>,
    tools: Vec<String>,
    memory_enabled: bool,
) -> AppResult<AiAgent> {
    crate::agent::models::ai_agent::create_agent(
        pool,
        tenant.as_deref(),
        owner,
        &name,
        &system_prompt,
        &provider,
        &model,
        temperature,
        tools,
        memory_enabled,
    )
    .await
}

/// Find an agent by id (tenant-scoped).
pub async fn find_agent(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant: Option<&str>,
) -> AppResult<AiAgent> {
    crate::agent::models::ai_agent::find_agent_by_id(pool, id, tenant).await
}

/// List agents of a tenant (admin/selection).
pub async fn list_agents(pool: &crate::db::Pool, tenant: Option<&str>) -> AppResult<Vec<AiAgent>> {
    crate::agent::models::ai_agent::list_agents(pool, tenant).await
}

/// Create a session owned by `owner_id` on an agent.
pub async fn create_session(
    pool: &crate::db::Pool,
    tenant: Option<String>,
    agent_id: SnowflakeId,
    owner_id: SnowflakeId,
    title: &str,
) -> AppResult<AiSession> {
    crate::agent::models::ai_session::create_session(
        pool,
        tenant.as_deref(),
        agent_id,
        owner_id,
        title,
    )
    .await
}

/// Find a session by id (tenant-scoped).
pub async fn find_session(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant: Option<&str>,
) -> AppResult<AiSession> {
    crate::agent::models::ai_session::find_session_by_id(pool, id, tenant).await
}

/// Sessions of one agent owned by `owner_id`.
pub async fn list_my_sessions(
    pool: &crate::db::Pool,
    tenant: Option<&str>,
    agent_id: SnowflakeId,
    owner_id: SnowflakeId,
) -> AppResult<Vec<AiSession>> {
    let all = crate::agent::models::ai_session::list_sessions(pool, tenant, agent_id).await?;
    Ok(all.into_iter().filter(|s| s.owner_id == owner_id).collect())
}

/// Replay slice of the session log.
pub async fn list_messages(
    pool: &crate::db::Pool,
    session_id: SnowflakeId,
    tenant: Option<&str>,
    since: Option<i64>,
    limit: i64,
) -> AppResult<Vec<AiMessage>> {
    crate::agent::models::ai_message::list_messages_after(pool, session_id, tenant, since, limit)
        .await
}
