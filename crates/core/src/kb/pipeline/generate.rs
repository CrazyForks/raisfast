//! S9 generation + S10 references (+ the S11 gate lives in `finish_answer*`)
//! [抄WK:prompt_templates/system_prompt.yaml + chat_pipeline/references.go
//! + fallback.yaml 语义].

use raisfast_agent::messages::{ChatMessage, ChatRole};
use raisfast_agent::{ChatRequest, StreamEvent};

use crate::errors::app_error::{AppError, AppResult};
use crate::kb::pipeline::ContextUnit;
use crate::kb::service::KbDeps;

/// System prompt: answer strictly from the given knowledge, cite with [n]
/// (P2 anti-fabrication; citation markers map to the S10 list).
const SYSTEM_PROMPT: &str = "你是知识库问答助手。仅依据下面提供的知识内容回答问题；\
每条知识以 [n] 编号，答案中引用对应知识时使用 [n] 标注。\
如果给定知识不足以回答，明确说明知识库未覆盖，不要编造。";

fn build_messages(prompt_units: &[ContextUnit], question: &str) -> Vec<ChatMessage> {
    let mut knowledge = String::new();
    for (i, u) in prompt_units.iter().enumerate() {
        knowledge.push_str(&format!(
            "[{}]（{}）{}\n{}\n\n",
            i + 1,
            u.kind,
            u.title,
            u.content
        ));
    }
    vec![
        ChatMessage {
            role: ChatRole::System,
            content: Some(SYSTEM_PROMPT.to_string()),
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: ChatRole::User,
            content: Some(format!("知识内容：\n{knowledge}\n问题：{question}")),
            tool_calls: None,
            tool_call_id: None,
        },
    ]
}

/// Build the message list; callers keep it alive for the request borrow.
fn messages_for(prompt_units: &[ContextUnit], question: &str) -> Vec<ChatMessage> {
    build_messages(prompt_units, question)
}

fn chat_request<'a>(messages: &'a [ChatMessage]) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools: None,
        temperature: Some(0.2),
        max_tokens: None,
        stop: None,
    }
}

/// S9 non-streaming generation（底座 facade 优先，§10.2）。
pub async fn generate_answer(
    deps: &KbDeps,
    tenant: &str,
    prompt_units: &[ContextUnit],
    question: &str,
) -> AppResult<String> {
    let messages = messages_for(prompt_units, question);
    let request = chat_request(&messages);
    crate::kb::service::kb_chat(deps, tenant, &request)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("kb generate: {e}")))
}

/// S9 streaming generation: forward text deltas to `on_delta`
/// （底座 facade 优先，未装配/未注册时回退 env provider）。
pub async fn generate_answer_streaming(
    deps: &KbDeps,
    tenant: &str,
    prompt_units: &[ContextUnit],
    question: &str,
    on_delta: &mut (dyn FnMut(&str) + Send),
) -> AppResult<String> {
    let model = deps.config.ai.model.clone().unwrap_or_default();
    let messages = messages_for(prompt_units, question);
    let request = chat_request(&messages);

    if let Some(router) = crate::agent::service::router_handle() {
        let effective = (!model.is_empty()).then_some(model.as_str());
        let routable = match effective {
            None => true,
            Some(m) => router.model_info(tenant, m).is_some(),
        };
        if routable {
            let full = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
            let sink = full.clone();
            let deltas = std::sync::Mutex::new(on_delta);
            router
                .call(tenant, crate::llm::models::log::LogSource::Kb)
                .chat_stream(effective, &request, &mut |ev: StreamEvent| {
                    if let StreamEvent::TextDelta { delta } = ev {
                        if let Ok(mut f) = sink.lock() {
                            f.push_str(&delta);
                        }
                        if let Ok(mut cb) = deltas.lock() {
                            cb(&delta);
                        }
                    }
                })
                .await
                .map_err(|e| AppError::ServiceUnavailable(format!("kb generate: {e}")))?;
            return Ok(full.lock().map(|f| f.clone()).unwrap_or_default());
        }
    }

    let provider = deps
        .provider
        .as_deref()
        .ok_or_else(|| AppError::ServiceUnavailable("kb chat provider unavailable".into()))?;
    let mut full = String::new();
    let mut on_event = |ev: StreamEvent| {
        if let StreamEvent::TextDelta { delta } = ev {
            full.push_str(&delta);
            on_delta(&delta);
        }
    };
    provider
        .chat_stream(&request, &model, &mut on_event)
        .await
        .map_err(|e| AppError::ServiceUnavailable(format!("kb generate: {e}")))?;
    Ok(full)
}

/// S10: citation list over the units actually fed to the model (numbering
/// matches the [n] markers in the knowledge block).
pub fn build_references(prompt_units: &[ContextUnit]) -> Vec<crate::kb::pipeline::Reference> {
    prompt_units
        .iter()
        .enumerate()
        .map(|(i, u)| crate::kb::pipeline::Reference {
            n: i + 1,
            unit_id: u.unit_id,
            kind: u.kind.clone(),
            title: u.title.clone(),
            snippet: snippet(&u.content, 120),
            score: u.score,
        })
        .collect()
}

fn snippet(text: &str, max_chars: usize) -> String {
    let taken: String = text.chars().take(max_chars).collect();
    if taken.chars().count() < text.chars().count() {
        format!("{taken}…")
    } else {
        taken
    }
}
