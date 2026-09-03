//! Minimal native function-calling turn loop (MVP).
//!
//! One `run()` = one user turn: repeatedly call the model, execute any
//! requested tools (feeding results back), until the model answers without
//! tool calls or the iteration cap is reached. Conversation state lives in
//! the caller-owned `history`; the engine keeps no durable state (full design:
//! `dev-docs/agent/loop-engine.md`).

use std::sync::Arc;

use serde_json::Value;

use crate::messages::{ChatMessage, ChatRole, TokenUsage};
use crate::provider::{ChatRequest, ModelProvider, ProviderError};
use crate::tool::ToolRegistry;

#[derive(Debug, Clone, Copy)]
pub struct TurnConfig {
    /// Maximum model round-trips inside one turn.
    pub max_iterations: usize,
    pub temperature: Option<f64>,
}

impl Default for TurnConfig {
    fn default() -> Self {
        Self {
            max_iterations: 10,
            temperature: None,
        }
    }
}

/// Events surfaced to the caller (UI/tool trace). Not persisted by the engine.
#[derive(Debug, Clone)]
pub enum TurnEvent {
    /// Assistant text produced during an iteration (including the terminal one).
    Text { text: String },
    /// The model requested a tool.
    ToolCall { name: String, arguments: Value },
    /// A tool finished; `output` is what was fed back to the model.
    ToolResult { name: String, output: String },
}

#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
}

#[derive(Debug, Default)]
pub struct TurnOutcome {
    pub text: String,
    pub events: Vec<TurnEvent>,
    pub iterations: usize,
    pub tool_calls_made: usize,
    pub usage: Option<TokenUsage>,
}

/// A model + tool registry + config bound for repeated turns.
pub struct TurnEngine {
    provider: Arc<dyn ModelProvider>,
    model: String,
    tools: Arc<ToolRegistry>,
    cfg: TurnConfig,
}

impl TurnEngine {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        model: impl Into<String>,
        tools: Arc<ToolRegistry>,
        cfg: TurnConfig,
    ) -> Self {
        Self {
            provider,
            model: model.into(),
            tools,
            cfg,
        }
    }

    /// Run one turn: append `user`, loop until terminal or `max_iterations`.
    /// All messages produced are appended to `history` (caller persists).
    pub async fn run(
        &self,
        history: &mut Vec<ChatMessage>,
        system: Option<&str>,
        user: &str,
    ) -> Result<TurnOutcome, TurnError> {
        // Ensure a single leading system message exists.
        if let Some(system) = system
            && !matches!(history.first(), Some(m) if m.role == ChatRole::System)
        {
            history.insert(0, ChatMessage::system(system));
        }
        history.push(ChatMessage::user(user));

        let specs = self.tools.specs();
        let tools_arg = (!specs.is_empty()).then_some(specs.as_slice());

        let mut outcome = TurnOutcome::default();
        let mut narration: Option<String> = None;

        loop {
            if outcome.iterations >= self.cfg.max_iterations {
                if outcome.text.is_empty() {
                    outcome.text = narration
                        .take()
                        .unwrap_or_else(|| "已到最大迭代次数，尚未收敛".to_string());
                }
                break;
            }
            outcome.iterations += 1;

            let request = ChatRequest {
                messages: history.as_slice(),
                tools: tools_arg,
                temperature: self.cfg.temperature,
            };
            let resp = self.provider.chat(&request, &self.model).await?;

            if let Some(u) = resp.usage {
                match &mut outcome.usage {
                    Some(acc) => acc.accumulate(u),
                    None => outcome.usage = Some(u),
                }
            }

            // Assistant text (any iteration) is narration; terminal if no tools.
            let text = resp.text.clone().filter(|t| !t.trim().is_empty());
            if let Some(t) = &text {
                narration = Some(t.clone());
                outcome.events.push(TurnEvent::Text { text: t.clone() });
            }

            if !resp.has_tool_calls() {
                outcome.text = text
                    .clone()
                    .or(narration.clone())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                break;
            }

            // Keep the assistant message with its tool_calls for the provider,
            // then execute each requested tool sequentially and feed back.
            history.push(ChatMessage::assistant(
                text.clone(),
                Some(resp.tool_calls.clone()),
            ));
            for call in &resp.tool_calls {
                outcome.tool_calls_made += 1;
                let arguments = serde_json::from_str(&call.arguments)
                    .unwrap_or(Value::String(call.arguments.clone()));
                outcome.events.push(TurnEvent::ToolCall {
                    name: call.name.clone(),
                    arguments: arguments.clone(),
                });

                let output = match self.tools.get(&call.name) {
                    Some(tool) => match tool.execute(arguments).await {
                        Ok(o) => o,
                        Err(e) => format!("工具执行失败: {e}"),
                    },
                    None => format!("工具不存在: {}", call.name),
                };
                outcome.events.push(TurnEvent::ToolResult {
                    name: call.name.clone(),
                    output: output.clone(),
                });
                history.push(ChatMessage::tool(call.id.clone(), output));
            }
        }

        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{ChatMessage, ChatRole, ToolCall};
    use crate::provider::{ChatRequest, ChatResponse, ModelProvider, ProviderError};
    use crate::tool::Tool;
    use async_trait::async_trait;
    use serde_json::Value;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    fn num_params_schema() -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "a": { "type": "number" },
                "b": { "type": "number" }
            },
            "required": ["a", "b"]
        })
    }

    /// Scripted provider: each call pops the next canned response.
    struct ScriptedProvider {
        responses: Mutex<VecDeque<ChatResponse>>,
    }

    #[async_trait]
    impl ModelProvider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        async fn chat(
            &self,
            _request: &ChatRequest<'_>,
            _model: &str,
        ) -> Result<ChatResponse, ProviderError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| ProviderError::Config("test script exhausted".into()))
        }
    }

    fn text_response(text: &str) -> ChatResponse {
        ChatResponse {
            text: Some(text.to_string()),
            tool_calls: Vec::new(),
            usage: None,
        }
    }

    fn tool_calls_response(calls: Vec<ToolCall>) -> ChatResponse {
        ChatResponse {
            text: None,
            tool_calls: calls,
            usage: None,
        }
    }

    fn call(id: &str, name: &str, a: f64, b: f64) -> ToolCall {
        ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: serde_json::json!({ "a": a, "b": b }).to_string(),
        }
    }

    struct AddTool;

    #[async_trait]
    impl Tool for AddTool {
        fn name(&self) -> &str {
            "add"
        }
        fn description(&self) -> &str {
            "Add two numbers"
        }
        fn parameters_schema(&self) -> Value {
            num_params_schema()
        }
        async fn execute(&self, args: Value) -> crate::tool::ToolExecution {
            let a = args.get("a").and_then(Value::as_f64).ok_or("a required")?;
            let b = args.get("b").and_then(Value::as_f64).ok_or("b required")?;
            Ok(format!("{}", a + b))
        }
    }

    struct MulTool;

    #[async_trait]
    impl Tool for MulTool {
        fn name(&self) -> &str {
            "mul"
        }
        fn description(&self) -> &str {
            "Multiply two numbers"
        }
        fn parameters_schema(&self) -> Value {
            num_params_schema()
        }
        async fn execute(&self, args: Value) -> crate::tool::ToolExecution {
            let a = args.get("a").and_then(Value::as_f64).ok_or("a required")?;
            let b = args.get("b").and_then(Value::as_f64).ok_or("b required")?;
            Ok(format!("{}", a * b))
        }
    }

    fn engine_with(tools: Vec<Arc<dyn Tool>>, provider: Arc<dyn ModelProvider>) -> TurnEngine {
        let mut reg = ToolRegistry::new();
        for t in tools {
            reg.push(t);
        }
        TurnEngine::new(provider, "test-model", Arc::new(reg), TurnConfig::default())
    }

    #[tokio::test]
    async fn single_tool_chain_feeds_result_and_terminates() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                tool_calls_response(vec![call("c1", "add", 1.0, 2.0)]),
                text_response("结果是 3"),
            ])),
        });
        let engine = engine_with(vec![Arc::new(AddTool)], provider);

        let mut history = Vec::new();
        let outcome = engine
            .run(&mut history, Some("你是助手"), "1+2?")
            .await
            .unwrap();

        assert_eq!(outcome.iterations, 2);
        assert_eq!(outcome.tool_calls_made, 1);
        assert!(outcome.text.contains('3'));
        // system only once, at the front
        assert!(matches!(history.first(), Some(m) if m.role == ChatRole::System));
        // assistant(tool_calls) then tool result are in history
        assert!(
            history
                .iter()
                .any(|m| matches!(&m.role, ChatRole::Assistant))
        );
        let tool_msgs: Vec<&ChatMessage> = history
            .iter()
            .filter(|m| m.role == ChatRole::Tool)
            .collect();
        assert_eq!(tool_msgs.len(), 1);
        assert_eq!(tool_msgs[0].content.as_deref(), Some("3"));
        assert_eq!(tool_msgs[0].tool_call_id.as_deref(), Some("c1"));
    }

    #[tokio::test]
    async fn multiple_tool_calls_in_one_turn_all_execute() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                tool_calls_response(vec![
                    call("c1", "add", 2.0, 3.0),
                    call("c2", "mul", 4.0, 5.0),
                ]),
                text_response("完成"),
            ])),
        });
        let engine = engine_with(vec![Arc::new(AddTool), Arc::new(MulTool)], provider);

        let mut history = Vec::new();
        let outcome = engine.run(&mut history, None, "一起算").await.unwrap();

        assert_eq!(outcome.tool_calls_made, 2);
        let results: Vec<&String> = outcome
            .events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::ToolResult { name, output } if name == "add" => Some(output),
                _ => None,
            })
            .collect();
        assert_eq!(results, vec!["5"]);
        let mul_out = outcome
            .events
            .iter()
            .any(|e| matches!(e, TurnEvent::ToolResult { name, output } if name == "mul" && output == "20"));
        assert!(mul_out, "mul(4,5)=20 must be fed back");
        assert_eq!(
            history.iter().filter(|m| m.role == ChatRole::Tool).count(),
            2
        );
    }

    #[tokio::test]
    async fn unknown_tool_soft_fails_and_loop_continues() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                tool_calls_response(vec![ToolCall {
                    id: "c1".into(),
                    name: "nope".into(),
                    arguments: "{}".into(),
                }]),
                text_response("继续"),
            ])),
        });
        let engine = engine_with(vec![Arc::new(AddTool)], provider);

        let mut history = Vec::new();
        let outcome = engine
            .run(&mut history, None, "用不存在的工具")
            .await
            .unwrap();

        assert_eq!(outcome.iterations, 2);
        assert_eq!(outcome.text, "继续");
        assert!(outcome.events.iter().any(|e| matches!(
            e,
            TurnEvent::ToolResult { name, output } if name == "nope" && output.contains("工具不存在")
        )));
    }

    #[tokio::test]
    async fn iteration_cap_stops_without_infinite_loop() {
        let provider = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::from([
                tool_calls_response(vec![call("c1", "add", 1.0, 1.0)]),
                tool_calls_response(vec![call("c2", "add", 1.0, 1.0)]),
            ])),
        });
        let mut reg = ToolRegistry::new();
        reg.register(AddTool);
        let engine = TurnEngine::new(
            provider,
            "test-model",
            Arc::new(reg),
            TurnConfig {
                max_iterations: 2,
                ..Default::default()
            },
        );
        let mut history = Vec::new();
        let outcome = engine.run(&mut history, None, "别停").await.unwrap();
        assert_eq!(outcome.iterations, 2);
        assert_eq!(outcome.tool_calls_made, 2);
        assert!(!outcome.text.is_empty(), "cap path returns a fallback text");
    }
}
