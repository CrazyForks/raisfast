//! LLM relay Anthropic 渠道 HTTP 集成测试 — wiremock 模拟 Anthropic 原生上游。
//!
//! 覆盖 design §8.2 P4 的两条面：
//! 1. outbound — `provider: "anthropic"` 渠道承接 OpenAI 格式客户端
//!    （/v1/chat/completions → /v1/messages 双向转换，流式 + 非流式）；
//! 2. inbound — `/v1/messages` 原生门面（openai 兼容渠道双向转换 /
//!    anthropic 渠道近透传），同一把 sk- key、同一套计费。
//!
//! 非 chat 模态（embeddings 等）对 anthropic 渠道应短路 400。

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use raisfast::llm::cache::ChannelCache;
use raisfast::llm::models::channel::{
    LlmChannel, LlmChannelStatus, LlmKeyEntry, LlmKeyMode, LlmKeyStatus,
};
use raisfast::llm::models::log::LogFilters;
use raisfast::llm::models::token::LlmToken;
use raisfast::llm::service::LlmRouter;
use raisfast::types::quota::Quota;
use raisfast::types::snowflake_id::SnowflakeId;

// ── helpers（与 llm_relay.rs 同构，provider 可选 anthropic）─────

fn chan(
    id: i64,
    provider: &str,
    base_url: &str,
    upstream_key: &str,
    models: &str,
    mapping: Option<Value>,
) -> LlmChannel {
    LlmChannel {
        id: SnowflakeId(id),
        tenant_id: Some("default".to_owned()),
        name: format!("mock-ch-{id}"),
        provider: provider.to_owned(),
        base_url: base_url.to_owned(),
        api_keys: serde_json::to_value(vec![LlmKeyEntry {
            key: upstream_key.to_owned(),
            status: LlmKeyStatus::Active,
            disabled_reason: None,
            disabled_at: None,
            max_concurrency: None,
        }])
        .unwrap(),
        key_mode: LlmKeyMode::Polling,
        status: LlmChannelStatus::Enabled,
        models: models.to_owned(),
        model_mapping: mapping,
        priority: 0,
        weight: 0,
        channel_groups: "default".to_owned(),
        auto_ban: true,
        param_override: None,
        header_override: None,
        config: None,
        used_quota: 0,
        cost_mode: raisfast::llm::models::channel::LlmCostMode::Usage,
        cost_discount: 1.0,
        monthly_cost: None,
        test_model: None,
        test_time: None,
        response_time: None,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    }
}

async fn make_llm_token(pool: &raisfast::db::Pool, remain: i64) -> (String, LlmToken) {
    let username = format!("llm-claude-{}", raisfast::utils::id::new_id());
    let cmd = raisfast::commands::CreateUserCmd::new(
        username,
        raisfast::models::user::RegisteredVia::Email,
    );
    let u = raisfast::models::user::create(pool, &cmd, None)
        .await
        .unwrap();
    let plain = raisfast::llm::relay::generate_sk();
    let t = raisfast::llm::models::token::create_token(
        pool,
        None,
        u.id,
        "claude-relay-test",
        &plain,
        Quota(remain),
        false,
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    (plain, t)
}

fn relay_router(state: &raisfast::AppState, channels: Vec<LlmChannel>) -> axum::Router {
    let router = LlmRouter::from_cache_for_test(ChannelCache::build(channels, vec![]));
    let mut s = state.clone();
    s.llm_router = router;
    axum::Router::new()
        .merge(raisfast::llm::relay::routes())
        .with_state(s)
}

fn chat_req(sk: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn messages_req(sk: &str, body: Value) -> Request<Body> {
    // Anthropic SDK 风格：x-api-key 携带同一把 sk- key。
    Request::builder()
        .method("POST")
        .uri("/v1/messages")
        .header(header::CONTENT_TYPE, "application/json")
        .header("x-api-key", sk)
        .header("anthropic-version", "2023-06-01")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

fn chat_body(model: &str, stream: bool) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "max_tokens": 1000,
        "messages": [{ "role": "user", "content": "Say hello" }]
    })
}

fn messages_body(model: &str, stream: bool) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "max_tokens": 1000,
        "system": "be nice",
        "messages": [{ "role": "user", "content": "Say hello" }]
    })
}

async fn token_row(pool: &raisfast::db::Pool, id: SnowflakeId) -> LlmToken {
    raisfast::llm::models::token::find_by_id(pool, id, None)
        .await
        .unwrap()
        .unwrap()
}

async fn wait_token<F>(pool: &raisfast::db::Pool, id: SnowflakeId, pred: F) -> LlmToken
where
    F: Fn(&LlmToken) -> bool,
{
    for _ in 0..200 {
        let row = token_row(pool, id).await;
        if pred(&row) {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    token_row(pool, id).await
}

async fn wait_logs_of(
    pool: &raisfast::db::Pool,
    token: &LlmToken,
) -> Vec<raisfast::llm::models::log::LlmLog> {
    for _ in 0..200 {
        let (items, total) = raisfast::llm::models::log::query_paged(
            pool,
            None,
            &LogFilters {
                token_id: Some(token.id.to_string()),
                ..Default::default()
            },
            1,
            10,
        )
        .await
        .unwrap();
        if total == 1 && !items.is_empty() {
            return items;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("log row never appeared for token {}", token.id);
}

/// gpt-4o 内置定价（$2.5/$10 per 1M）：quota = ceil((p×2.5 + c×10))。
fn gpt4o_quota(prompt: i64, completion: i64) -> Quota {
    let usd = (prompt as f64 * 2.5 + completion as f64 * 10.0) / 1_000_000.0;
    Quota::from_usd_ceil(usd)
}

/// Anthropic 原生 message 响应（含 cache 拆分字段）。
fn anthropic_message(prompt_in: i64, cache_read: i64, completion: i64) -> Value {
    json!({
        "id": "msg_mock",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{ "type": "text", "text": "Hello!" }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": prompt_in,
            "cache_read_input_tokens": cache_read,
            "cache_creation_input_tokens": 0,
            "output_tokens": completion
        }
    })
}

const CLAUDE_SSE: &str = concat!(
    "event: message_start\n",
    r#"data: {"type":"message_start","message":{"id":"msg_s1","model":"claude-sonnet-4-5","role":"assistant","content":[],"usage":{"input_tokens":80}}}"#,
    "\n\n",
    "event: content_block_start\n",
    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}"#,
    "\n\n",
    "event: content_block_delta\n",
    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}"#,
    "\n\n",
    "event: content_block_stop\n",
    r#"data: {"type":"content_block_stop","index":0}"#,
    "\n\n",
    "event: message_delta\n",
    r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":20}}"#,
    "\n\n",
    "event: message_stop\n",
    r#"data: {"type":"message_stop"}"#,
    "\n\n",
);

// ── tests ────────────────────────────────────────────────────────

/// outbound 非流式：OpenAI 客户端 → anthropic 渠道。请求被转换为
/// /v1/messages（x-api-key + system + model 映射），响应被转换回 OpenAI
/// 形状；按 anthropic usage（含 cache 拆分归一化）结算。
#[tokio::test]
async fn anthropic_channel_serves_openai_clients() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-upstream-claude"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_partial_json(json!({
            "model": "claude-sonnet-4-5",
            "max_tokens": 1000,
            "messages": [{"role": "user", "content": "Say hello"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_message(80, 20, 50)))
        .expect(1)
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(
        1,
        "anthropic",
        &server.uri(),
        "sk-upstream-claude",
        "gpt-4o",
        Some(json!({ "gpt-4o": "claude-sonnet-4-5" })),
    );
    let mut app = relay_router(&state, vec![channel]);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    // 响应转换回 OpenAI 形状
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["choices"][0]["message"]["content"], "Hello!");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    // usage 归一化：prompt = input + cache_read = 100
    assert_eq!(body["usage"]["prompt_tokens"], 100);
    assert_eq!(body["usage"]["completion_tokens"], 50);
    assert_eq!(body["usage"]["prompt_tokens_details"]["cached_tokens"], 20);

    // 结算：gpt-4o 价 × (100 输入 + 50 输出)
    let expected = gpt4o_quota(100, 50);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);

    server.verify().await;

    let logs = wait_logs_of(&state.pool, &token).await;
    let log = &logs[0];
    assert_eq!(
        (
            log.prompt_tokens,
            log.completion_tokens,
            log.cache_read_tokens
        ),
        (100, 50, 20),
        "cache splits normalized into the log"
    );
}

/// outbound 流式：anthropic SSE 事件流 → OpenAI chunk 流（含 usage 帧 +
/// [DONE]），结算以流终止后的 usage 为准。
#[tokio::test]
async fn anthropic_channel_streams_openai_chunks() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_partial_json(json!({ "stream": true })))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(CLAUDE_SSE)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(
        1,
        "anthropic",
        &server.uri(),
        "sk-up-claude",
        "gpt-4o",
        None,
    );
    let mut app = relay_router(&state, vec![channel]);

    let (status, bytes) = crate::send_raw(&mut app, chat_req(&sk, chat_body("gpt-4o", true))).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(bytes).expect("utf8 sse body");
    assert!(
        text.contains(r#""delta":{"role":"assistant","content":""}"#),
        "{text:?}"
    );
    assert!(text.contains(r#""content":"Hel""#));
    assert!(text.contains(r#""finish_reason":"stop""#));
    assert!(text.contains(r#""cached_tokens":0"#));
    assert!(text.contains("data: [DONE]"));

    // 结算：message_start 80 输入 + message_delta 20 输出
    let expected = gpt4o_quota(80, 20);
    let row = wait_token(&state.pool, token.id, |t| t.used_quota == expected).await;
    assert_eq!(
        row.used_quota, expected,
        "stream settle by translated usage"
    );
}

/// inbound：Anthropic SDK 客户端（x-api-key + /v1/messages）打在 openai
/// 兼容渠道上 — 请求转换、响应转换回 anthropic message 形状，同一套计费。
#[tokio::test]
async fn messages_face_converts_on_openai_channel() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(body_partial_json(json!({
            "model": "gpt-4o",
            "max_tokens": 1000,
            "messages": [
                {"role": "system", "content": "be nice"},
                {"role": "user", "content": "Say hello"}
            ]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-mock",
            "object": "chat.completion",
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello!" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 100, "completion_tokens": 50 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(1, "openai", &server.uri(), "sk-up-openai", "gpt-4o", None);
    let mut app = relay_router(&state, vec![channel]);

    let (status, body) =
        crate::send(&mut app, messages_req(&sk, messages_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    // anthropic message 形状
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["content"][0]["type"], "text");
    assert_eq!(body["content"][0]["text"], "Hello!");
    assert_eq!(body["stop_reason"], "end_turn");
    // openai usage → anthropic usage（input 不含 cache）
    assert_eq!(body["usage"]["input_tokens"], 100);
    assert_eq!(body["usage"]["output_tokens"], 50);

    let expected = gpt4o_quota(100, 50);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    server.verify().await;
}

/// inbound 流式：openai 兼容渠道的 SSE chunk → anthropic 事件流
/// （message_start → content_block_* → message_delta → message_stop）。
#[tokio::test]
async fn messages_face_streams_on_openai_channel() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    let sse = concat!(
        r#"data: {"id":"s1","object":"chat.completion.chunk","model":"gpt-4o","choices":[{"index":0,"delta":{"role":"assistant","content":""}}]}"#,
        "\n\n",
        r#"data: {"id":"s1","choices":[{"index":0,"delta":{"content":"Hel"}}]}"#,
        "\n\n",
        r#"data: {"id":"s1","choices":[{"index":0,"delta":{"content":"lo"}}]}"#,
        "\n\n",
        r#"data: {"id":"s1","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":80,"completion_tokens":20}}"#,
        "\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(1, "openai", &server.uri(), "sk-up-openai", "gpt-4o", None);
    let mut app = relay_router(&state, vec![channel]);

    let (status, bytes) =
        crate::send_raw(&mut app, messages_req(&sk, messages_body("gpt-4o", true))).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(bytes).expect("utf8 sse body");
    assert!(text.contains("event: message_start"), "{text:?}");
    assert!(text.contains(r#"event: content_block_start"#));
    assert!(text.contains(r#""type":"text_delta","text":"Hel""#));
    assert!(text.contains(r#"event: content_block_stop"#));
    assert!(text.contains(r#""stop_reason":"end_turn""#));
    assert!(text.contains(r#""output_tokens":20"#));
    assert!(text.contains("event: message_stop"));

    let expected = gpt4o_quota(80, 20);
    let row = wait_token(&state.pool, token.id, |t| t.used_quota == expected).await;
    assert_eq!(row.used_quota, expected);
}

/// inbound 透传：anthropic 原生渠道只做 model 改写，响应原样回传，
/// usage 从 anthropic 形状提取。
#[tokio::test]
async fn messages_face_passthrough_on_anthropic_channel() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-up-claude"))
        .and(body_partial_json(json!({
            "model": "claude-sonnet-4-5",
            "system": "be nice",
            "messages": [{"role": "user", "content": "Say hello"}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_message(100, 0, 50)))
        .expect(1)
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(
        1,
        "anthropic",
        &server.uri(),
        "sk-up-claude",
        "gpt-4o",
        Some(json!({ "gpt-4o": "claude-sonnet-4-5" })),
    );
    let mut app = relay_router(&state, vec![channel]);

    let (status, body) =
        crate::send(&mut app, messages_req(&sk, messages_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    // 原样透传（不做 openai 往返）
    assert_eq!(body["id"], "msg_mock");
    assert_eq!(body["model"], "claude-sonnet-4-5");
    assert_eq!(body["content"][0]["text"], "Hello!");
    assert_eq!(body["usage"]["input_tokens"], 100);

    let expected = gpt4o_quota(100, 50);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    server.verify().await;
}

/// 非 chat 模态短路：anthropic 渠道被 embeddings 请求跳过（配置错配不是
/// 上游健康信号），全额退款并返回 400。
#[tokio::test]
async fn anthropic_channel_skipped_for_embeddings() {
    let (_, state) = crate::test_app().await;
    let (sk, token) = make_llm_token(&state.pool, 10_000_000).await;
    let channel = chan(
        1,
        "anthropic",
        "http://127.0.0.1:1",
        "sk-up-claude",
        "text-embedding-3-small",
        None,
    );
    let mut app = relay_router(&state, vec![channel]);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/embeddings")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::from(
            serde_json::to_string(&json!({
                "model": "text-embedding-3-small",
                "input": "hi"
            }))
            .unwrap(),
        ))
        .unwrap();
    let (status, body) = crate::send(&mut app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("does not support this endpoint"),
        "{body:?}"
    );
    // 预扣全额退款
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(0));
    assert_eq!(row.remain_quota, Quota(10_000_000));
}
