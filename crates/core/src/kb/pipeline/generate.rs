//! S9 generation + S10 references (+ the S11 gate lives in `finish_answer*`)
//! [抄WK:prompt_templates/system_prompt.yaml + chat_pipeline/references.go
//! + fallback.yaml 语义].

use raisfast_agent::messages::{ChatMessage, ChatRole};
use raisfast_agent::{ChatRequest, StreamEvent};

use crate::errors::app_error::{AppError, AppResult};
use crate::kb::pipeline::ContextUnit;
use crate::kb::service::KbDeps;
use crate::utils::prompt_file::prompt_file;

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
            // Answer strictly from the given knowledge, cite with [n]
            // (P2 anti-fabrication; markers map to the S10 list).
            content: Some(prompt_file!("src/kb/prompts/generate_system.md")),
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
    let messages = messages_for(prompt_units, question);
    let request = chat_request(&messages);

    // 唯一入口 llm 底座（§10.2）：模型解析（租户默认）+ 路由/日志/计费在内核。
    let full = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let sink = full.clone();
    let deltas = std::sync::Mutex::new(on_delta);
    deps.router
        .call(tenant, crate::llm::models::log::LogSource::Kb)
        .chat_stream(None, &request, &mut |ev: StreamEvent| {
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
    Ok(full.lock().map(|f| f.clone()).unwrap_or_default())
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
