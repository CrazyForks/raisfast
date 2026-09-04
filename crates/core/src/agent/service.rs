//! AgentService: glue between `raisfast-agent` (engine) and the `ai_*` tables.
//!
//! One turn = two-phase persistence (loop-engine §2 落库时序契约):
//!   running 置位 → 先落 user 行 → 引擎跑回合 → 落 assistant/tool 行 +
//!   `turn:meta` → 幂等推进 `last_seq` → 回 `open`。
//! Provider/base-url/key resolution is MVP env-driven (`RAISFAST_AI_*`) until the
//! `[ai]` config section lands.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

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

/// In-process registry of sessions with a running turn. A session found
/// `running` in the DB but NOT here was left behind by a previous crash/panic
/// → AgentService recovers it to `open` automatically (single-process BaaS).
static ACTIVE_TURNS: OnceLock<Mutex<HashSet<i64>>> = OnceLock::new();

fn active_turns() -> &'static Mutex<HashSet<i64>> {
    ACTIVE_TURNS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Returns true when this process may start a turn on `session_id`.
fn claim_turn(session_id: i64) -> bool {
    active_turns().lock().unwrap().insert(session_id)
}

fn release_turn(session_id: i64) {
    active_turns().lock().unwrap().remove(&session_id);
}

/// Tool allowlist semantics: memory tools are always available; domain tools
/// only when named in `ai_agents.tools` (or `"*"`).
fn apply_tool_allowlist(tools: &mut ToolRegistry, agent: &AiAgent) {
    let memory = ["memory_store", "memory_recall", "memory_forget"];
    let allow: Vec<String> = agent
        .tools
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let all = allow.iter().any(|n| n == "*");
    tools.retain(|name| memory.contains(&name) || all || allow.iter().any(|a| a == name));
}

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
    if !claim_turn(session.id.0) {
        return Err(AppError::Conflict("session_busy".into()));
    }
    // Recover a session stuck `running` from a previous crash (not in this process).
    if session.status == "running" {
        tracing::warn!(session = session.id.0, "recovering stale running session");
        ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
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
            release_turn(session.id.0);
            return Err(e);
        }
    };
    ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
    release_turn(session.id.0);
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
    if !claim_turn(session.id.0) {
        return Err(AppError::Conflict("session_busy".into()));
    }
    if session.status == "running" {
        tracing::warn!(session = session.id.0, "recovering stale running session");
        ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
    }

    ai_session::set_session_status(pool, session_id, tenant_id, "running").await?;
    tracing::info!(
        session = session_id.0,
        agent = agent.id.0,
        "agent_service: streamed turn start"
    );
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
    tracing::debug!(
        session = session_id.0,
        err = executed.is_err(),
        "agent_service: streamed turn end"
    );
    let result = match executed {
        Ok(r) => r,
        Err(e) => {
            let _ = ai_session::set_session_status(pool, session_id, tenant_id, "open").await;
            release_turn(session.id.0);
            return Err(e);
        }
    };
    ai_session::set_session_status(pool, session_id, tenant_id, "open").await?;
    release_turn(session.id.0);
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
    // We always send an assembled framework system prompt (agent/system_prompt
    // is embedded inside it), so the engine always inserts one leading System.
    let had_system = true;

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
    apply_tool_allowlist(&mut tools, agent);

    // M5-A skills: resolve config + skills before building tool_names.
    let skills_root = crate::agent::skills::skills_root();
    let skill_enabled = crate::agent::skills::enabled_bundles(agent);
    let skills_full = !agent
        .params
        .as_ref()
        .and_then(|p| p.get("skills_mode"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|m| m == "compact");
    let loaded_skills =
        crate::agent::skills::load_skills(&skills_root, agent.tenant_id.as_deref(), &skill_enabled);
    // `read_skill` is only meaningful in Compact mode (Full already inlines).
    if !skill_enabled.is_empty() && !skills_full {
        tools.register(crate::agent::tools::skills::ReadSkillTool::new(
            skills_root.clone(),
            agent.tenant_id.clone(),
            skill_enabled.clone(),
        ));
    }
    // Composed `skill__<tool>` wrappers for declared, available platform tools
    // (§12-B): registered after the allowlist so availability is accurate.
    crate::agent::tools::skills::register_skill_composed(&mut tools, &loaded_skills);
    let tool_names = tools.names();

    // Load enabled skills and render the system section.
    let skills_section = crate::agent::skills::render_skills(&loaded_skills, skills_full);
    let assembled =
        crate::agent::prompt::assemble_with_skills(agent, &tool_names, skills_section.as_deref());
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
            .run_streamed(&mut history, Some(&assembled.text), user, &mut cb)
            .await
            .map_err(turn_error)?,
        None => engine
            .run(&mut history, Some(&assembled.text), user)
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
        "system_hash": assembled.hash,
        "prompt_version": assembled.version,
        "prompt": {
            "system_chars": assembled.system_chars,
            "skills_chars": assembled.skills_chars,
        },
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
    params: Option<serde_json::Value>,
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
        params,
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

/// Partial update payload for an agent (admin). Fields present are applied.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct AgentPatch {
    pub system_prompt: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub temperature: Option<f64>,
    pub max_iterations: Option<i32>,
    pub tools: Option<Vec<String>>,
    pub memory_enabled: Option<bool>,
    pub params: Option<serde_json::Value>,
}

/// Apply a partial patch to an agent (overlay on current row) and return it.
pub async fn update_agent(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    patch: &AgentPatch,
) -> AppResult<AiAgent> {
    let current = crate::agent::models::ai_agent::find_agent_by_id(pool, id, tenant_id).await?;
    let tools = match &patch.tools {
        Some(t) => serde_json::to_value(t).unwrap_or(serde_json::Value::Array(vec![])),
        None => current.tools,
    };
    crate::agent::models::ai_agent::update_agent(
        pool,
        tenant_id,
        id,
        patch
            .system_prompt
            .as_deref()
            .unwrap_or(&current.system_prompt),
        patch.provider.as_deref().unwrap_or(&current.provider),
        patch.model.as_deref().unwrap_or(&current.model),
        patch.temperature.or(current.temperature),
        patch.max_iterations.unwrap_or(current.max_iterations),
        tools,
        patch.memory_enabled.unwrap_or(current.memory_enabled),
        patch.params.clone().or(current.params),
    )
    .await?;
    crate::agent::models::ai_agent::find_agent_by_id(pool, id, tenant_id).await
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

/// Daily usage of one agent over the last `days` (default 30, clamped to 1-90).
/// Aggregated from `turn:meta` rows (`usage_total`/`tool_calls_made`), one row
/// per completed or cancelled turn — no schema/JSON-extraction per dialect.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentUsageDay {
    pub date: String,
    pub turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tool_calls: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentUsageReport {
    pub agent_id: SnowflakeId,
    pub days: i64,
    pub total_turns: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_tool_calls: i64,
    pub daily: Vec<AgentUsageDay>,
}

pub async fn usage_report(
    pool: &crate::db::Pool,
    tenant: Option<&str>,
    agent_id: SnowflakeId,
    days: i64,
) -> AppResult<AgentUsageReport> {
    let days = days.clamp(1, 90);
    let to = crate::utils::tz::now_utc();
    let from = to - chrono::Duration::days(days);
    let rows =
        ai_message::agent_turn_meta_rows(pool, tenant, agent_id, Some(from), Some(to)).await?;

    let mut buckets: BTreeMap<String, AgentUsageDay> = BTreeMap::new();
    for row in &rows {
        let Ok(meta) = serde_json::from_str::<serde_json::Value>(&row.content) else {
            continue;
        };
        let usage = meta
            .get("usage_total")
            .and_then(serde_json::Value::as_object);
        let input = usage
            .and_then(|u| u.get("input"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let output = usage
            .and_then(|u| u.get("output"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let tool_calls = meta
            .get("tool_calls_made")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let date = row.created_at.format("%Y-%m-%d").to_string();
        let bucket = buckets.entry(date.clone()).or_insert(AgentUsageDay {
            date,
            turns: 0,
            input_tokens: 0,
            output_tokens: 0,
            tool_calls: 0,
        });
        bucket.turns += 1;
        bucket.input_tokens += input;
        bucket.output_tokens += output;
        bucket.tool_calls += tool_calls;
    }

    let mut report = AgentUsageReport {
        agent_id,
        days,
        total_turns: 0,
        total_input_tokens: 0,
        total_output_tokens: 0,
        total_tool_calls: 0,
        daily: buckets.into_values().collect(),
    };
    for day in &report.daily {
        report.total_turns += day.turns;
        report.total_input_tokens += day.input_tokens;
        report.total_output_tokens += day.output_tokens;
        report.total_tool_calls += day.tool_calls;
    }
    Ok(report)
}
