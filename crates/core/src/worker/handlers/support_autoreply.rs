//! `support.autoreply` — MVP-M1 LLM auto-reply pipeline (mvp-plan D4/D5).
//!
//! One job per inbound user message (enqueued by the route stage when the
//! channel declares `route_extra.jobs[].job_type = "support.autoreply"`).
//! The handler is pure orchestration: every business parameter lives in the
//! channel's `route_extra.autoreply` config.
//!
//! Flow: receipt envelope snapshot → contact merge by (channel, sender) →
//! open conversation (create if none) → attach the routed user message →
//! context window → `call_api_traced` LLM call → assistant message row +
//! `integration.message` SSE event. The runner flips the receipt's pending
//! `job:support.autoreply` step on completion (§10.7, zero code here).
//!
//! Failure policy (mvp-plan): LLM failure fails the job (declare
//! `max_attempts: 1` on the job config), sets the conversation to
//! `failure_status` (human takeover) and emits an `integration.alert`.

use serde_json::Value;

use crate::content_type::repository::{ContentQuery, FieldFilter, FilterOp, SaveContext};
use crate::errors::app_error::{AppError, AppResult};
use crate::worker::Job;
use crate::worker::handler::{HandlerMeta, JobHandler};

pub static META: HandlerMeta = HandlerMeta {
    id: "support.autoreply",
    display_name: "客服自动回复",
    description: "入站用户消息触发的 LLM 自动回复：会话归并 + 上下文窗口 + call_api 出站 + assistant 落库 + SSE。全部参数来自渠道 route_extra.autoreply 配置",
    category: "集成",
    params_schema: Some(
        r#"{"type":"object","description":"payload 由管道注入 {trace_id, channel_key}"}"#,
    ),
    icon: None,
};

/// Parsed `route_extra.autoreply` config.
struct AutoreplyConfig {
    client: String,
    op: String,
    context_window: i64,
    system_prompt: Option<String>,
    output_field: Option<String>,
    failure_status: String,
    conversation_table: String,
    contact_table: String,
    /// Request body style: `messages` (default, `{query, messages, system}` —
    /// Dify-ish) or `openai` (`{model, messages:[{role,content}...]}` —
    /// GLM/OpenAI-compatible chat completions).
    input_style: String,
    /// Model id for `openai` style bodies (`glm-4-flash`, …).
    model: Option<String>,
}

fn cfg_str(cfg: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    cfg.get(key).and_then(Value::as_str).map(str::to_string)
}

impl AutoreplyConfig {
    fn parse(channel_extra: &Value) -> AppResult<Self> {
        let cfg = channel_extra
            .get("autoreply")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                AppError::BadRequest(
                    "channel route_extra has no 'autoreply' config — add it to enable \
                     support.autoreply"
                        .into(),
                )
            })?;
        let client = cfg_str(cfg, "client").ok_or_else(|| {
            AppError::BadRequest("autoreply config requires 'client' (api-client key)".into())
        })?;
        Ok(Self {
            client,
            op: cfg_str(cfg, "op").unwrap_or_else(|| "chat".into()),
            context_window: cfg
                .get("context_window")
                .and_then(Value::as_i64)
                .unwrap_or(10)
                .clamp(1, 100),
            system_prompt: cfg_str(cfg, "system_prompt"),
            output_field: cfg_str(cfg, "output_field"),
            failure_status: cfg_str(cfg, "failure_status").unwrap_or_else(|| "pending".into()),
            conversation_table: cfg
                .get("tables")
                .and_then(|t| t.get("conversation"))
                .and_then(Value::as_str)
                .unwrap_or("sc_conversation")
                .to_string(),
            contact_table: cfg
                .get("tables")
                .and_then(|t| t.get("contact"))
                .and_then(Value::as_str)
                .unwrap_or("sc_contact")
                .to_string(),
            input_style: cfg_str(cfg, "input_style").unwrap_or_else(|| "messages".into()),
            model: cfg_str(cfg, "model"),
        })
    }
}

/// Query helper: equality filters, newest first, small page.
async fn find_rows(
    repo: &crate::content_type::repository::ContentRepository,
    ct: &crate::content_type::schema::ContentTypeSchema,
    filters: Vec<FieldFilter>,
    sort: &str,
) -> AppResult<Vec<Value>> {
    let query = ContentQuery {
        page: 1,
        page_size: 20,
        sort: Some(sort.into()),
        filters,
        search: None,
        fields: None,
        tenant_id: None,
        include: None,
        skip_total: true,
        rule_where: None,
        rule_params: Vec::new(),
        max_page_size: 100,
        include_private: true,
        meta_filters: Vec::new(),
    };
    repo.find(ct, query).await.map(|(rows, _)| rows)
}

pub struct SupportAutoreplyHandler;

fn id_of(row: &Value) -> Option<i64> {
    // CT rows carry ids as encoded base62 strings when ID_ENCODING is on —
    // plain `as_i64`/`parse` both fail there.
    crate::types::snowflake_id::parse_id_value(row.get("id")?)
}

fn body_of(row: &Value) -> String {
    row.get("body")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[async_trait::async_trait]
impl JobHandler for SupportAutoreplyHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        let Job::Custom { payload, .. } = job else {
            return Ok(());
        };
        let Some(plane) = crate::integration::shared() else {
            return Ok(());
        };
        let trace_id = payload
            .get("trace_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| AppError::BadRequest("support.autoreply: missing trace_id".into()))?;
        let channel_key = payload
            .get("channel_key")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest("support.autoreply: missing channel_key".into()))?;

        let pipeline = plane.pipeline();
        let repo = pipeline.repo();
        let registry = pipeline.registry();
        let tenant = crate::constants::DEFAULT_TENANT;

        // ── Channel + config ────────────────────────────────────────────
        let channel = plane.channels().get(tenant, channel_key).await?;
        let cfg = AutoreplyConfig::parse(
            channel
                .route_extra
                .as_ref()
                .ok_or_else(|| AppError::BadRequest("channel has no route_extra".into()))?,
        )?;

        let get_ct = |name: &str| -> AppResult<_> {
            registry
                .get(name)
                .or_else(|| registry.get(&name.replace('_', "-")))
                .ok_or_else(|| {
                    AppError::BadRequest(format!("autoreply: content type '{name}' not registered"))
                })
        };
        let ct_msg = get_ct(&channel.target_type)?;
        let ct_conv = get_ct(&cfg.conversation_table)?;
        let ct_contact = get_ct(&cfg.contact_table)?;

        // ── Receipt envelope snapshot (deterministic facts, §6.4) ──────
        let receipt = crate::integration::receipt::find_by_id(
            plane.pool(),
            crate::types::snowflake_id::SnowflakeId::new(trace_id),
        )
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("support.autoreply: receipt {trace_id} not found"))
        })?;
        let envelope = receipt.envelope.clone().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!(
                "support.autoreply: receipt {trace_id} has no envelope snapshot"
            ))
        })?;
        let sender = envelope
            .get("sender")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AppError::BadRequest(
                    "support.autoreply: envelope has no sender — add it to the channel mapping \
                     (\"sender\": \"$.from...\")"
                        .into(),
                )
            })?
            .to_string();
        let external_id = envelope
            .get("external_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let user_text = envelope
            .pointer("/payload/body")
            .map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .unwrap_or_default();

        let save_ctx = SaveContext {
            user_id: None,
            user_int_id: None,
            user_role: None,
            tenant_id: Some(tenant.to_string()),
        };

        // ── Contact merge by (channel, sender) ──────────────────────────
        let contacts = find_rows(
            repo,
            &ct_contact,
            vec![
                FieldFilter {
                    field: "channel".into(),
                    op: FilterOp::Eq,
                    value: Value::String(channel_key.to_string()),
                },
                FieldFilter {
                    field: "sender".into(),
                    op: FilterOp::Eq,
                    value: Value::String(sender.clone()),
                },
            ],
            "id desc",
        )
        .await?;
        let contact_id = match contacts.first().and_then(id_of) {
            Some(id) => crate::types::snowflake_id::SnowflakeId::new(id),
            None => {
                let created = repo
                    .create(
                        &ct_contact,
                        serde_json::json!({
                            "channel": channel_key,
                            "sender": sender,
                        }),
                        Some(tenant),
                        &save_ctx,
                    )
                    .await?;
                crate::types::snowflake_id::SnowflakeId::new(id_of(&created).ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!("contact create returned no id"))
                })?)
            }
        };

        // ── Open conversation for the contact (create if none) ──────────
        let conversations = find_rows(
            repo,
            &ct_conv,
            vec![
                FieldFilter {
                    field: "contact_id".into(),
                    op: FilterOp::Eq,
                    value: contact_id.0.into(),
                },
                FieldFilter {
                    field: "status".into(),
                    op: FilterOp::Eq,
                    value: Value::String("open".into()),
                },
            ],
            "id desc",
        )
        .await?;
        let (conversation_id, is_new_conversation) = match conversations.first().and_then(id_of) {
            Some(id) => (crate::types::snowflake_id::SnowflakeId::new(id), false),
            None => {
                let created = repo
                    .create(
                        &ct_conv,
                        serde_json::json!({
                            "contact_id": contact_id.0,
                            "status": "open",
                        }),
                        Some(tenant),
                        &save_ctx,
                    )
                    .await?;
                (
                    crate::types::snowflake_id::SnowflakeId::new(id_of(&created).ok_or_else(
                        || {
                            AppError::Internal(anyhow::anyhow!(
                                "conversation create returned no id"
                            ))
                        },
                    )?),
                    true,
                )
            }
        };

        // ── Attach the routed user message to the conversation ──────────
        // The route stage already wrote the user message row (channel
        // target_type); link it now. Fresh conversations on a NEW message:
        // link unconditionally; existing conversations only when the row is
        // still unlinked (retries must not clobber older links).
        if !external_id.is_empty() {
            let messages = find_rows(
                repo,
                &ct_msg,
                vec![FieldFilter {
                    field: "external_id".into(),
                    op: FilterOp::Eq,
                    value: Value::String(external_id.clone()),
                }],
                "id desc",
            )
            .await?;
            let needs_link = messages.first().is_some_and(|m| {
                is_new_conversation
                    || m.get("conversation_id")
                        .map(|v| v.is_null())
                        .unwrap_or(true)
            });
            if let (true, Some(msg_id)) = (needs_link, messages.first().and_then(id_of)) {
                repo.update(
                    &ct_msg,
                    crate::types::snowflake_id::SnowflakeId::new(msg_id),
                    serde_json::json!({"conversation_id": conversation_id.0}),
                    Some(tenant),
                    &save_ctx,
                )
                .await?;
            }
        }

        // ── Context window (recent N messages, chronological) ───────────
        let window_rows = find_rows(
            repo,
            &ct_msg,
            vec![FieldFilter {
                field: "conversation_id".into(),
                op: FilterOp::Eq,
                value: conversation_id.0.into(),
            }],
            "id desc",
        )
        .await?;
        let mut history: Vec<Value> = window_rows
            .into_iter()
            .take(cfg.context_window as usize)
            .rev()
            .map(|m| {
                serde_json::json!({
                    "role": m.get("role").and_then(Value::as_str).unwrap_or("user"),
                    "content": body_of(&m),
                })
            })
            .collect();

        // The user message may not be linked yet on the linking-failure path
        // (or mapping omitted external_id) — make sure it is in the window.
        if !user_text.is_empty()
            && !history
                .iter()
                .any(|m| m["content"].as_str() == Some(user_text.as_str()))
        {
            history.push(serde_json::json!({"role": "user", "content": user_text}));
        }
        if history.len() > cfg.context_window as usize {
            let skip = history.len() - cfg.context_window as usize;
            history.drain(..skip);
        }

        // ── LLM call via the egress plane (trace follows the receipt) ───
        let input = build_llm_input(
            &cfg.input_style,
            cfg.model.as_deref(),
            cfg.system_prompt.as_deref(),
            history,
            &user_text,
        );
        let llm = plane
            .call_api_traced(trace_id, cfg.client.clone(), cfg.op.clone(), input)
            .await;

        let reply_text = match llm {
            Ok(receipt) => extract_reply(&receipt.output, cfg.output_field.as_deref()),
            Err(err) => {
                // Human takeover: mark the conversation + alert, then fail the
                // job (max_attempts=1 → no queue retries, mvp-plan M1).
                let _ = repo
                    .update(
                        &ct_conv,
                        conversation_id,
                        serde_json::json!({"status": cfg.failure_status}),
                        Some(tenant),
                        &save_ctx,
                    )
                    .await;
                plane.emit_alert(
                    "integration.autoreply_failed",
                    serde_json::json!({
                        "trace_id": trace_id,
                        "channel": channel_key,
                        "conversation_id": conversation_id.0,
                        "error": err.to_string(),
                    }),
                );
                return Err(err);
            }
        };
        if reply_text.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "support.autoreply: LLM reply resolved to empty text (trace {trace_id})"
            )));
        }

        // ── Assistant message + SSE fan-out ─────────────────────────────
        let assistant = repo
            .create(
                &ct_msg,
                serde_json::json!({
                    "conversation_id": conversation_id.0,
                    "role": "assistant",
                    "body": reply_text,
                    "external_id": format!("reply-{trace_id}"),
                }),
                Some(tenant),
                &save_ctx,
            )
            .await?;
        let message_id = id_of(&assistant);

        plane.emit_event(
            "integration.message",
            serde_json::json!({
                "trace_id": trace_id,
                "channel": channel_key,
                "conversation_id": conversation_id.0,
                "message_id": message_id,
                "role": "assistant",
                "body": reply_text,
            }),
        );
        tracing::info!(
            trace_id,
            channel = channel_key,
            conversation_id = conversation_id.0,
            "support.autoreply delivered"
        );
        Ok(())
    }
}

/// Build the LLM request body for the configured style.
///
/// - `messages` (default): `{query, messages, system}` — chat-workflow APIs
/// - `openai`: `{model, messages: [system?, ...history]}` — GLM / OpenAI
///   compatible chat completions
fn build_llm_input(
    style: &str,
    model: Option<&str>,
    system: Option<&str>,
    history: Vec<Value>,
    query: &str,
) -> Value {
    match style {
        "openai" => {
            let mut messages = Vec::new();
            if let Some(sys) = system {
                messages.push(serde_json::json!({"role": "system", "content": sys}));
            }
            messages.extend(history);
            let mut body = serde_json::json!({"messages": messages});
            if let Some(model) = model {
                body["model"] = Value::String(model.to_string());
            }
            body
        }
        _ => {
            let mut input = serde_json::json!({"query": query, "messages": history});
            if let Some(system) = system {
                input["system"] = Value::String(system.to_string());
            }
            input
        }
    }
}

/// Pull the reply text out of the egress output: `output_field` dot-path
/// (`output.text`) or the whole output when it is a scalar.
fn extract_reply(output: &Value, output_field: Option<&str>) -> String {
    let v = match output_field {
        Some(path) => {
            let mut cur = output;
            for seg in path.split('.').filter(|s| !s.is_empty()) {
                cur = cur.get(seg).unwrap_or(&Value::Null);
            }
            cur
        }
        None => output,
    };
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_input_openai_style() {
        // The handler guarantees history ends with the latest user message.
        let history = vec![
            serde_json::json!({"role": "user", "content": "hi"}),
            serde_json::json!({"role": "assistant", "content": "hello"}),
            serde_json::json!({"role": "user", "content": "继续"}),
        ];
        let body = build_llm_input(
            "openai",
            Some("glm-4-flash"),
            Some("你是客服"),
            history,
            "继续",
        );
        assert_eq!(body["model"], "glm-4-flash");
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["content"], "hi");
        assert_eq!(msgs[2]["role"], "assistant");
        assert_eq!(msgs[3]["content"], "继续");
    }

    #[test]
    fn llm_input_default_messages_style() {
        let history = vec![serde_json::json!({"role": "user", "content": "hi"})];
        let body = build_llm_input("messages", None, Some("你是客服"), history, "hi");
        assert_eq!(body["query"], "hi");
        assert_eq!(body["system"], "你是客服");
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert!(body.get("model").is_none());
    }

    #[test]
    fn extract_reply_paths_and_fallbacks() {
        let out = serde_json::json!({"text": "hi", "nested": {"answer": "yo"}});
        assert_eq!(extract_reply(&out, Some("text")), "hi");
        assert_eq!(extract_reply(&out, Some("nested.answer")), "yo");
        assert_eq!(extract_reply(&out, Some("missing")), "");
        assert_eq!(extract_reply(&serde_json::json!("plain"), None), "plain");
    }
}
