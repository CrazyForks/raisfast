//! HTTP handlers for the AI agent core.
//!
//! Thin layer: parse params/auth, delegate to `agent::service`, shape
//! responses. `/api/v1/ai/*` are user-facing (authed, owner-scoped);
//! `/api/v1/admin/ai/*` are admin-scoped.

use std::convert::Infallible;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::sse::{Event as SseEvent, Sse};
use futures::Stream;
use raisfast_agent::CancellationToken;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::AppState;
use crate::agent::service as ai_service;
use crate::agent::service::AgentTurnResult;
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;

/// Register routes. Paths are prefixed `/api/v1` by `reg_route!`.
pub fn routes(
    registry: &mut crate::server::RouteRegistry,
    _config: &crate::config::app::AppConfig,
) -> axum::Router<AppState> {
    let r = axum::Router::new();
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/admin/ai/agents",
        post,
        admin_create_agent,
        "system",
        "admin/ai/agents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/admin/ai/agents",
        get,
        admin_list_agents,
        "system",
        "admin/ai/agents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/admin/ai/agents/{id}",
        put,
        admin_update_agent,
        "system",
        "admin/ai/agents",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/ai/agents/{agent_id}/sessions",
        post,
        create_session,
        "system",
        "ai/sessions",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/ai/agents/{agent_id}/sessions",
        get,
        list_my_sessions,
        "system",
        "ai/sessions",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/ai/sessions/{id}/messages",
        get,
        get_messages,
        "system",
        "ai/sessions",
        "authed"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/admin/ai/agents/{id}/usage",
        get,
        admin_agent_usage,
        "system",
        "admin/ai/agents/usage",
        "admin"
    );
    let r = reg_route!(
        r,
        registry,
        _config.api_restful,
        "/ai/sessions/{id}/compact",
        post,
        compact_session,
        "system",
        "ai/sessions/compact",
        "authed"
    );
    reg_route!(
        r,
        registry,
        _config.api_restful,
        "/ai/sessions/{id}/turns",
        post,
        run_turn,
        "system",
        "ai/sessions",
        "authed"
    )
}

#[derive(Deserialize)]
pub struct CreateAgentReq {
    pub name: String,
    pub system_prompt: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default = "default_true")]
    pub memory_enabled: bool,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
}

fn default_true() -> bool {
    true
}

pub async fn admin_create_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateAgentReq>,
) -> AppResult<ApiResponse<crate::agent::models::ai_agent::AiAgent>> {
    auth.ensure_admin()?;
    let agent = ai_service::create_agent(
        &state.pool,
        auth.tenant_id().map(str::to_string),
        auth.user_id().map(SnowflakeId),
        body.name,
        body.system_prompt,
        body.provider,
        body.model,
        body.temperature,
        body.tools,
        body.memory_enabled,
        body.params,
    )
    .await?;
    Ok(ApiResponse::success(agent))
}

pub async fn admin_update_agent(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(patch): Json<crate::agent::service::AgentPatch>,
) -> AppResult<ApiResponse<crate::agent::models::ai_agent::AiAgent>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let agent =
        crate::agent::service::update_agent(&state.pool, auth.tenant_id(), id, &patch).await?;
    Ok(ApiResponse::success(agent))
}

pub async fn admin_list_agents(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Vec<crate::agent::models::ai_agent::AiAgent>>> {
    auth.ensure_admin()?;
    let agents = ai_service::list_agents(&state.pool, auth.tenant_id()).await?;
    Ok(ApiResponse::success(agents))
}

#[derive(Deserialize)]
pub struct UsageQuery {
    /// Days of history to aggregate (default 30, clamped 1-90).
    #[serde(default = "default_usage_days")]
    pub days: i64,
}

fn default_usage_days() -> i64 {
    30
}

/// `GET /admin/ai/agents/{id}/usage` — daily LLM usage aggregation.
pub async fn admin_agent_usage(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<UsageQuery>,
) -> AppResult<ApiResponse<crate::agent::service::AgentUsageReport>> {
    auth.ensure_admin()?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    // Agent must exist in this tenant before aggregating its usage.
    let _agent = ai_service::find_agent(&state.pool, id, auth.tenant_id()).await?;
    let report = ai_service::usage_report(&state.pool, auth.tenant_id(), id, q.days).await?;
    Ok(ApiResponse::success(report))
}

#[derive(Deserialize)]
pub struct CreateSessionReq {
    #[serde(default)]
    pub title: Option<String>,
}

pub async fn create_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(body): Json<CreateSessionReq>,
) -> AppResult<ApiResponse<crate::agent::models::ai_session::AiSession>> {
    let owner = current_owner(&auth)?;
    let agent_id = crate::types::snowflake_id::parse_id(&agent_id)?;
    let tenant = auth.tenant_id().map(str::to_string);
    // Agent must exist in this tenant.
    let _agent = ai_service::find_agent(&state.pool, agent_id, tenant.as_deref()).await?;
    let session = ai_service::create_session(
        &state.pool,
        tenant,
        agent_id,
        owner,
        body.title.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(ApiResponse::success(session))
}

pub async fn list_my_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> AppResult<ApiResponse<Vec<crate::agent::models::ai_session::AiSession>>> {
    let owner = current_owner(&auth)?;
    let agent_id = crate::types::snowflake_id::parse_id(&agent_id)?;
    let sessions =
        ai_service::list_my_sessions(&state.pool, auth.tenant_id(), agent_id, owner).await?;
    Ok(ApiResponse::success(sessions))
}

#[derive(Deserialize)]
pub struct MessagesQuery {
    #[serde(default)]
    pub after_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<i64>,
}

pub async fn get_messages(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> AppResult<ApiResponse<Vec<crate::agent::models::ai_message::AiMessage>>> {
    let owner = current_owner(&auth)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let session = ai_service::find_session(&state.pool, id, auth.tenant_id()).await?;
    if session.owner_id != owner {
        return Err(AppError::ForbiddenOwnership);
    }
    let messages = ai_service::list_messages(
        &state.pool,
        session.id,
        auth.tenant_id(),
        q.after_seq,
        q.limit.unwrap_or(200).clamp(1, 1000),
    )
    .await?;
    Ok(ApiResponse::success(messages))
}

#[derive(Deserialize)]
pub struct TurnReq {
    pub content: String,
}

/// `POST /api/v1/ai/sessions/{id}/compact` — manual LLM compaction (opencode
/// `/compact` analog). Folds the oldest turns into a durable summary now.
pub async fn compact_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    let owner = current_owner(&auth)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let session = ai_service::find_session(&state.pool, id, auth.tenant_id()).await?;
    if session.owner_id != owner {
        return Err(AppError::ForbiddenOwnership);
    }
    let agent = ai_service::find_agent(&state.pool, session.agent_id, auth.tenant_id()).await?;
    let result = ai_service::compact_session(
        &state.pool,
        &state.config.ai,
        &agent,
        session.id,
        auth.tenant_id(),
    )
    .await?;
    Ok(ApiResponse::success(json!({
        "compacted": result.is_some(),
        "cover_seq": result.as_ref().map(|(c, _)| c),
        "summary": result.map(|(_, s)| s),
    })))
}

/// `POST /api/v1/ai/sessions/{id}/turns` — streamed SSE of one turn.
pub async fn run_turn(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TurnReq>,
) -> AppResult<Sse<CancelOnDrop<ReceiverStream<Result<SseEvent, Infallible>>>>> {
    let owner = current_owner(&auth)?;
    let id = crate::types::snowflake_id::parse_id(&id)?;
    let session = ai_service::find_session(&state.pool, id, auth.tenant_id()).await?;
    if session.owner_id != owner {
        return Err(AppError::ForbiddenOwnership);
    }
    let agent = ai_service::find_agent(&state.pool, session.agent_id, auth.tenant_id()).await?;
    let extra_tools = crate::agent::tools::build_domain_tools(&state, &auth);

    let pool = state.pool.clone();
    let ai_cfg = state.config.ai.clone();
    let emitter = state.emitter.clone();
    let broadcast = state.config.ai.broadcast_events;
    let content = body.content;

    let cancel = CancellationToken::new();
    let task_cancel = cancel.clone();
    let (tx, rx) = mpsc::channel::<Result<SseEvent, Infallible>>(64);
    tracing::info!(session = session.id.0, "ai turn: starting (streamed)");
    tokio::spawn(async move {
        let mut emit = |ev: raisfast_agent::TurnEvent| {
            let _ = tx.try_send(Ok(agent_event(ev)));
        };
        let result = ai_service::run_turn_streamed(
            &pool,
            &ai_cfg,
            &agent,
            session.id,
            &content,
            extra_tools,
            Some(task_cancel),
            &mut emit,
        )
        .await;

        match result {
            Ok(outcome) => {
                tracing::info!(
                    session = session.id.0,
                    text_len = outcome.text.len(),
                    "ai turn: done"
                );
                if broadcast {
                    emitter.emit(crate::event::Event::Custom {
                        source: "ai".to_string(),
                        event_type: "ai.turn.done".to_string(),
                        data: json!({
                            "session_id": session.id.0,
                            "agent_id": agent.id.0,
                            "text": outcome.text,
                            "iterations": outcome.iterations,
                            "tool_calls_made": outcome.tool_calls_made,
                        }),
                    });
                }
                let _ = tx.send(Ok(done_event(&outcome))).await;
            }
            Err(e) => {
                tracing::warn!(session = session.id.0, error = %e, "ai turn: failed");
                if broadcast {
                    emitter.emit(crate::event::Event::Custom {
                        source: "ai".to_string(),
                        event_type: "ai.turn.error".to_string(),
                        data: json!({
                            "session_id": session.id.0,
                            "agent_id": agent.id.0,
                            "message": e.to_string(),
                        }),
                    });
                }
                let _ = tx
                    .send(Ok(SseEvent::default().event("error").data(
                        json!({
                            "code": "turn_failed",
                            "message": e.to_string(),
                            "fatal": true,
                        })
                        .to_string(),
                    )))
                    .await;
            }
        }
    });

    Ok(Sse::new(CancelOnDrop {
        inner: ReceiverStream::new(rx),
        cancel,
    }))
}

/// Stream wrapper that cancels the running turn when the SSE response is
/// dropped (client disconnected). The engine then stops at the next checkpoint
/// and the service persists the partial transcript.
pub struct CancelOnDrop<S> {
    inner: S,
    cancel: CancellationToken,
}

impl<S: Stream + Unpin> Stream for CancelOnDrop<S> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl<S> Drop for CancelOnDrop<S> {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn current_owner(auth: &AuthUser) -> AppResult<SnowflakeId> {
    auth.user_id()
        .map(SnowflakeId)
        .ok_or(AppError::Unauthorized)
}

fn agent_event(ev: raisfast_agent::TurnEvent) -> SseEvent {
    match ev {
        raisfast_agent::TurnEvent::Chunk { delta } => SseEvent::default()
            .event("chunk")
            .data(json!({ "delta": delta }).to_string()),
        raisfast_agent::TurnEvent::Thinking { delta } => SseEvent::default()
            .event("thinking")
            .data(json!({ "delta": delta }).to_string()),
        raisfast_agent::TurnEvent::Text { text } => SseEvent::default()
            .event("text")
            .data(json!({ "text": text }).to_string()),
        raisfast_agent::TurnEvent::ToolCall { name, arguments } => SseEvent::default()
            .event("tool_call")
            .data(json!({ "name": name, "args": arguments }).to_string()),
        raisfast_agent::TurnEvent::ToolResult { name, output } => SseEvent::default()
            .event("tool_result")
            .data(json!({ "name": name, "output": output, "success": true }).to_string()),
    }
}

fn done_event(outcome: &AgentTurnResult) -> SseEvent {
    let data = json!({
        "outcome": {
            "text": outcome.text,
            "iterations": outcome.iterations,
            "tool_calls_made": outcome.tool_calls_made,
            "usage": outcome.usage.as_ref().map(|u| json!({
                "input": u.input_tokens,
                "output": u.output_tokens,
            })),
        },
    });
    SseEvent::default().event("done").data(data.to_string())
}
