//! `llm` node executor (dev-docs/workflow/llm-node.md §4).
//!
//! Template rendering via `expr::resolve_text` (C3.1), provider call through
//! the shared `[ai]` runtime (or an injected test provider), structured output
//! via prompt-constrained JSON + one corrective regeneration, error mapping
//! 4xx→BadRequest so the engine's blind retry fails fast.

use std::sync::Arc;
use std::time::Instant;

use raisfast_agent::ChatMessage;
use raisfast_agent::provider::ChatRequest;
use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};
use crate::llm::service::LlmRouter;

use super::engine::{ExecOutcome, Pool};
use super::expr;
use super::graph::GraphNode;
use super::nodes::LlmConfig;

/// LLM runtime an executor resolves per call: injected mock router (tests) or
/// the process-wide router handle (production). 模型访问唯一入口 = llm 底座
/// （design §10.2）——节点只带租户与（可选）触发用户，其余全在内核。
#[derive(Clone)]
pub struct LlmRuntime {
    pub router: Arc<LlmRouter>,
    pub tenant: String,
    /// 触发用户（计费归因/日限额）；cron/system = None。
    pub caller: Option<crate::types::snowflake_id::SnowflakeId>,
}

/// facade/内核已把错误分类为 AppError（4xx 确定性失败 fail-fast，
/// 429/5xx/transport → Internal 可重试）——引擎重试语义由内核承接。
fn map_facade_error(e: AppError) -> AppError {
    e
}

fn render_prompt_text(text: &str, pool: &Pool) -> AppResult<String> {
    // Prompts are always text: a whole-string `{{#ref#}}` returning an object
    // is stringified instead of failing (C3.1 keeps typed values).
    match expr::resolve_text(text, pool)? {
        Value::String(s) => Ok(s),
        other => Ok(match &other {
            Value::String(s) => s.clone(),
            v => serde_json::to_string(v).unwrap_or_default(),
        }),
    }
}

fn to_chat_messages(cfg: &LlmConfig, pool: &Pool) -> AppResult<Vec<ChatMessage>> {
    let mut out = Vec::with_capacity(cfg.messages.len());
    for m in &cfg.messages {
        let text = render_prompt_text(&m.text, pool)?;
        let msg = match m.role.as_str() {
            "system" => ChatMessage::system(text),
            "assistant" => ChatMessage::assistant(Some(text), None),
            _ => ChatMessage::user(text),
        };
        out.push(msg);
    }
    Ok(out)
}

fn usage_json(u: Option<raisfast_agent::TokenUsage>) -> Value {
    let u = u.unwrap_or_default();
    let input = u.input_tokens.unwrap_or(0_u64);
    let output = u.output_tokens.unwrap_or(0_u64);
    json!({
        "prompt_tokens": input,
        "completion_tokens": output,
        "total_tokens": input + output,
    })
}

/// Parse model text as JSON: direct parse first, then strip a ```json fence
/// (llm-node.md §4 implementation memo).
fn parse_json_text(text: &str) -> Option<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
        return Some(v);
    }
    let t = text.trim();
    let inner = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```"))?;
    let inner = inner.trim_end().strip_suffix("```").unwrap_or(inner);
    serde_json::from_str::<Value>(inner.trim()).ok()
}

/// Execute the `llm` node against the variable pool.
///
/// # Errors
/// `BadRequest` on missing template refs, disabled `[ai]`, or a non-retryable
/// provider error; `Internal` on transient provider/timeout failures.
pub async fn run_llm(
    runtime: &LlmRuntime,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: LlmConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("llm config: {e}")))?;
    // 模型解析链在内核：显式指定 → 租户默认（llm.default_chat_model）→ 400。
    let model = cfg.model.clone();
    let call = runtime
        .router
        .call(&runtime.tenant, crate::llm::models::log::LogSource::Flow);

    let messages = to_chat_messages(&cfg, pool)?;
    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(60_000) as u64;
    let started = Instant::now();

    let mut call_messages = messages.clone();
    let mut final_text: Option<String> = None;
    let mut final_usage: Option<raisfast_agent::TokenUsage> = None;
    let mut structured: Option<Value> = None;

    // Up to 2 passes when json_schema is set: initial + one corrective
    // regeneration feeding the parse error back (llm-node.md W3).
    let passes = if cfg.json_schema.is_some() { 2 } else { 1 };
    for pass in 0..passes {
        let request = ChatRequest {
            messages: &call_messages,
            tools: None,
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            stop: cfg.stop.clone(),
        };
        let response = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            call.clone().chat(model.as_deref(), &request),
        )
        .await
        .map_err(|_| AppError::Internal(anyhow::anyhow!("llm 超时 {timeout_ms}ms")))?
        .map_err(map_facade_error)?;
        if let Some(u) = response.usage {
            final_usage = Some(u);
        }
        let Some(text) = response.text else {
            return Err(AppError::Internal(anyhow::anyhow!("llm 响应无文本内容")));
        };
        if let Some(schema) = &cfg.json_schema {
            let Some(parsed) = parse_json_text(&text) else {
                if pass + 1 < passes {
                    call_messages.push(ChatMessage::assistant(Some(text.clone()), None));
                    call_messages.push(ChatMessage::user(format!(
                        "你上一条回复不是合法 JSON（解析失败）。请严格只输出符合此 JSON Schema 的 JSON，不要任何解释或代码围栏：\n{schema}"
                    )));
                    continue;
                }
                return Err(AppError::Internal(anyhow::anyhow!(
                    "llm structured output 解析失败（含一次纠错重生成）"
                )));
            };
            if let Err(reason) = super::nodes::shallow_schema_check(&parsed, schema) {
                if pass + 1 < passes {
                    call_messages.push(ChatMessage::assistant(Some(text.clone()), None));
                    call_messages.push(ChatMessage::user(format!(
                        "你上一条回复不符合 JSON Schema（{reason}）。请严格只输出符合此 Schema 的 JSON，不要任何解释或代码围栏：\n{schema}"
                    )));
                    continue;
                }
                return Err(AppError::Internal(anyhow::anyhow!(
                    "llm structured output 校验失败: {reason}"
                )));
            }
            structured = Some(parsed);
        }
        final_text = Some(text);
        break;
    }

    let latency_ms = started.elapsed().as_millis() as i64;
    let mut out = Map::new();
    out.insert("text".into(), json!(final_text.unwrap_or_default()));
    if let Some(s) = structured {
        out.insert("structured".into(), s);
    }
    out.insert("usage".into(), usage_json(final_usage));
    out.insert("latency_ms".into(), json!(latency_ms));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: Some(usage_json(final_usage)),
        latency_ms: Some(latency_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;
    use crate::types::snowflake_id::SnowflakeId;
    use async_trait::async_trait;
    use raisfast_agent::TokenUsage;
    use raisfast_agent::provider::{ChatResponse, ModelProvider, ProviderError};
    use std::sync::Mutex;

    async fn seed_option(pool: &crate::db::Pool, key: &str, value: serde_json::Value) {
        use crate::db::Driver;
        use crate::db::driver::DbDriver;
        let ph = |i: usize| Driver::ph(i);
        let sql = format!(
            "INSERT INTO options (id, option_key, value, type, group_name, label, autoload, sort_order, updated_at) \
             VALUES ({}, {}, {}, 'text', 'llm', 'llm', 1, 0, {})",
            ph(1),
            ph(2),
            ph(3),
            ph(4)
        );
        sqlx::query(crate::db::safe_sql(&sql))
            .bind(crate::utils::id::new_id())
            .bind(key)
            .bind(value.to_string())
            .bind(crate::utils::tz::now_utc())
            .execute(pool)
            .await
            .unwrap();
    }

    /// Mock 渠道行（chat 类型，$1/$1，polling 单 key）。
    fn mock_channel(id: i64, models: &[&str]) -> crate::llm::models::channel::LlmChannel {
        crate::llm::models::channel::LlmChannel {
            id: SnowflakeId(id),
            tenant_id: Some("default".to_owned()),
            name: "mock".to_owned(),
            provider: "openai".to_owned(),
            base_url: "http://mock.test/v1".to_owned(),
            api_keys: serde_json::to_value(vec![crate::llm::models::channel::LlmKeyEntry {
                key: "plain".to_owned(),
                status: crate::llm::models::channel::LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: None,
            }])
            .unwrap(),
            key_mode: crate::llm::models::channel::LlmKeyMode::Polling,
            status: crate::llm::models::channel::LlmChannelStatus::Enabled,
            models: models.join(","),
            model_mapping: None,
            priority: 0,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            used_quota: 0,
            cost_mode: crate::llm::models::channel::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
            test_time: None,
            response_time: None,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    /// Mock 渠道 cache：单渠道服务 `models` 列表（chat 类型，$1/$1）。
    fn cache_for(models: &[&str]) -> crate::llm::cache::ChannelCache {
        let row = mock_channel(1, models);
        let mut cache = crate::llm::cache::ChannelCache::default();
        let cached = crate::llm::cache::ChannelCache::from_row(&row);
        cache
            .channels
            .insert(cached.id, std::sync::Arc::new(cached));
        for m in models {
            cache.models.insert(
                ("default".to_owned(), (*m).to_owned()),
                std::sync::Arc::new(crate::llm::cache::ModelInfo {
                    name: (*m).to_owned(),
                    model_type: crate::llm::models::model::LlmModelType::Chat,
                    pricing: crate::llm::cache::Pricing {
                        price_mode: crate::llm::models::model::LlmPriceMode::Token,
                        input_price: 1.0,
                        output_price: 1.0,
                        cache_read_price: None,
                        cache_write_price: None,
                        call_price: None,
                    },
                    params: None,
                }),
            );
        }
        cache.rebuild_routes();
        cache
    }

    /// Mock router：provider 注入到 (ch1, key0)。
    fn rt(provider: Arc<MockProvider>, models: &[&str]) -> LlmRuntime {
        let router = LlmRouter::from_cache_for_test(cache_for(models));
        router.providers.insert(
            (SnowflakeId(1), 0),
            provider as std::sync::Arc<dyn ModelProvider>,
        );
        LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        }
    }

    /// Scripted mock: pops queued responses, records every ChatRequest.
    struct MockProvider {
        responses: Mutex<std::collections::VecDeque<Result<ChatResponse, ProviderError>>>,
        seen: Mutex<Vec<Seen>>,
    }

    #[derive(Debug, Clone, PartialEq)]
    struct Seen {
        model: String,
        temperature: Option<f64>,
        max_tokens: Option<i64>,
        stop: Option<Vec<String>>,
        n_messages: usize,
    }

    impl MockProvider {
        fn new(responses: Vec<Result<ChatResponse, ProviderError>>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        async fn chat(
            &self,
            request: &ChatRequest<'_>,
            model: &str,
        ) -> Result<ChatResponse, ProviderError> {
            self.seen.lock().unwrap().push(Seen {
                model: model.to_string(),
                temperature: request.temperature,
                max_tokens: request.max_tokens,
                stop: request.stop.clone(),
                n_messages: request.messages.len(),
            });
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(ChatResponse::text_only("ok")))
        }
    }

    fn llm_node(config: Value) -> GraphNode {
        GraphNode {
            id: "n1".into(),
            data: NodeData {
                kind: "llm".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    fn pool_with(pairs: &[(&str, &str, Value)]) -> Pool {
        let mut pool = Pool::new();
        for (ns, name, v) in pairs {
            pool.entry((*ns).to_string())
                .or_default()
                .insert((*name).to_string(), v.clone());
        }
        pool
    }

    fn usage_ok() -> TokenUsage {
        TokenUsage {
            input_tokens: Some(812),
            output_tokens: Some(150),
            cache_read: None,
            cache_write: None,
        }
    }

    fn resp_with_usage(text: &str) -> Result<ChatResponse, ProviderError> {
        Ok(ChatResponse {
            text: Some(text.into()),
            tool_calls: Vec::new(),
            usage: Some(usage_ok()),
        })
    }

    #[tokio::test]
    async fn happy_path_output_shape_and_usage() {
        let mock = Arc::new(MockProvider::new(vec![resp_with_usage("答案是42")]));
        let node = llm_node(json!({
            "model": "m1",
            "messages": [{"role": "user", "text": "Q: {{#start.q#}}"}]
        }));
        let pool = pool_with(&[("start", "q", json!("1+1"))]);
        let out = run_llm(&rt(mock.clone(), &["m1"]), &node, &pool)
            .await
            .unwrap();
        assert_eq!(out.output["text"], "答案是42");
        assert_eq!(out.output["usage"]["total_tokens"], 962);
        assert_eq!(out.usage.as_ref().unwrap()["prompt_tokens"], 812);
        assert!(out.latency_ms.is_some());
        assert_eq!(mock.seen.lock().unwrap()[0].model, "m1");
    }

    #[tokio::test]
    async fn params_passthrough_and_default_model() -> AppResult<()> {
        // 默认模型解析链在内核（§10.2）：租户 options llm.default_chat_model。
        let pool = crate::test_pool!();
        seed_option(
            &pool,
            "llm.default_chat_model",
            serde_json::json!("default-model"),
        )
        .await;
        let mock = Arc::new(MockProvider::new(vec![resp_with_usage("ok")]));
        let router = LlmRouter::from_cache_for_test_with_pool(cache_for(&["default-model"]), pool);
        router.providers.insert(
            (SnowflakeId(1), 0),
            mock.clone() as std::sync::Arc<dyn ModelProvider>,
        );
        let rt = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = llm_node(json!({
            "messages": [{"role": "user", "text": "hi"}],
            "temperature": 0.2, "max_tokens": 99, "stop": ["\n"]
        }));
        run_llm(&rt, &node, &Pool::new()).await?;
        let seen = &mock.seen.lock().unwrap()[0];
        assert_eq!(seen.model, "default-model");
        assert_eq!(seen.max_tokens, Some(99));
        assert_eq!(seen.stop.as_deref(), Some(&["\n".to_string()][..]));
        Ok(())
    }

    #[tokio::test]
    async fn missing_template_ref_is_bad_request() {
        let mock = Arc::new(MockProvider::new(vec![resp_with_usage("x")]));
        let node = llm_node(json!({
            "model": "m1", "messages": [{"role": "user", "text": "{{#start.nope#}}"}]
        }));
        let err = run_llm(&rt(mock, &["m1"]), &node, &Pool::new())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[tokio::test]
    async fn auth_error_maps_to_bad_request_fail_fast() {
        let mock = Arc::new(MockProvider::new(vec![Err(ProviderError::Http {
            status: 401,
            body: "bad key".into(),
        })]));
        let node = llm_node(json!({
            "model": "m1", "messages": [{"role": "user", "text": "hi"}]
        }));
        let err = run_llm(&rt(mock, &["m1"]), &node, &Pool::new())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[tokio::test]
    async fn transport_error_fails_over_to_second_channel() {
        // 重试已收敛到内核（§10.1）：ch1 传输失败 → 自动换 ch2 成功。
        let p1 = Arc::new(MockProvider::new(vec![Err(ProviderError::Transport(
            "conn reset".into(),
        ))]));
        let p2 = Arc::new(MockProvider::new(vec![resp_with_usage("ok")]));
        let mut cache = cache_for(&["m1"]);
        let row2 = mock_channel(2, &["m1"]);
        let cached2 = crate::llm::cache::ChannelCache::from_row(&row2);
        cache
            .channels
            .insert(cached2.id, std::sync::Arc::new(cached2));
        cache.rebuild_routes();
        let router = LlmRouter::from_cache_for_test(cache);
        router
            .providers
            .insert((SnowflakeId(1), 0), p1 as std::sync::Arc<dyn ModelProvider>);
        router.providers.insert(
            (SnowflakeId(2), 0),
            p2.clone() as std::sync::Arc<dyn ModelProvider>,
        );
        let rt = LlmRuntime {
            router,
            tenant: "default".to_owned(),
            caller: None,
        };
        let node = llm_node(json!({
            "model": "m1",
            "messages": [{"role": "user", "text": "hi"}]
        }));
        let out = run_llm(&rt, &node, &Pool::new()).await.unwrap();
        assert_eq!(out.output["text"], "ok");
        assert_eq!(p2.seen.lock().unwrap()[0].model, "m1");
    }

    #[tokio::test]
    async fn fenced_json_parsed_without_regen() {
        let mock = Arc::new(MockProvider::new(vec![resp_with_usage(
            "```json\n{\"score\": 9}\n```",
        )]));
        let node = llm_node(json!({
            "model": "m1", "messages": [{"role": "user", "text": "质检"}],
            "json_schema": {"type":"object","properties":{"score":{"type":"number"}},"required":["score"]}
        }));
        let out = run_llm(&rt(mock.clone(), &["m1"]), &node, &Pool::new())
            .await
            .unwrap();
        assert_eq!(out.output["structured"]["score"], 9);
        assert_eq!(mock.seen.lock().unwrap().len(), 1, "一次成功不触发纠错");
    }

    #[tokio::test]
    async fn corrective_regen_recovers_from_garbage() {
        let mock = Arc::new(MockProvider::new(vec![
            resp_with_usage("抱歉，我无法输出 JSON"),
            resp_with_usage("{\"score\": 7}"),
        ]));
        let node = llm_node(json!({
            "model": "m1", "messages": [{"role": "user", "text": "质检"}],
            "json_schema": {"type":"object","properties":{"score":{"type":"number"}},"required":["score"]}
        }));
        let out = run_llm(&rt(mock.clone(), &["m1"]), &node, &Pool::new())
            .await
            .unwrap();
        assert_eq!(out.output["structured"]["score"], 7);
        assert_eq!(mock.seen.lock().unwrap().len(), 2);
        assert_eq!(
            mock.seen.lock().unwrap()[1].n_messages,
            3,
            "纠错轮 = 原始 + 模型回复 + 反馈指令"
        );
    }

    #[tokio::test]
    async fn schema_violation_twice_fails_node() {
        let mock = Arc::new(MockProvider::new(vec![
            resp_with_usage("{\"nope\": 1}"),
            resp_with_usage("{\"still_nope\": 2}"),
        ]));
        let node = llm_node(json!({
            "model": "m1", "messages": [{"role": "user", "text": "质检"}],
            "json_schema": {"type":"object","properties":{"score":{"type":"number"}},"required":["score"]}
        }));
        let err = run_llm(&rt(mock, &["m1"]), &node, &Pool::new())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("校验失败"), "{err}");
    }

    #[test]
    fn parse_json_text_variants() {
        assert_eq!(parse_json_text(" {\"a\":1} "), Some(json!({"a":1})));
        assert_eq!(
            parse_json_text("```json\n{\"a\":1}\n```"),
            Some(json!({"a":1}))
        );
        assert_eq!(parse_json_text("plain junk"), None);
    }

    #[test]
    fn shallow_schema_check_types_and_required() {
        let schema = json!({"type":"object","properties":{"score":{"type":"number"},"tag":{"type":"string"}},"required":["score"]});
        assert!(crate::flows::nodes::shallow_schema_check(&json!({"score": 1}), &schema).is_ok());
        assert!(crate::flows::nodes::shallow_schema_check(&json!({}), &schema).is_err());
        assert!(crate::flows::nodes::shallow_schema_check(&json!({"score":"x"}), &schema).is_err());
    }
}
