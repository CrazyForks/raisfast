//! Builtin worker handler: run one AI agent turn from a job (`agent.run`).
//!
//! Enables cron/flow/plugin-triggered, headless agent turns. The turn runs
//! through `AgentService::run_turn` (same two-phase persistence / turn:meta as
//! HTTP turns); the actor is taken from the job payload (see architecture §5.8).
//! Results are broadcast as `Event::Custom{source:"ai", event_type:"ai.turn.*"}`.

use std::sync::Arc;

use serde_json::{Value, json};

use crate::agent::service as ai_service;
use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::event::EventEmitter;
use crate::types::snowflake_id::SnowflakeId;
use crate::worker::handler::HandlerMeta;
use crate::worker::{Job, JobHandler};

pub struct AgentRunHandler {
    pool: Pool,
    config: Arc<AppConfig>,
    emitter: EventEmitter,
}

impl AgentRunHandler {
    #[must_use]
    pub fn new(pool: Pool, config: Arc<AppConfig>, emitter: EventEmitter) -> Self {
        Self {
            pool,
            config,
            emitter,
        }
    }
}

pub static META: HandlerMeta = HandlerMeta {
    id: "agent_run",
    display_name: "运行 AI Agent 回合",
    description: "对一个 AI agent/会话执行一次回合（payload 指定 agent_id 与 content，可选 session_id/tenant）",
    category: "AI",
    params_schema: Some(
        r#"{
  "type": "object",
  "properties": {
    "agent_id": { "type": ["integer", "string"], "description": "ai_agents.id" },
    "session_id": { "type": ["integer", "string"], "description": "可选：复用已有会话" },
    "tenant": { "type": "string", "description": "默认 default" },
    "content": { "type": "string", "description": "用户消息" }
  },
  "required": ["agent_id", "content"]
}"#,
    ),
    icon: Some("Bot"),
};

impl AgentRunHandler {
    fn id_field(payload: &Value, key: &str) -> AppResult<SnowflakeId> {
        let raw = payload.get(key).ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("agent_run: missing '{key}' in payload"))
        })?;
        let id = raw
            .as_str()
            .map(str::to_string)
            .or_else(|| raw.as_i64().map(|n| n.to_string()))
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("agent_run: invalid '{key}'")))?;
        crate::types::snowflake_id::parse_id(&id)
    }

    fn broadcast(&self, event_type: &str, data: Value) {
        self.emitter.emit(crate::event::Event::Custom {
            source: "ai".to_string(),
            event_type: event_type.to_string(),
            data,
        });
    }
}

#[async_trait::async_trait]
impl JobHandler for AgentRunHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let agent_id = Self::id_field(payload, "agent_id")?;
        let content = payload
            .get("content")
            .and_then(Value::as_str)
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("agent_run: missing non-empty 'content'"))
            })?;
        let tenant = payload
            .get("tenant")
            .and_then(Value::as_str)
            .unwrap_or(crate::constants::DEFAULT_TENANT);

        // Resolve agent + session (session is optional; created when absent).
        let agent =
            crate::agent::models::ai_agent::find_agent_by_id(&self.pool, agent_id, Some(tenant))
                .await?;
        let session_id = match payload.get("session_id") {
            Some(v) => {
                let sid_str = v
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                    .ok_or_else(|| {
                        AppError::Internal(anyhow::anyhow!("agent_run: invalid 'session_id'"))
                    })?;
                let sid = crate::types::snowflake_id::parse_id(&sid_str)?;
                let session = crate::agent::models::ai_session::find_session_by_id(
                    &self.pool,
                    sid,
                    Some(tenant),
                )
                .await?;
                if session.agent_id != agent_id {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "agent_run: session {sid} does not belong to agent {agent_id}"
                    )));
                }
                session.id
            }
            None => {
                crate::agent::models::ai_session::create_session(
                    &self.pool,
                    Some(tenant),
                    agent_id,
                    agent.owner_id.unwrap_or(agent_id),
                    "scheduled",
                )
                .await?
                .id
            }
        };

        let result =
            ai_service::run_turn(&self.pool, &self.config.ai, &agent, session_id, content).await;
        match result {
            Ok(outcome) => {
                if self.config.ai.broadcast_events {
                    self.broadcast(
                        "ai.turn.done",
                        json!({
                            "agent_id": agent.id.0,
                            "session_id": session_id.0,
                            "text": outcome.text,
                            "iterations": outcome.iterations,
                            "tool_calls_made": outcome.tool_calls_made,
                        }),
                    );
                }
                Ok(())
            }
            Err(e) => {
                if self.config.ai.broadcast_events {
                    self.broadcast(
                        "ai.turn.error",
                        json!({
                            "agent_id": agent.id.0,
                            "session_id": session_id.0,
                            "message": e.to_string(),
                        }),
                    );
                }
                Err(e)
            }
        }
    }
}
