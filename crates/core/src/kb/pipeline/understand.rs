//! S1 query understanding — rewrite + keyword extraction via LLM
//! [抄WK:chat_pipeline/query_understand.go + rewrite.yaml 语义].
//!
//! Degrades gracefully: without a provider, a malformed LLM reply, or any
//! error, the raw question is used verbatim (retrieval must never hard-fail
//! on an optional understanding step).

use raisfast_agent::ChatRequest;
use raisfast_agent::messages::{ChatMessage, ChatRole};

use crate::kb::service::KbDeps;

/// Understood query: the (possibly rewritten) search text plus keywords.
#[derive(Debug, Clone)]
pub struct UnderstoodQuery {
    pub text: String,
    pub keywords: Vec<String>,
}

impl UnderstoodQuery {
    pub fn raw(question: &str) -> Self {
        Self {
            text: question.to_string(),
            keywords: Vec::new(),
        }
    }
}

/// LLM prompt: strict JSON reply `{ "query": "...", "keywords": [...] }`.
const PROMPT: &str = "你是搜索查询优化器。将用户问题改写为更适合知识库检索的查询，\
并抽取 2-5 个关键词。只输出 JSON：{\"query\": \"...\", \"keywords\": [\"...\"]}。\
若问题已经足够清晰，query 原样返回。";

pub async fn run(deps: &KbDeps, question: &str) -> UnderstoodQuery {
    let Some(provider) = deps.provider.as_ref() else {
        return UnderstoodQuery::raw(question);
    };
    let model = deps.config.ai.model.as_deref().unwrap_or_default();
    if model.is_empty() {
        return UnderstoodQuery::raw(question);
    }
    let request = ChatRequest {
        messages: &[
            ChatMessage {
                role: ChatRole::System,
                content: Some(PROMPT.to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: ChatRole::User,
                content: Some(question.to_string()),
                tool_calls: None,
                tool_call_id: None,
            },
        ],
        tools: None,
        temperature: Some(0.0),
        max_tokens: Some(200),
        stop: None,
    };
    let Ok(response) = provider.chat(&request, model).await else {
        return UnderstoodQuery::raw(question);
    };
    let Some(text) = response.text else {
        return UnderstoodQuery::raw(question);
    };
    parse(&text).unwrap_or_else(|| UnderstoodQuery::raw(question))
}

/// Parse the JSON reply, tolerating code fences around the object.
fn parse(text: &str) -> Option<UnderstoodQuery> {
    let trimmed = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```");
    let trimmed = trimmed.trim_end_matches("```").trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    let value: serde_json::Value = serde_json::from_str(&trimmed[start..=end]).ok()?;
    let query = value.get("query")?.as_str()?.trim().to_string();
    if query.is_empty() {
        return None;
    }
    let keywords = value
        .get("keywords")
        .and_then(|k| k.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(UnderstoodQuery {
        text: query,
        keywords,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let q = parse(r#"{"query":"安装 raisfast 需要什么","keywords":["安装","raisfast"]}"#)
            .expect("must parse");
        assert_eq!(q.text, "安装 raisfast 需要什么");
        assert_eq!(q.keywords.len(), 2);
    }

    #[test]
    fn parses_fenced_json() {
        let q =
            parse("```json\n{\"query\":\"数据库配置\",\"keywords\":[]}\n```").expect("must parse");
        assert_eq!(q.text, "数据库配置");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("I cannot help with that").is_none());
        assert!(parse("{\"keywords\":[\"x\"]}").is_none());
    }
}
