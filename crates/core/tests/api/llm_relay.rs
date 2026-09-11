//! LLM relay HTTP 集成测试 — wiremock 模拟 OpenAI 兼容上游。
//!
//! 覆盖 `/v1/chat/completions` 全链路（design §4/§8/§9）：sk- 认证 →
//! 预扣（pre-consume）→ 通道路由/重试 → SSE 逐帧转发与非流式透传 →
//! 结算（settle/refund）→ 用量日志落库。channel 的 `base_url` 指向
//! wiremock 实例，上游 key 用明文存储（`crypto::decrypt` 对无前缀值
//! 透传，无需配置 AES 环境变量）。

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
use raisfast::llm::models::model::{LlmModel, LlmModelStatus, LlmModelType, LlmPriceMode};
use raisfast::llm::models::token::LlmToken;
use raisfast::llm::service::LlmRouter;
use raisfast::types::quota::Quota;
use raisfast::types::snowflake_id::SnowflakeId;

// ── helpers ──────────────────────────────────────────────────────

/// One upstream channel row pointing at a mock server (plaintext key —
/// decrypt passes unprefixed values through, so no AES env needed).
fn chan(
    id: i64,
    base_url: &str,
    upstream_key: &str,
    priority: i64,
    mapping: Option<Value>,
) -> LlmChannel {
    LlmChannel {
        id: SnowflakeId(id),
        tenant_id: Some("default".to_owned()),
        name: format!("mock-ch-{id}"),
        provider: "openai".to_owned(),
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
        models: "gpt-4o".to_owned(),
        model_mapping: mapping,
        priority,
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

/// Create an active user + sk- token with the given quota settings.
async fn make_llm_token(
    pool: &raisfast::db::Pool,
    remain: i64,
    unlimited: bool,
    allowed_models: Option<&str>,
) -> (String, LlmToken) {
    let username = format!("llm-relay-{}", raisfast::utils::id::new_id());
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
        "relay-test",
        &plain,
        Quota(remain),
        unlimited,
        None,
        allowed_models,
        None,
        None,
    )
    .await
    .unwrap();
    (plain, t)
}

/// Mount the relay `/v1` routes on a state whose llm_router is preloaded
/// with the given channels (mirrors the production root-level mount).
/// Returns the router handle for cooldown/cache state assertions.
fn relay_router(
    state: &raisfast::AppState,
    channels: Vec<LlmChannel>,
) -> (axum::Router, std::sync::Arc<LlmRouter>) {
    let router = LlmRouter::from_cache_for_test(ChannelCache::build(channels, vec![]));
    let mut s = state.clone();
    s.llm_router = router.clone();
    let app = axum::Router::new()
        .merge(raisfast::llm::relay::routes())
        .with_state(s);
    (app, router)
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

fn chat_body(model: &str, stream: bool) -> Value {
    json!({
        "model": model,
        "stream": stream,
        "max_tokens": 1000,
        "messages": [{ "role": "user", "content": "Say hello" }]
    })
}

async fn token_row(pool: &raisfast::db::Pool, id: SnowflakeId) -> LlmToken {
    raisfast::llm::models::token::find_by_id(pool, id, None)
        .await
        .unwrap()
        .unwrap()
}

/// Poll the token row until `pred` holds (stream settlement runs in a
/// spawned task), returning the last seen row.
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
    // 流式结算的日志插入是独立 spawn（settle-on-drop 语义），可能晚于
    // billing 提交——轮询等待而非立即断言（query_paged 先 SELECT 后
    // COUNT，恰好会观察到 total=1 / items=0 的中间态）。
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

/// 200 + usage mock returning OpenAI-shaped completion JSON.
fn ok_completion(usage_prompt: i64, usage_completion: i64, content: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "cmpl-mock",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": { "prompt_tokens": usage_prompt, "completion_tokens": usage_completion }
    }))
}

/// gpt-4o builtin pricing: input $2.5 / output $10 per 1M tokens. Since
/// `QUOTA_PER_USD == 1e6`, quota = ceil((p×2.5 + c×10)) (pricing.md §3).
fn gpt4o_quota(prompt: i64, completion: i64) -> Quota {
    let usd = (prompt as f64 * 2.5 + completion as f64 * 10.0) / 1_000_000.0;
    Quota::from_usd_ceil(usd)
}

// ── tests ────────────────────────────────────────────────────────

/// 非流式全链路：model 映射改写、上游 key 透传、响应透传、按真实
/// usage 结算并落一条 relay 日志；上游恰好被命中一次。
#[tokio::test]
async fn relay_nonstream_settles_and_logs() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-upstream-abc"))
        // model_mapping 改写必须到达上游
        .and(body_partial_json(json!({ "model": "gpt-4o-2024-11-20" })))
        .respond_with(ok_completion(100, 50, "Hello!"))
        .expect(1)
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channel = chan(
        1,
        &server.uri(),
        "sk-upstream-abc",
        0,
        Some(json!({ "gpt-4o": "gpt-4o-2024-11-20" })),
    );
    let (mut app, _router) = relay_router(&state, vec![channel]);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["choices"][0]["message"]["content"], "Hello!");

    // 上游恰好一次，且 auth 头 / 模型改写均匹配（expect(1) + verify）。
    server.verify().await;

    // 结算：actual = (100×2.5 + 50×2.0)/2 = 175（非流式 settle 在返回前 await）。
    let expected = gpt4o_quota(100, 50);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    assert_eq!(row.remain_quota, Quota(10_000_000) - expected);

    let logs = wait_logs_of(&state.pool, &token).await;
    let log = &logs[0];
    assert_eq!(log.model_name, "gpt-4o");
    assert_eq!(
        (log.prompt_tokens, log.completion_tokens),
        (100, 50),
        "usage normalized into the log"
    );
    assert_eq!(log.quota, expected);
    assert!(!log.is_stream);
    assert_eq!(log.status_code, Some(200));
}

/// 流式全链路：SSE 帧逐字节透传（含 [DONE] 与 usage 帧）、以流式标记
/// 落日志；结算在流终止后由 spawn 任务完成（轮询等待）。
#[tokio::test]
async fn relay_stream_forwards_sse_and_settles() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    let sse = concat!(
        "data: {\"id\":\"s1\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        "data: {\"id\":\"s1\",\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        "data: {\"id\":\"s1\",\"choices\":[],\"usage\":{\"prompt_tokens\":80,\"completion_tokens\":20}}\n\n",
        "data: [DONE]\n\n",
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channel = chan(1, &server.uri(), "sk-up-1", 0, None);
    let (mut app, _router) = relay_router(&state, vec![channel]);

    let (status, bytes) = crate::send_raw(&mut app, chat_req(&sk, chat_body("gpt-4o", true))).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(bytes).expect("utf8 sse body");
    assert!(text.contains("Hel"), "delta frames forwarded: {text:?}");
    assert!(text.contains("[DONE]"), "terminator forwarded: {text:?}");
    assert!(
        text.contains("\"prompt_tokens\":80"),
        "usage frame forwarded verbatim: {text:?}"
    );
    server.verify().await;

    // 流式结算由 spawn 完成 —— 轮询等待 used_quota 到位。
    let expected = gpt4o_quota(80, 20);
    let row = wait_token(&state.pool, token.id, |t| t.used_quota == expected).await;
    assert_eq!(row.remain_quota, Quota(10_000_000) - expected);

    let logs = wait_logs_of(&state.pool, &token).await;
    let log = &logs[0];
    assert!(log.is_stream);
    assert_eq!((log.prompt_tokens, log.completion_tokens), (80, 20));
    assert_eq!(log.quota, expected);
}

/// 跨 channel 故障转移：高优先级渠道 500 → 下一优先级渠道接住，
/// 客户端拿到 200；两个上游各命中一次。
#[tokio::test]
async fn relay_failover_across_channels_on_5xx() {
    let (_, state) = crate::test_app().await;
    let broken = MockServer::start().await;
    let healthy = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({
            "error": { "message": "upstream exploded", "type": "server_error" }
        })))
        .expect(1)
        .mount(&broken)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(10, 10, "from healthy"))
        .expect(1)
        .mount(&healthy)
        .await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channels = vec![
        chan(1, &broken.uri(), "sk-broken", 10, None),
        chan(2, &healthy.uri(), "sk-healthy", 5, None),
    ];
    let (mut app, _router) = relay_router(&state, channels);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["choices"][0]["message"]["content"], "from healthy");
    broken.verify().await;
    healthy.verify().await;

    // 只按成功渠道的 usage 结算。
    let expected = gpt4o_quota(10, 10);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    assert_eq!(row.remain_quota, Quota(10_000_000) - expected);
}

/// 全部渠道失败：重试预算耗尽（DEFAULT_RETRY_TIMES+1 = 3 次上游命中，
/// 每个渠道至少一次），上游 5xx 状态码透传 + OpenAI 错误信封，预扣全额退还。
#[tokio::test]
async fn relay_all_channels_fail_passthrough_status_and_full_refund() {
    let (_, state) = crate::test_app().await;
    let a = MockServer::start().await;
    let b = MockServer::start().await;

    for server in [&a, &b] {
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream exploded"))
            // 两个同级渠道随机加权：每个被 1~2 次命中，合计恰好 3。
            .expect(1..=2)
            .mount(server)
            .await;
    }

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channels = vec![
        chan(1, &a.uri(), "sk-a", 0, None),
        chan(2, &b.uri(), "sk-b", 0, None),
    ];
    let (mut app, _router) = relay_router(&state, channels);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "body: {body:?}");
    // OpenAI 兼容错误信封（SDK 依赖该形状解析错误）。
    assert_eq!(body["error"]["type"], "api_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("upstream exploded")),
        "body: {body:?}"
    );
    a.verify().await;
    b.verify().await;

    // 预扣全额退还：余额分文未动。
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(0));
    assert_eq!(row.remain_quota, Quota(10_000_000));
}

/// 认证与准入错误：缺 bearer / 未知 sk → 401；未知模型 → 400；
/// 模型白名单拒绝 → 403；配额不足 → 429 且余额不被透支。
#[tokio::test]
async fn relay_auth_model_and_quota_errors() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(1, 1, "unused"))
        .expect(0)
        .mount(&server)
        .await;

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (sk_wl, _t_wl) = make_llm_token(&state.pool, 10_000_000, false, Some("gpt-4o-mini")).await;
    let (sk_poor, token_poor) = make_llm_token(&state.pool, 100, false, None).await;
    let channel = chan(1, &server.uri(), "sk-up", 0, None);
    let (mut app, _router) = relay_router(&state, vec![channel]);

    // 缺 bearer 头 → 401 + OpenAI 错误信封。
    let no_bearer = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_string(&chat_body("gpt-4o", false)).unwrap(),
        ))
        .unwrap();
    let (status, body) = crate::send(&mut app, no_bearer).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "body: {body:?}");
    assert_eq!(body["error"]["type"], "invalid_request_error");

    // 未知 sk → 401。
    let (status, _) = crate::send(
        &mut app,
        chat_req("sk-does-not-exist", chat_body("gpt-4o", false)),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // 未知模型 → 400（builtin 目录之外的模型直接拒绝，不触碰配额）。
    let (status, body) =
        crate::send(&mut app, chat_req(&sk, chat_body("no-such-model", false))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown model")
    );

    // 模型白名单 → 403。
    let (status, _) = crate::send(&mut app, chat_req(&sk_wl, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 配额不足 → 429，且余额不被透支（预扣原子守卫）。
    let (status, body) =
        crate::send(&mut app, chat_req(&sk_poor, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body: {body:?}");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    let row = token_row(&state.pool, token_poor.id).await;
    assert_eq!(row.remain_quota, Quota(100), "overdraw protection");
    assert_eq!(row.used_quota, Quota(0));

    // 全部被拒 —— 上游零命中。
    server.verify().await;
}

/// GET /v1/models：渠道模型 ∩ 激活目录（builtin）∩ token 白名单。
#[tokio::test]
async fn relay_models_listing_intersected_with_allowlist() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;

    let (sk, _t) = make_llm_token(&state.pool, 10_000, false, Some("gpt-4o")).await;
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "gpt-4o,gpt-4o-mini,no-such-model".to_owned();
    let (mut app, _router) = relay_router(&state, vec![channel]);

    let req = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = crate::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    // 白名单只剩 gpt-4o；目录外模型（no-such-model）被过滤。
    assert_eq!(ids, vec!["gpt-4o"]);
    assert_eq!(body["data"][0]["object"], "model");
}

// ── 上游故障场景（design §6.2 三分类：Transient / Arrears / Window）──

/// A URI 指向一个刚释放的端口 —— connection refused（transport error）。
fn dead_uri() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{addr}")
}

/// 偶发网络错误（connection refused）：分类为 Transient，同请求内
/// 立即切换到下一渠道；失败 key 进入指数退避冷却。
#[tokio::test]
async fn relay_transient_network_error_fails_over() {
    let (_, state) = crate::test_app().await;
    let healthy = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(10, 10, "after network blip"))
        .expect(1)
        .mount(&healthy)
        .await;

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channels = vec![
        chan(1, &dead_uri(), "sk-dead", 10, None),
        chan(2, &healthy.uri(), "sk-healthy", 5, None),
    ];
    let (mut app, router) = relay_router(&state, channels);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(
        body["choices"][0]["message"]["content"],
        "after network blip"
    );
    healthy.verify().await;

    // 死渠道 key 处于 Transient 冷却（指数退避起点 5s，§6.2；断言时已
    // 流逝数毫秒，下界放宽到 4s）。
    let snaps = router.cooldown_snapshot();
    assert!(
        snaps.iter().any(|(id, _idx, kind, dur, _)| {
            *id == SnowflakeId(1)
                && *kind == raisfast::llm::service::CooldownKind::Transient
                && dur.as_secs() >= 4
        }),
        "transient cooldown recorded: {snaps:?}"
    );
}

/// 5 小时窗口限额（coding-plan 风格 429 + reset 关键词）：分类为
/// Window —— key 保持 Active 但冷却 30min，**不会**被 last-resort 重试，
/// 后续请求直接绕开它。
#[tokio::test]
async fn relay_window_limit_cooldown_never_last_resorted() {
    let (_, state) = crate::test_app().await;
    let limited = MockServer::start().await;
    let healthy = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "message": "You exceeded your usage limit; limit will reset at 2026-09-11T22:00:00Z",
                "type": "rate_limit_error"
            }
        })))
        // 窗口冷却只允许第一次命中 —— 任何重试都说明 last-resort 泄漏。
        .expect(1)
        .mount(&limited)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(5, 5, "window standby"))
        // 两次请求均由健康渠道接住。
        .expect(2)
        .mount(&healthy)
        .await;

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channels = vec![
        chan(1, &limited.uri(), "sk-limited", 10, None),
        chan(2, &healthy.uri(), "sk-healthy", 5, None),
    ];
    let (mut app, router) = relay_router(&state, channels);

    // 请求 1：高优先级渠道吃 429 → Window 冷却 → 健康渠道接住。
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["choices"][0]["message"]["content"], "window standby");

    // 请求 2：窗口冷却中的 key 在选路阶段即被排除（而非 last-resort）。
    let (status, _) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK);
    limited.verify().await;
    healthy.verify().await;

    let snaps = router.cooldown_snapshot();
    assert!(
        snaps.iter().any(|(id, _idx, kind, dur, reason)| {
            *id == SnowflakeId(1)
                && *kind == raisfast::llm::service::CooldownKind::Window
                && dur.as_secs() >= 29 * 60
                && reason.contains("usage limit")
        }),
        "window cooldown recorded with reset horizon: {snaps:?}"
    );
}

/// §6.2 头信号（文案之外的另一半）：429 + `Retry-After: 18000`（5h 窗口）
/// 但文案**无** reset 关键词 → 头解析兜住，仍判 Window 并冷却至 reset
/// 时刻（≤6h 封顶）；单渠道下快速失败、不 last-resort、上游恰好 1 次。
#[tokio::test]
async fn relay_retry_after_header_marks_window_without_keywords() {
    let (_, state) = crate::test_app().await;
    let limited = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "18000")
                // generic 文案：不命中任何窗口/欠费关键词。
                .set_body_string("Rate limit exceeded"),
        )
        .expect(1)
        .mount(&limited)
        .await;

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app, router) = relay_router(&state, vec![chan(1, &limited.uri(), "sk-l", 0, None)]);

    // 单渠道：429 → 头信号判 Window → 无 last-resort → NoRoute 出口回
    // 透传上游错误（而非"no available channel"）。
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body: {body:?}");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Rate limit exceeded")),
        "upstream error surfaced: {body:?}"
    );
    limited.verify().await;

    // 冷却至 reset 时刻（18000s，6h 封顶内），Window 类。
    let snaps = router.cooldown_snapshot();
    assert!(
        snaps.iter().any(|(id, _idx, kind, dur, _)| {
            *id == SnowflakeId(1)
                && *kind == raisfast::llm::service::CooldownKind::Window
                && dur.as_secs() >= 17_000
        }),
        "header-driven window cooldown ≈ 5h: {snaps:?}"
    );
}

/// §6.2 小值 Retry-After（≤60s）：仍是 Transient（秒级自愈），但冷却
/// 时长取 Retry-After 值（优先于 5s 指数退避起点）；健康渠道接住请求。
#[tokio::test]
async fn relay_small_retry_after_stays_transient_with_precise_cooldown() {
    let (_, state) = crate::test_app().await;
    let blip = MockServer::start().await;
    let healthy = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_string("Rate limit exceeded"),
        )
        .expect(1)
        .mount(&blip)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(5, 5, "precise cooldown"))
        .expect(1)
        .mount(&healthy)
        .await;

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channels = vec![
        chan(1, &blip.uri(), "sk-blip", 10, None),
        chan(2, &healthy.uri(), "sk-ok", 5, None),
    ];
    let (mut app, router) = relay_router(&state, channels);

    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    blip.verify().await;
    healthy.verify().await;

    // Transient 类、冷却 ≈ 30s（Retry-After 优先，非 5s 起步）。
    let snaps = router.cooldown_snapshot();
    assert!(
        snaps.iter().any(|(id, _idx, kind, dur, _)| {
            *id == SnowflakeId(1)
                && *kind == raisfast::llm::service::CooldownKind::Transient
                && (28..=30).contains(&dur.as_secs())
        }),
        "precise transient cooldown from Retry-After: {snaps:?}"
    );
}

/// 欠费（402 + insufficient balance 关键词）：分类为 Arrears —— key 同步
/// 拉黑（内存路由表驱逐，渠道 AutoDisabled），异步持久化到 DB 的 key
/// pool JSON；§7.3 欠费类**重试且封禁**：当次换渠道救活请求，后续请求
/// 永久绕开被禁渠道（直到 admin/健康巡检恢复）。
#[tokio::test]
async fn relay_arrears_disables_key_and_persists_to_db() {
    let (_, state) = crate::test_app().await;
    let broke = MockServer::start().await;
    let healthy = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(402).set_body_json(json!({
            "error": { "message": "insufficient balance", "type": "invalid_request_error" }
        })))
        // 欠费是硬拉黑：生命周期内只允许第一次命中。
        .expect(1)
        .mount(&broke)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(5, 5, "arrears standby"))
        .expect(2)
        .mount(&healthy)
        .await;

    // 真实 DB channel 行 + LlmRouter::new —— 验证欠费禁用的持久化闭环。
    let broke_row = insert_db_channel(&state.pool, &broke.uri(), &["sk-broke"], 10).await;
    let _healthy_row = insert_db_channel(&state.pool, &healthy.uri(), &["sk-healthy2"], 5).await;

    let router = LlmRouter::new(state.pool.clone()).await;
    let mut s = state.clone();
    s.llm_router = router.clone();
    let mut app = axum::Router::new()
        .merge(raisfast::llm::relay::routes())
        .with_state(s);

    let (sk, _token) = make_llm_token(&state.pool, 10_000_000, false, None).await;

    // 请求 1：402 → 欠费封禁该 key → 同请求内换健康渠道救活。
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["choices"][0]["message"]["content"], "arrears standby");

    // 请求 2：欠费渠道已被路由表驱逐，直接命中健康渠道。
    let (status, _) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK);
    broke.verify().await;
    healthy.verify().await;

    // 内存：渠道 AutoDisabled，无 cooldown 条目（Arrears 走状态不走冷却）。
    let cache = router.cache_snapshot();
    let broke_cached = cache.channels.get(&broke_row.id).expect("channel cached");
    assert_eq!(
        broke_cached.status,
        raisfast::llm::models::channel::LlmChannelStatus::AutoDisabled
    );
    assert!(
        !router
            .cooldown_snapshot()
            .iter()
            .any(|(id, ..)| *id == broke_row.id),
        "arrears disables via key status, not cooldown"
    );

    // DB：欠费禁用异步落库（key pool JSON 内 status + reason）。
    let persisted = wait_key_disabled(&state.pool, broke_row.id).await;
    assert!(
        persisted
            .disabled_reason
            .as_deref()
            .is_some_and(|r| r.contains("insufficient balance")),
        "reason persisted: {persisted:?}"
    );
}

/// 流中途断连（上游 TCP 异常关闭）：SSE 已转发的帧保留，结算按断连前
/// 已累积的 usage 执行（settle-on-error，§8.4）。
#[tokio::test]
async fn relay_stream_abort_midway_settles_partial_usage() {
    let (_, state) = crate::test_app().await;
    let uri = spawn_abort_sse_upstream().await;

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let channel = chan(1, &uri, "sk-flaky", 0, None);
    let (mut app, _router) = relay_router(&state, vec![channel]);

    let (status, bytes) =
        send_stream_lossy(&mut app, chat_req(&sk, chat_body("gpt-4o", true))).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\"prompt_tokens\":80"),
        "frames before the abort were forwarded: {text:?}"
    );

    // 断连触发 settle-on-error：按已累积 usage（80/20）结算并落日志。
    let expected = gpt4o_quota(80, 20);
    let row = wait_token(&state.pool, token.id, |t| t.used_quota == expected).await;
    assert_eq!(row.remain_quota, Quota(10_000_000) - expected);

    let logs = wait_logs_of(&state.pool, &token).await;
    let log = &logs[0];
    assert!(log.is_stream);
    assert_eq!((log.prompt_tokens, log.completion_tokens), (80, 20));
    assert_eq!(log.quota, expected);
}

// ── 并发 / 排队准入场景（design §7.6：有界排队、双上限、RAII 槽）──

/// 并发容量受限的渠道（key 级 `max_concurrency` 信号量）。
fn chan_cap(id: i64, base_url: &str, key: &str, max_concurrency: i32) -> LlmChannel {
    let mut channel = chan(id, base_url, key, 0, None);
    channel.api_keys = serde_json::to_value(vec![LlmKeyEntry {
        key: key.to_owned(),
        status: LlmKeyStatus::Active,
        disabled_reason: None,
        disabled_at: None,
        max_concurrency: Some(max_concurrency),
    }])
    .unwrap();
    channel
}

/// Spawn one concurrent chat request against a cloned router.
fn spawn_chat(
    app: &axum::Router,
    sk: &str,
    body: Value,
) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let app = app.clone();
    let sk = sk.to_owned();
    tokio::spawn(async move { crate::send(&mut { app }, chat_req(&sk, body)).await })
}

/// `send` + response headers (Retry-After assertions).
async fn send_with_headers(
    app: &mut axum::Router,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, headers, val)
}

/// 满载排队后 drain：key 容量 2、4 个并发（独立 token）→ 2 个直连、
/// 2 个入 FIFO 队列，上游释放后依次获槽 —— 全部 200，无拒绝。
#[tokio::test]
async fn relay_concurrency_cap_queues_then_drains() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ok_completion(5, 5, "queued ok").set_delay(std::time::Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    let mut sks = Vec::new();
    for _ in 0..4 {
        let (sk, _t) = make_llm_token(&state.pool, 10_000_000, false, None).await;
        sks.push(sk);
    }
    let (app, router) = relay_router(&state, vec![chan_cap(1, &server.uri(), "sk-up", 2)]);

    let handles: Vec<_> = sks
        .iter()
        .map(|sk| spawn_chat(&app, sk, chat_body("gpt-4o", false)))
        .collect();

    // 占坑期间必然出现排队（500ms 窗口内轮询观测）。
    let mut saw_waiting = false;
    for _ in 0..25 {
        if router.total_waiting() > 0 {
            saw_waiting = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(saw_waiting, "2-capacity slot must queue the other requests");

    let mut oks = 0;
    for h in handles {
        let (status, body) = h.await.unwrap();
        assert_eq!(status, StatusCode::OK, "body: {body:?}");
        oks += 1;
    }
    assert_eq!(oks, 4, "queued requests drain after slot release");
}

/// per-token 并发上限（默认 10）：同一 token 的第 11 个在飞请求立即
/// 429 + Retry-After: 5，其余 10 个正常完成（§7.6 防单 token 独占）。
#[tokio::test]
async fn relay_token_concurrency_cap_returns_429() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ok_completion(5, 5, "inflight ok").set_delay(std::time::Duration::from_millis(800)),
        )
        .mount(&server)
        .await;

    let (sk, _t) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (app, _router) = relay_router(&state, vec![chan(1, &server.uri(), "sk-up", 0, None)]);

    let handles: Vec<_> = (0..11)
        .map(|_| {
            let app = app.clone();
            let sk = sk.clone();
            tokio::spawn(async move {
                let req = chat_req(&sk, chat_body("gpt-4o", false));
                let mut a = app;
                send_with_headers(&mut a, req).await
            })
        })
        .collect();

    let mut oks = 0;
    let mut rejected = 0;
    for h in handles {
        let (status, headers, body) = h.await.unwrap();
        if status == StatusCode::OK {
            oks += 1;
        } else {
            assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body: {body:?}");
            assert!(
                body["error"]["message"]
                    .as_str()
                    .is_some_and(|m| m.contains("token concurrency limit reached (10)")),
                "body: {body:?}"
            );
            assert_eq!(
                headers.get("retry-after").and_then(|v| v.to_str().ok()),
                Some("5")
            );
            rejected += 1;
        }
    }
    assert_eq!(
        (oks, rejected),
        (10, 1),
        "exactly one over-cap request rejected"
    );
}

/// Router 层准入内核：队满（深度上限）立即 503 overloaded；排队者等过
/// 时间上限 → 429 "queue wait budget exhausted" + Retry-After（§7.6 双上限）。
#[tokio::test]
async fn slot_admission_queue_full_and_wait_timeout() {
    use raisfast::llm::service::{QueueTier, ResolveCtx, RetryState, SlotError, SlotRequest};

    let router = LlmRouter::from_cache_for_test(ChannelCache::build(
        vec![chan_cap(1, "http://up.test", "sk-up", 1)],
        vec![],
    ));
    let cache = router.cache_snapshot();
    let ctx = ResolveCtx {
        tenant: "default",
        group: None,
        pin_channel: None,
    };
    // 深度上限压到 1：第二个排队者即满员。
    let tiny = QueueTier {
        max_waiting: 1,
        max_waiting_bytes: 1 << 20,
        max_wait_secs: 60,
    };
    let far = std::time::Instant::now() + std::time::Duration::from_secs(60);

    // #1 占住唯一槽位（permit 持有不放）。
    let (_c, _i, _permit1) = router
        .acquire_slot(
            &SlotRequest {
                cache: &cache,
                ctx: &ctx,
                model: "gpt-4o",
                tier: tiny,
                caller: None,
                body_bytes: 8,
                deadline: far,
            },
            &mut RetryState::default(),
        )
        .await
        .expect("first acquirer takes the only slot");

    // #2 排队（唯一队位），等待预算 150ms。
    let r2 = router.clone();
    let h2 = tokio::spawn(async move {
        let cache2 = r2.cache_snapshot();
        let ctx2 = ResolveCtx {
            tenant: "default",
            group: None,
            pin_channel: None,
        };
        r2.acquire_slot(
            &SlotRequest {
                cache: &cache2,
                ctx: &ctx2,
                model: "gpt-4o",
                tier: tiny,
                caller: None,
                body_bytes: 8,
                deadline: std::time::Instant::now() + std::time::Duration::from_millis(150),
            },
            &mut RetryState::default(),
        )
        .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await; // 让 #2 入队

    // #3：队满（深度 1 已占）→ 立即 503 overloaded，无 Retry-After。
    match router
        .acquire_slot(
            &SlotRequest {
                cache: &cache,
                ctx: &ctx,
                model: "gpt-4o",
                tier: tiny,
                caller: None,
                body_bytes: 8,
                deadline: far,
            },
            &mut RetryState::default(),
        )
        .await
    {
        Err(SlotError::Rejected {
            status,
            message,
            retry_after_secs,
        }) => {
            assert_eq!(status, 503);
            assert!(message.contains("overloaded"), "message: {message}");
            assert_eq!(retry_after_secs, None);
        }
        _ => panic!("queue-full must reject with 503"),
    }

    // #2：等到头 → 429 + Retry-After（队位 × EWMA 时长）。
    match h2.await.unwrap() {
        Err(SlotError::Rejected {
            status,
            message,
            retry_after_secs,
        }) => {
            assert_eq!(status, 429);
            assert!(
                message.contains("queue wait budget exhausted"),
                "message: {message}"
            );
            assert!(retry_after_secs.is_some_and(|s| s >= 1), "retry-after hint");
        }
        _ => panic!("waiter must time out with 429"),
    }
}

/// Router 层在飞计数：per-token 上限 10 / per-user 上限 20（防多 token
/// 放大）；user 拒绝时 token 计数必须回滚（§7.6 RAII 钉子——漏一个
/// 出口 = 该 token 永久 429）。
#[tokio::test]
async fn slot_admission_token_and_user_concurrency_caps() {
    use raisfast::llm::service::{ResolveCtx, RetryState, SlotError, SlotRequest};

    let router = LlmRouter::from_cache_for_test(ChannelCache::build(
        vec![chan(1, "http://up.test", "sk-up", 0, None)],
        vec![],
    ));
    let cache = router.cache_snapshot();
    let ctx = ResolveCtx {
        tenant: "default",
        group: None,
        pin_channel: None,
    };
    let far = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let req = |token, user| SlotRequest {
        cache: &cache,
        ctx: &ctx,
        model: "gpt-4o",
        tier: RELAY_TIER_FOR_TEST,
        caller: Some((SnowflakeId(token), SnowflakeId(user))),
        body_bytes: 8,
        deadline: far,
    };
    let mut permits = Vec::new();

    // 同 token 满 10 个在飞（permits 持有不放，保持计数占用）。
    for _ in 0..10 {
        match router
            .acquire_slot(&req(101, 201), &mut RetryState::default())
            .await
        {
            Ok((_ch, _i, p)) => permits.push(p),
            Err(_) => panic!("acquire within token cap must succeed"),
        }
    }
    // 第 11 个同 token 请求 → token 上限 429。
    match router
        .acquire_slot(&req(101, 201), &mut RetryState::default())
        .await
    {
        Err(SlotError::Rejected {
            status, message, ..
        }) => {
            assert_eq!(status, 429);
            assert!(
                message.contains("token concurrency limit reached (10)"),
                "message: {message}"
            );
        }
        _ => panic!("token cap must reject"),
    }

    // 同 user 换 token 继续：user 计数 10 → 20，第 21 个（新 token）→
    // user 上限 429。
    for _ in 0..10 {
        match router
            .acquire_slot(&req(102, 201), &mut RetryState::default())
            .await
        {
            Ok((_ch, _i, p)) => permits.push(p),
            Err(_) => panic!("acquire within user cap must succeed"),
        }
    }
    match router
        .acquire_slot(&req(103, 201), &mut RetryState::default())
        .await
    {
        Err(SlotError::Rejected {
            status, message, ..
        }) => {
            assert_eq!(status, 429);
            assert!(
                message.contains("user concurrency limit reached (20)"),
                "message: {message}"
            );
        }
        _ => panic!("user cap must reject"),
    }

    // 回滚回归：user 拒绝不得泄漏 token 103 的计数——换 user 后同 token
    // 仍应可用满 10 个额度。
    for _ in 0..10 {
        let acquired = router
            .acquire_slot(&req(103, 202), &mut RetryState::default())
            .await;
        match acquired {
            Ok((_ch, _i, p)) => permits.push(p),
            Err(SlotError::Rejected { message, .. }) => {
                panic!("in-flight counter leaked across user-cap rejection: {message}")
            }
            _ => panic!("unexpected"),
        }
    }
}

/// 流式槽生命周期（§7.6 钉死）：流终止（含客户端中断 drop）前槽与
/// 在飞计数不得释放——A 流挂着时 B 不得穿透到上游；A 中断后 B 获槽。
#[tokio::test]
async fn relay_stream_holds_slot_until_stream_ends() {
    let (_, state) = crate::test_app().await;
    let conns = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let uri = spawn_hang_then_complete_sse(conns.clone()).await;

    let (sk_a, _ta) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (sk_b, _tb) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (app, _router) = relay_router(&state, vec![chan_cap(1, &uri, "sk-up", 1)]);

    // A：流式请求，上游发一帧后挂住（流保持打开、槽被占用）。
    let app_a = app.clone();
    let sk_a2 = sk_a.clone();
    let handle_a = tokio::spawn(async move {
        let mut a = app_a;
        crate::send_raw(&mut a, chat_req(&sk_a2, chat_body("gpt-4o", true))).await
    });
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        conns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "A connected and holds the only slot"
    );

    // B：同模型流式请求 —— 必须排队，不得穿透到上游。
    let handle_b = spawn_chat(&app, &sk_b, chat_body("gpt-4o", true));
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert_eq!(
        conns.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "B must not reach upstream while A's stream holds the slot"
    );
    assert!(!handle_b.is_finished(), "B is still queued");

    // A 客户端中断 → 流终止 → 槽释放 → B 获槽并完成。
    handle_a.abort();
    let (status_b, body_b) = tokio::time::timeout(std::time::Duration::from_secs(5), handle_b)
        .await
        .expect("B completes after A's disconnect")
        .unwrap();
    assert_eq!(status_b, StatusCode::OK, "body: {body_b:?}");
    assert_eq!(
        conns.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "B connected only after the slot was released"
    );
}

/// RELAY_TIER 常量在 router 层测试中复用（槽不限容，纯在飞计数验证）。
const RELAY_TIER_FOR_TEST: raisfast::llm::service::QueueTier = raisfast::llm::service::RELAY_TIER;

/// 原生 TCP 上游：第 1 个连接发一帧 SSE 后挂住（等客户端断开），
/// 之后的连接发完整 chunked 响应后关闭。用于流式槽生命周期测试。
async fn spawn_hang_then_complete_sse(
    conns: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n";
        let frame = "data: {\"id\":\"s\",\"choices\":[{\"delta\":{\"content\":\"x\"}}]}\n\n";
        let chunk = format!("{:x}\r\n{}\r\n", frame.len(), frame);
        while let Ok((mut sock, _)) = listener.accept().await {
            let n = conns.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            let chunk = chunk.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 16384];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(chunk.as_bytes()).await;
                if n == 1 {
                    // A：保持流打开，直到客户端中断（读到 EOF）。
                    let _ = sock.read(&mut buf).await;
                } else {
                    // B：完整终止 chunked 响应。
                    let _ = sock.write_all(b"0\r\n\r\n").await;
                }
            });
        }
    });
    format!("http://{addr}")
}

// ── 故障场景 helpers ─────────────────────────────────────────────

/// Insert a real channel row (plaintext keys — decrypt passes through) and
/// return it, for tests that exercise `LlmRouter::new` persistence.
/// One channel per call; `keys` seeds the key pool (polling mode).
async fn insert_db_channel(
    pool: &raisfast::db::Pool,
    base_url: &str,
    keys: &[&str],
    priority: i64,
) -> LlmChannel {
    use raisfast::llm::models::channel as ch;
    let entries: Vec<LlmKeyEntry> = keys
        .iter()
        .map(|k| LlmKeyEntry {
            key: (*k).to_owned(),
            status: LlmKeyStatus::Active,
            disabled_reason: None,
            disabled_at: None,
            max_concurrency: None,
        })
        .collect();
    ch::create_channel(
        pool,
        None,
        ch::NewChannel {
            name: format!("relay-db-{}", raisfast::utils::id::new_id()),
            provider: "openai".to_owned(),
            base_url: base_url.to_owned(),
            api_keys: ch::keys_value(&entries),
            key_mode: LlmKeyMode::Polling,
            models: "gpt-4o".to_owned(),
            model_mapping: None,
            priority,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            cost_mode: ch::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
        },
    )
    .await
    .unwrap()
}

/// Poll the DB until the channel's first key lands Disabled (arrears
/// persistence is spawned behind the per-channel lock).
async fn wait_key_disabled(
    pool: &raisfast::db::Pool,
    channel_id: SnowflakeId,
) -> raisfast::llm::models::channel::LlmKeyEntry {
    for _ in 0..200 {
        let row = raisfast::llm::models::channel::find_by_id(pool, channel_id, None)
            .await
            .unwrap()
            .unwrap();
        let entry = raisfast::llm::models::channel::parse_keys(&row)
            .into_iter()
            .next()
            .unwrap();
        if entry.status == LlmKeyStatus::Disabled {
            return entry;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("key never persisted as disabled for channel {channel_id}");
}

/// A raw TCP upstream that speaks chunked SSE for two frames, then drops
/// the connection without the terminating chunk — simulating an upstream
/// dying mid-stream (wiremock can only send complete bodies).
async fn spawn_abort_sse_upstream() -> String {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 16384];
        let _ = sock.read(&mut buf).await;
        let frame1 = "data: {\"id\":\"s1\",\"choices\":[],\"usage\":{\"prompt_tokens\":80,\"completion_tokens\":20}}\n\n";
        let frame2 = "data: {\"id\":\"s1\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{}\r\n{:x}\r\n{}\r\n",
            frame1.len(),
            frame1,
            frame2.len(),
            frame2
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    });
    format!("http://{addr}")
}

/// Collect a (possibly erroring) response body, keeping bytes forwarded
/// before the error — `send_raw` would panic on a mid-stream abort.
async fn send_stream_lossy(app: &mut axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let clone = app.clone();
    let resp = clone.oneshot(req).await.unwrap();
    let status = resp.status();
    let mut body = resp.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        match frame {
            Ok(f) => {
                if let Some(data) = f.data_ref() {
                    bytes.extend_from_slice(data);
                }
            }
            Err(_) => break,
        }
    }
    (status, bytes)
}

// ── 计费模式与重试边界场景（§7.3 / §9.3）──────────────────────────

/// Build a directory model row for pricing tests (USD per 1M tokens).
fn model_row(
    name: &str,
    price_mode: LlmPriceMode,
    input_price: f64,
    output_price: f64,
    cache_read_price: Option<f64>,
    call_price: Option<f64>,
) -> LlmModel {
    LlmModel {
        id: SnowflakeId(raisfast::utils::id::new_id()),
        tenant_id: Some("default".to_owned()),
        name: name.to_owned(),
        model_type: LlmModelType::Chat,
        price_mode,
        input_price,
        output_price,
        cache_read_price,
        cache_write_price: None,
        call_price,
        params: None,
        status: LlmModelStatus::Active,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    }
}

/// `relay_router` 变体：同时注入目录模型行（定价测试用）。
fn relay_router_with_models(
    state: &raisfast::AppState,
    channels: Vec<LlmChannel>,
    models: Vec<LlmModel>,
) -> (axum::Router, std::sync::Arc<LlmRouter>) {
    let router = LlmRouter::from_cache_for_test(ChannelCache::build(channels, models));
    let mut s = state.clone();
    s.llm_router = router.clone();
    let app = axum::Router::new()
        .merge(raisfast::llm::relay::routes())
        .with_state(s);
    (app, router)
}

/// 多 key 号池（§6.2"先 key 后渠道"）：单 key 欠费 → 该 key 拉黑但渠道
/// 存活（还有 active key）→ 同请求内换 key 接住；后续请求轮转绕开死 key。
#[tokio::test]
async fn relay_arrears_single_key_fails_over_within_pool() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;

    // 同一上游用 Authorization 区分两把 key：key0 欠费，key1 健康。
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-pool-a"))
        .respond_with(ResponseTemplate::new(402).set_body_json(json!({
            "error": { "message": "insufficient balance" }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer sk-pool-b"))
        .respond_with(ok_completion(5, 5, "pool key-b"))
        .expect(2)
        .mount(&server)
        .await;

    // 渠道带两把明文 key（polling 从 key0 起轮转）。
    let row = insert_db_channel(&state.pool, &server.uri(), &["sk-pool-a", "sk-pool-b"], 0).await;
    let router = LlmRouter::new(state.pool.clone()).await;
    let mut s = state.clone();
    s.llm_router = router.clone();
    let mut app = axum::Router::new()
        .merge(raisfast::llm::relay::routes())
        .with_state(s);

    let (sk, _t) = make_llm_token(&state.pool, 10_000_000, false, None).await;

    // 请求 1：key0 402 → 欠费拉黑（渠道仍有 key1，不 AutoDisabled）→ 换 key1。
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    assert_eq!(body["choices"][0]["message"]["content"], "pool key-b");

    // 请求 2：key0 已禁用，轮转直接落 key1。
    let (status, _) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    // 渠道整体仍 enabled（有任一 active key 即活，§6.3）。
    let cache = router.cache_snapshot();
    let cached = cache.channels.get(&row.id).expect("channel cached");
    assert_eq!(cached.status, LlmChannelStatus::Enabled);
    assert_eq!(cached.keys.len(), 2);
    assert_eq!(
        cached.keys[0].status,
        LlmKeyStatus::Disabled,
        "arrears key banned"
    );
    assert_eq!(cached.keys[1].status, LlmKeyStatus::Active);

    // DB：仅 key0 落禁用。
    let persisted = wait_key_disabled(&state.pool, row.id).await;
    assert!(
        persisted
            .disabled_reason
            .as_deref()
            .is_some_and(|r| r.contains("insufficient balance"))
    );
}

/// per_call 计费（§9.3 资损补丁）：按次全额预扣（call_price × 500k），
/// 结算同额；余额不足时全额拒绝——零余额 token 无法"先放行后补扣"。
#[tokio::test]
async fn relay_per_call_billing_full_precharge() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(10, 10, "per call"))
        .mount(&server)
        .await;

    let model = model_row(
        "per-call-model",
        LlmPriceMode::PerCall,
        0.0,
        0.0,
        None,
        Some(0.01),
    );
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "per-call-model".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    // call_price 0.01 → 预扣 = 结算 = 0.01 × 1,000,000 = 10000。
    let (sk, token) = make_llm_token(&state.pool, 12_000, false, None).await;
    let body = json!({
        "model": "per-call-model",
        "messages": [{ "role": "user", "content": "Say hello" }]
    });
    let (status, resp) = crate::send(&mut app, chat_req(&sk, body.clone())).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(
        row.used_quota,
        Quota(10_000),
        "per-call settles the flat call price"
    );
    assert_eq!(row.remain_quota, Quota(2_000));

    // 余额 9999 < 全额预扣 10000 → 429 拒绝，余额不动。
    let (sk_poor, token_poor) = make_llm_token(&state.pool, 9_999, false, None).await;
    let (status, resp) = crate::send(&mut app, chat_req(&sk_poor, body)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "body: {resp:?}");
    let row = token_row(&state.pool, token_poor.id).await;
    assert_eq!(
        row.remain_quota,
        Quota(9_999),
        "full pre-charge blocks freeloaders"
    );
    assert_eq!(row.used_quota, Quota(0));
}

/// cache 读分离计费（§9.3）：prompt_tokens 含 cache（OpenAI 语义），
/// base = prompt − cached 按 model_ratio、cached 按 cache_ratio 分别计价。
#[tokio::test]
async fn relay_cache_split_billing() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-c", "object": "chat.completion", "model": "cache-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "hi" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 500,
                "prompt_tokens_details": { "cached_tokens": 600 }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    // input $1 / output $2 / cache_read $0.1（USD per 1M）：
    // base (1000−600)×$1 + 600×$0.1 + 500×$2 = 400+60+1000 → 1460 quota。
    let model = model_row(
        "cache-model",
        LlmPriceMode::Token,
        1.0,
        2.0,
        Some(0.1),
        None,
    );
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "cache-model".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let body = json!({
        "model": "cache-model",
        "messages": [{ "role": "user", "content": "Say hello" }]
    });
    let (status, resp) = crate::send(&mut app, chat_req(&sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    server.verify().await;

    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(1_460), "cache split pricing");
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].cache_read_tokens, 600, "cached tokens logged");
    assert_eq!(logs[0].quota, Quota(1_460));
}

/// §7.3 不重试集合：400（确定性失败，不惩罚 key）、504（超时类，冷却
/// 但不重试）、解析失败（上游响应体异常）——全部单次命中即返回，预扣
/// 全额退还。
#[tokio::test]
async fn relay_non_retryable_failures_400_504_parse() {
    let (_, state) = crate::test_app().await;

    // ── 400：客户端错 —— 单次命中、key 不冷却（§6.2 class 4 豁免）。──
    let s400 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": { "message": "max_tokens is required" }
        })))
        .expect(1)
        .mount(&s400)
        .await;
    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app, router) = relay_router(&state, vec![chan(1, &s400.uri(), "sk-a", 0, None)]);
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body:?}");
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("max_tokens")),
        "upstream 400 forwarded verbatim: {body:?}"
    );
    s400.verify().await;
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(
        (row.remain_quota, row.used_quota),
        (Quota(10_000_000), Quota(0)),
        "full refund"
    );
    assert!(
        router.cooldown_snapshot().is_empty(),
        "400 must not cool the key (client error, key innocent)"
    );

    // ── 504：超时类 —— 单次命中、全额退款，但照常冷却（过载分流）。──
    let s504 = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(504).set_body_string("gateway timeout"))
        .expect(1)
        .mount(&s504)
        .await;
    let (sk2, token2) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app2, router2) = relay_router(&state, vec![chan(2, &s504.uri(), "sk-b", 0, None)]);
    let (status2, body2) = crate::send(&mut app2, chat_req(&sk2, chat_body("gpt-4o", false))).await;
    assert_eq!(status2, StatusCode::GATEWAY_TIMEOUT, "body: {body2:?}");
    s504.verify().await;
    let row2 = token_row(&state.pool, token2.id).await;
    assert_eq!(
        (row2.remain_quota, row2.used_quota),
        (Quota(10_000_000), Quota(0)),
        "full refund"
    );
    assert!(
        !router2.cooldown_snapshot().is_empty(),
        "504 still cools the key (overload shedding), it just never retries"
    );

    // ── 解析失败：200 + 非 JSON 体 —— 单次命中即 502，不重试。──
    let sparse = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>maintenance</html>"))
        .expect(1)
        .mount(&sparse)
        .await;
    let (sk3, token3) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app3, _r3) = relay_router(&state, vec![chan(3, &sparse.uri(), "sk-c", 0, None)]);
    let (status3, body3) = crate::send(&mut app3, chat_req(&sk3, chat_body("gpt-4o", false))).await;
    assert_eq!(status3, StatusCode::BAD_GATEWAY, "body: {body3:?}");
    sparse.verify().await;
    let row3 = token_row(&state.pool, token3.id).await;
    assert_eq!(
        (row3.remain_quota, row3.used_quota),
        (Quota(10_000_000), Quota(0)),
        "full refund"
    );
}

/// §8.4 无 usage 估算兜底：上游不报 usage（部分兼容实现）时按字符估算
/// 计费（chars/4），流式与非流式都不允许零结算白嫖。
#[tokio::test]
async fn relay_no_usage_billed_by_estimation() {
    let (_, state) = crate::test_app().await;

    // ── 非流式：响应体无 usage 字段 → prompt 9/4=2、content 11/4=2 估算。──
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-e", "object": "chat.completion", "model": "gpt-4o",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "Hello there" }, "finish_reason": "stop" }]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app, _r) = relay_router(&state, vec![chan(1, &server.uri(), "sk-a", 0, None)]);
    let (status, body) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK, "body: {body:?}");
    server.verify().await;
    let expected = gpt4o_quota(9 / 4, "Hello there".len() as i64 / 4);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected, "estimated, never zero-billed");
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].prompt_tokens, 2, "estimated prompt tokens");
    assert_eq!(logs[0].completion_tokens, 2, "estimated completion tokens");

    // ── 流式：无 usage 帧 → 按转发 delta 字符估算（"Hel"+"lo" 5/4=1）。──
    let sse = concat!(
        "data: {\"id\":\"s\",\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
        "data: {\"id\":\"s\",\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
        "data: [DONE]\n\n",
    );
    let sserver = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&sserver)
        .await;
    let (sk2, token2) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (mut app2, _r2) = relay_router(&state, vec![chan(2, &sserver.uri(), "sk-b", 0, None)]);
    let (status2, bytes) =
        crate::send_raw(&mut app2, chat_req(&sk2, chat_body("gpt-4o", true))).await;
    assert_eq!(status2, StatusCode::OK);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("[DONE]"), "stream forwarded: {text:?}");
    sserver.verify().await;

    let expected2 = gpt4o_quota(9 / 4, 5 / 4);
    let row2 = wait_token(&state.pool, token2.id, |t| t.used_quota == expected2).await;
    assert_eq!(row2.remain_quota, Quota(10_000_000) - expected2);
    let logs2 = wait_logs_of(&state.pool, &token2).await;
    assert!(logs2[0].is_stream);
    assert_eq!(
        logs2[0].quota, expected2,
        "stream estimation settles, not zero"
    );
}

// ── 定价与利润：逐笔精确账（pricing.md §3/§7）────────────────────

/// Persist the `llm_group_ratios` option (JSON string, as the seed stores it).
async fn set_group_ratios(pool: &raisfast::db::Pool, ratios: serde_json::Value) {
    raisfast::models::options::upsert_value(
        pool,
        "llm_group_ratios",
        &serde_json::Value::String(ratios.to_string()),
        None,
    )
    .await
    .unwrap();
}

/// Create an active user + token in a given billing group.
async fn make_llm_token_group(
    pool: &raisfast::db::Pool,
    remain: i64,
    group: &str,
) -> (String, LlmToken) {
    let username = format!("llm-price-{}", raisfast::utils::id::new_id());
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
        "price-test",
        &plain,
        Quota(remain),
        false,
        None,
        None,
        None,
        Some(group),
    )
    .await
    .unwrap();
    (plain, t)
}

/// Full price/cost/profit ledger for one non-stream request:
/// input $2.5 / output $10 / cache-read $0.5; usage 1000 prompt (200 cached)
/// + 500 completion; group 1.5 sell, 0.8 cost discount.
///
/// base = 1000−200 = 800 → 800×2.5 + 200×0.5 + 500×10 = 7100 (USD×1M)
///   → billable $0.0071
/// charge     = ceil(0.0071 × 1.5 × 1e6) = 10,650 quota  ($0.01065)
/// cost_quota = ceil(0.0071 × 0.8 × 1e6) =  5,680 quota  ($0.00568)
/// profit     = 4,970 quota ($0.00497)
#[tokio::test]
async fn relay_pricing_exact_charge_cost_and_profit() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.5, "vip": 2.0 })).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "cmpl-p", "object": "chat.completion", "model": "price-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }],
            "usage": {
                "prompt_tokens": 1000,
                "completion_tokens": 500,
                "prompt_tokens_details": { "cached_tokens": 200 }
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let model = model_row(
        "price-model",
        LlmPriceMode::Token,
        2.5,
        10.0,
        Some(0.5),
        None,
    );
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "price-model".to_owned();
    channel.cost_mode = raisfast::llm::models::channel::LlmCostMode::Usage;
    channel.cost_discount = 0.8;
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({
        "model": "price-model",
        "messages": [{ "role": "user", "content": "price please" }]
    });
    let (status, resp) = crate::send(&mut app, chat_req(&sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    server.verify().await;

    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(10_650), "exact user charge");
    assert_eq!(row.remain_quota, Quota(1_000_000 - 10_650));

    let logs = wait_logs_of(&state.pool, &token).await;
    let log = &logs[0];
    assert_eq!(log.quota, Quota(10_650), "log charge");
    assert_eq!(log.cost_quota, Quota(5_680), "exact channel cost");
    assert_eq!(log.cache_read_tokens, 200, "cache split logged");
    assert_eq!(
        log.quota - log.cost_quota,
        Quota(4_970),
        "per-request profit"
    );
    // Audit trail: group ratio + cost mode persisted in detail.
    assert_eq!(log.detail.as_ref().unwrap()["group_ratio"], 1.5);
    assert_eq!(log.detail.as_ref().unwrap()["cost_discount"], 0.8);
}

/// Sell group comes from the token's `token_group`; unknown → 1.0.
#[tokio::test]
async fn relay_pricing_group_ratio_from_token_group() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.0, "vip": 2.0 })).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "g-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 1000, "completion_tokens": 0 }
        })))
        .mount(&server)
        .await;

    let model = model_row("g-model", LlmPriceMode::Token, 3.0, 0.0, None, None);
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "g-model".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    // vip: 1000×$3/1M = $0.003 → ×2 = $0.006 → 6000 quota.
    let (sk_vip, t_vip) = make_llm_token_group(&state.pool, 1_000_000, "vip").await;
    let body = json!({ "model": "g-model", "messages": [{ "role": "user", "content": "hi" }] });
    let (status, _) = crate::send(&mut app, chat_req(&sk_vip, body.clone())).await;
    assert_eq!(status, StatusCode::OK);
    let row = token_row(&state.pool, t_vip.id).await;
    assert_eq!(row.used_quota, Quota(6_000), "vip ×2");

    // ghost group not in the map → defaults to 1.0 → 3000 quota.
    let (sk_ghost, t_ghost) = make_llm_token_group(&state.pool, 1_000_000, "ghost").await;
    let (status, _) = crate::send(&mut app, chat_req(&sk_ghost, body)).await;
    assert_eq!(status, StatusCode::OK);
    let row = token_row(&state.pool, t_ghost.id).await;
    assert_eq!(row.used_quota, Quota(3_000), "unknown group → 1.0");
}

/// Subscription (`fixed`) upstream: per-request cost is 0, so logged profit
/// equals the full charge (real cost lives in the monthly pool, §7.2).
#[tokio::test]
async fn relay_pricing_fixed_cost_mode_zero_cost() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.0 })).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(1_000_000, 0, "sub"))
        .expect(1)
        .mount(&server)
        .await;

    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.cost_mode = raisfast::llm::models::channel::LlmCostMode::Fixed;
    channel.monthly_cost = Some(200.0);
    let (mut app, _r) = relay_router(&state, vec![channel]);

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let (status, _) = crate::send(&mut app, chat_req(&sk, chat_body("gpt-4o", false))).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    // gpt-4o builtin $2.5 input × 1M = $2.5 → 2,500,000 quota, cost 0.
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(2_500_000));
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].quota, Quota(2_500_000));
    assert_eq!(logs[0].cost_quota, Quota(0));
    assert_eq!(
        logs[0].quota - logs[0].cost_quota,
        Quota(2_500_000),
        "full revenue booked as profit"
    );
}

/// Streaming settles to exactly the same charge/cost as non-stream for the
/// same usage — no rounding drift between the two paths.
#[tokio::test]
async fn relay_pricing_stream_matches_nonstream() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.5 })).await;

    let sse = concat!(
        "data: {\"id\":\"s\",\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        "data: {\"id\":\"s\",\"choices\":[],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":500,\"prompt_tokens_details\":{\"cached_tokens\":200}}}\n\n",
        "data: [DONE]\n\n",
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let model = model_row("s-model", LlmPriceMode::Token, 2.5, 10.0, Some(0.5), None);
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "s-model".to_owned();
    channel.cost_mode = raisfast::llm::models::channel::LlmCostMode::Usage;
    channel.cost_discount = 0.8;
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 10_000_000, false, None).await;
    let body = json!({
        "model": "s-model",
        "stream": true,
        "messages": [{ "role": "user", "content": "stream please" }]
    });
    let (status, _) = crate::send_raw(&mut app, chat_req(&sk, body)).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    let (charge, cost) = (Quota(10_650), Quota(5_680));
    let row = wait_token(&state.pool, token.id, |t| t.used_quota == charge).await;
    assert_eq!(row.remain_quota, Quota(10_000_000) - charge);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert!(logs[0].is_stream);
    assert_eq!(logs[0].quota, charge, "stream charge == non-stream charge");
    assert_eq!(logs[0].cost_quota, cost, "stream cost == non-stream cost");
}

/// Pre-charge is a hold, not the final bill: the ledger must reflect the
/// actual settle (over-hold refunded), and the log detail records the hold.
#[tokio::test]
async fn relay_pricing_settle_refunds_overhold_exactly() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.0 })).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c", "object": "chat.completion", "model": "h-model",
            "choices": [{ "index": 0, "message": { "role": "assistant", "content": "ok" }, "finish_reason": "stop" }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 10 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let model = model_row("h-model", LlmPriceMode::Token, 2.0, 4.0, None, None);
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "h-model".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    // Request has max_tokens 1000 → hold = (500×2 + 1000×4) = 5000 quota.
    // Actual = 10×2 + 10×4 = 60 quota → refund 4940.
    let (sk, token) = make_llm_token(&state.pool, 100_000, false, None).await;
    let (status, _) = crate::send(&mut app, chat_req(&sk, chat_body("h-model", false))).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(60), "actual usage only");
    assert_eq!(row.remain_quota, Quota(100_000 - 60), "hold fully refunded");
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].quota, Quota(60));
    // detail audits the hold in USD (5000 quota = $0.005).
    assert_eq!(logs[0].detail.as_ref().unwrap()["pre_consumed"], 0.005);
}

/// Hold uses the documented max_tokens chain: request → model
/// `params.max_output_tokens` → 4096 (pricing.md §3.2).
#[tokio::test]
async fn relay_pricing_hold_uses_params_max_output_tokens() {
    let (_, state) = crate::test_app().await;
    set_group_ratios(&state.pool, json!({ "default": 1.0 })).await;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ok_completion(10, 10, "ok"))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("p-model", LlmPriceMode::Token, 2.0, 4.0, None, None);
    model.params = Some(json!({ "max_output_tokens": 2000 }));
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "p-model".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    // No request max_tokens → params 2000. hold = (500×2 + 2000×4) = 9000
    // quota = $0.009.
    let (sk, token) = make_llm_token(&state.pool, 100_000, false, None).await;
    let body = json!({ "model": "p-model", "messages": [{ "role": "user", "content": "hi" }] });
    let (status, _) = crate::send(&mut app, chat_req(&sk, body)).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    // actual = 10×2 + 10×4 = 60 quota.
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(60));
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].detail.as_ref().unwrap()["pre_consumed"], 0.009);
}

// ── /v1/embeddings + /v1/rerank（非 chat 模态数据面）──────────────

fn json_req(uri: &str, sk: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap()
}

/// /v1/embeddings 全链路：透传 + 按 usage.prompt_tokens 结算 + 日志
/// （completion 恒 0，input 价计费）。
#[tokio::test]
async fn relay_embeddings_settles_from_usage() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .and(body_partial_json(json!({ "model": "emb-1" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "index": 0, "embedding": [0.1, 0.2] }],
            "model": "emb-1",
            "usage": { "prompt_tokens": 100, "total_tokens": 100 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("emb-1", LlmPriceMode::Token, 2.0, 0.0, None, None);
    model.model_type = LlmModelType::Embedding;
    let mut channel = chan(1, &server.uri(), "sk-up-emb", 0, None);
    channel.models = "emb-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "emb-1", "input": "hello world" });
    let (status, resp) = crate::send(&mut app, json_req("/v1/embeddings", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"][0]["embedding"][0], 0.1);
    server.verify().await;

    // actual = 100 tokens × $2/1M = 200 quota（无 output 预留）。
    let expected = Quota(200);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    assert_eq!(row.remain_quota, Quota(1_000_000) - expected);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].model_name, "emb-1");
    assert_eq!((logs[0].prompt_tokens, logs[0].completion_tokens), (100, 0));
    assert_eq!(logs[0].quota, expected);
}

/// /v1/rerank 全链路：Jina/Cohere 形态 usage.total_tokens 计费。
#[tokio::test]
async fn relay_rerank_settles_from_total_tokens() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/rerank"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "rr-1",
            "results": [{ "index": 1, "relevance_score": 0.98 }],
            "usage": { "total_tokens": 500 }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("rr-1", LlmPriceMode::Token, 1.0, 0.0, None, None);
    model.model_type = LlmModelType::Rerank;
    let mut channel = chan(1, &server.uri(), "sk-up-rr", 0, None);
    channel.models = "rr-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({
        "model": "rr-1",
        "query": "what is rust",
        "documents": ["a language", "a car polish", "iron oxide"],
        "top_n": 2
    });
    let (status, resp) = crate::send(&mut app, json_req("/v1/rerank", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["results"][0]["index"], 1);
    server.verify().await;

    // total_tokens 500 → prompt 500 × $1/1M = 500 quota。
    let expected = Quota(500);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!((logs[0].prompt_tokens, logs[0].completion_tokens), (500, 0));
    assert_eq!(logs[0].quota, expected);
}

/// 上游无 usage → §8.4 字符估算兜底（input 400 chars → 100 tokens），
/// 绝不免费放行。
#[tokio::test]
async fn relay_embeddings_estimates_when_upstream_has_no_usage() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{ "index": 0, "embedding": [0.5] }],
            "model": "emb-1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("emb-1", LlmPriceMode::Token, 2.0, 0.0, None, None);
    model.model_type = LlmModelType::Embedding;
    let mut channel = chan(1, &server.uri(), "sk-up-emb", 0, None);
    channel.models = "emb-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "emb-1", "input": "a".repeat(400) });
    let (status, _) = crate::send(&mut app, json_req("/v1/embeddings", &sk, body)).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    // 400 chars / 4 = 100 tokens × $2/1M = 200 quota。
    let expected = Quota(200);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
}

/// 模态守卫：chat 模型调 /v1/embeddings → 400；embedding 模型调
/// /v1/chat/completions → 400（现有 chat 守卫）。
#[tokio::test]
async fn relay_rejects_model_type_mismatch_on_new_endpoints() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;

    // chat 渠道（gpt-4o builtin 定价）+ embedding 目录行。
    let mut model = model_row("emb-1", LlmPriceMode::Token, 2.0, 0.0, None, None);
    model.model_type = LlmModelType::Embedding;
    let mut channel = chan(1, &server.uri(), "sk-up", 0, None);
    channel.models = "gpt-4o,emb-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;

    // chat 模型 → /v1/embeddings：400，不触上游。
    let (status, resp) = crate::send(
        &mut app,
        json_req(
            "/v1/embeddings",
            &sk,
            json!({ "model": "gpt-4o", "input": "x" }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {resp:?}");
    // embedding 模型 → /v1/chat/completions：400（现有守卫）。
    let (status, resp) = crate::send(
        &mut app,
        json_req(
            "/v1/chat/completions",
            &sk,
            json!({ "model": "emb-1", "messages": [{ "role": "user", "content": "hi" }] }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {resp:?}");
    server.verify().await;

    // 守卫拒绝不扣费（rejection refunds nothing——未预扣）。
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(0));
}

// ── /v1/images/generations（文生图数据面）────────────────────────

/// token 模式按张计费：completion 侧 = 生成图片数（output_price = 每张
/// 价的 1e6 倍），input 侧 = 1。
#[tokio::test]
async fn relay_images_token_mode_bills_per_image() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .and(body_partial_json(json!({ "model": "img-1", "n": 2 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "created": 1700000000,
            "data": [
                { "url": "https://cdn.test/a.png" },
                { "url": "https://cdn.test/b.png" }
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    // output_price $0.04/张 → 40_000；input_price 0。
    let mut model = model_row("img-1", LlmPriceMode::Token, 0.0, 40_000.0, None, None);
    model.model_type = LlmModelType::Image;
    let mut channel = chan(1, &server.uri(), "sk-up-img", 0, None);
    channel.models = "img-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "img-1", "prompt": "a cat", "n": 2, "size": "1024x1024" });
    let (status, resp) = crate::send(&mut app, json_req("/v1/images/generations", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"].as_array().unwrap().len(), 2);
    server.verify().await;

    // 2 张 × $0.04 = $0.08 → 80_000 quota（hold 同额，settle 无差额）。
    let expected = Quota(80_000);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    assert_eq!(row.remain_quota, Quota(1_000_000) - expected);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(
        (logs[0].prompt_tokens, logs[0].completion_tokens),
        (1, 2),
        "images billed on the completion side"
    );
    assert_eq!(logs[0].quota, expected);
}

/// per_call 模式：不论 n 张，按次一口价。
#[tokio::test]
async fn relay_images_per_call_mode_flat_price() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "created": 1700000000,
            "data": [{ "b64_json": "AAAA" }, { "b64_json": "BBBB" }, { "b64_json": "CCCC" }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("img-2", LlmPriceMode::PerCall, 0.0, 0.0, None, Some(0.02));
    model.model_type = LlmModelType::Image;
    let mut channel = chan(1, &server.uri(), "sk-up-img", 0, None);
    channel.models = "img-2".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "img-2", "prompt": "a dog", "n": 3 });
    let (status, resp) = crate::send(&mut app, json_req("/v1/images/generations", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    server.verify().await;

    // $0.02/次 → 20_000 quota，n=3 不放大。
    let expected = Quota(20_000);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
}

// ── /v1/audio/*（asr / tts 数据面）────────────────────────────────

fn multipart_body(model: &str, file_bytes: &[u8]) -> Vec<u8> {
    let boundary = "raisfast-test-boundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"model\"\r\n\r\n{model}\r\n").as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        "Content-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\n\
         Content-Type: audio/wav\r\n\r\n"
            .as_bytes(),
    );
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn stt_req(sk: &str, model: &str, file_bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/v1/audio/transcriptions")
        .header(
            header::CONTENT_TYPE,
            "multipart/form-data; boundary=raisfast-test-boundary",
        )
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::from(multipart_body(model, file_bytes)))
        .unwrap()
}

/// STT：verbose_json 带 duration → 按秒计费（input_price = $/1M 秒）。
#[tokio::test]
async fn relay_asr_bills_by_duration() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "hello world",
            "duration": 6.4
        })))
        .expect(1)
        .mount(&server)
        .await;

    // input_price 100.0 → $0.0001/秒（≈$0.006/分钟）。
    let mut model = model_row("asr-1", LlmPriceMode::Token, 100.0, 0.0, None, None);
    model.model_type = LlmModelType::Asr;
    let mut channel = chan(1, &server.uri(), "sk-up-asr", 0, None);
    channel.models = "asr-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let file = vec![0u8; 65_536]; // 64KB → est 2s（hold），实际 duration 6.4→7s
    let (status, resp) = crate::send(&mut app, stt_req(&sk, "asr-1", &file)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["text"], "hello world");
    server.verify().await;

    // 7 秒 × $0.0001 = $0.0007 → 700 quota。
    let expected = Quota(700);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].prompt_tokens, 7);
    assert_eq!(logs[0].quota, expected);
}

/// STT 无 duration → 文件大小估算兜底（64KB ≈ 2s），绝不免费。
#[tokio::test]
async fn relay_asr_estimates_from_file_size_without_duration() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/transcriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "text": "short clip"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("asr-1", LlmPriceMode::Token, 100.0, 0.0, None, None);
    model.model_type = LlmModelType::Asr;
    let mut channel = chan(1, &server.uri(), "sk-up-asr", 0, None);
    channel.models = "asr-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let file = vec![0u8; 65_536]; // → 2s
    let (status, _) = crate::send(&mut app, stt_req(&sk, "asr-1", &file)).await;
    assert_eq!(status, StatusCode::OK);
    server.verify().await;

    let expected = Quota(200);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
}

/// TTS：按输入字符计费，二进制音频透传（content-type 保留）。
#[tokio::test]
async fn relay_tts_bills_by_input_chars_and_streams_audio() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/audio/speech"))
        .and(body_partial_json(
            json!({ "model": "tts-1", "input": "hello there" }),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "audio/mpeg")
                .set_body_bytes(vec![0xFF, 0xF3, 0x40, 0x00]),
        )
        .expect(1)
        .mount(&server)
        .await;

    // input_price 150.0 → $0.00015/字符（tts-1 ≈ $15/1M chars）。
    let mut model = model_row("tts-1", LlmPriceMode::Token, 150.0, 0.0, None, None);
    model.model_type = LlmModelType::Tts;
    let mut channel = chan(1, &server.uri(), "sk-up-tts", 0, None);
    channel.models = "tts-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "tts-1", "input": "hello there", "voice": "alloy" });
    let (status, body_bytes) =
        crate::send_raw(&mut app, json_req("/v1/audio/speech", &sk, body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body_bytes,
        vec![0xFF, 0xF3, 0x40, 0x00],
        "audio bytes forwarded"
    );
    server.verify().await;

    // "hello there" = 11 字符 × $0.00015 = $0.00165 → 1650 quota。
    let expected = Quota(1_650);
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, expected);
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].prompt_tokens, 11);
    assert_eq!(logs[0].quota, expected);
}

// ── /v1/videos（异步视频生成数据面）───────────────────────────────

/// 全链路：提交（预扣）→ 轮询（结算+落日志）→ 下载内容。
#[tokio::test]
async fn relay_video_submit_poll_and_settle() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;

    // 提交 → 返回上游任务 id。
    Mock::given(method("POST"))
        .and(path("/videos"))
        .and(body_partial_json(json!({ "model": "vid-1", "seconds": 8 })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vid-upstream-1", "object": "video", "status": "queued"
        })))
        .expect(1)
        .mount(&server)
        .await;
    // 轮询 → completed。
    Mock::given(method("GET"))
        .and(path("/videos/vid-upstream-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vid-upstream-1", "status": "completed", "progress": 100,
            "url": "https://up.test/v/1.mp4"
        })))
        .expect(1)
        .mount(&server)
        .await;
    // 内容下载 → mp4 字节。
    Mock::given(method("GET"))
        .and(path("/videos/vid-upstream-1/content"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "video/mp4")
                .set_body_bytes(vec![0x00, 0x00, 0x00, 0x18, 0x66, 0x74]),
        )
        .expect(1)
        .mount(&server)
        .await;

    // output_price 10_000 → $0.01/秒；input 0。
    let mut model = model_row("vid-1", LlmPriceMode::Token, 0.0, 10_000.0, None, None);
    model.model_type = LlmModelType::Video;
    let mut channel = chan(1, &server.uri(), "sk-up-vid", 0, None);
    channel.models = "vid-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;

    // 提交：预扣 8s × $0.01 = 80_000 quota。
    let body = json!({ "model": "vid-1", "prompt": "a cat surfing", "seconds": 8 });
    let (status, resp) = crate::send(&mut app, json_req("/v1/videos", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["status"], "queued");
    let task_id = resp["id"].as_str().unwrap().to_owned();
    let held = token_row(&state.pool, token.id).await;
    assert_eq!(
        held.remain_quota,
        Quota(1_000_000 - 80_000),
        "hold taken at submit"
    );

    // 轮询：completed → 结算（hold == actual，无差额）。
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{task_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::empty())
        .unwrap();
    let (status, resp) = crate::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["status"], "completed");
    assert_eq!(resp["result"]["url"], "https://up.test/v/1.mp4");
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.used_quota, Quota(80_000));
    assert_eq!(row.remain_quota, Quota(1_000_000 - 80_000));
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].completion_tokens, 8, "billed on the seconds side");
    assert_eq!(logs[0].quota, Quota(80_000));

    // 内容下载：mp4 字节透传。
    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{task_id}/content"))
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::empty())
        .unwrap();
    let (status, bytes) = crate::send_raw(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x18, 0x66, 0x74]);
    server.verify().await;
}

/// 上游 failed → 全额退款 + failed 终态 + 错误日志。
#[tokio::test]
async fn relay_video_fail_refunds() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/videos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vid-up-2", "status": "queued"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/videos/vid-up-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vid-up-2", "status": "failed",
            "error": { "message": "content policy violation" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("vid-1", LlmPriceMode::Token, 0.0, 10_000.0, None, None);
    model.model_type = LlmModelType::Video;
    let mut channel = chan(1, &server.uri(), "sk-up-vid", 0, None);
    channel.models = "vid-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let body = json!({ "model": "vid-1", "prompt": "a dog", "seconds": 5 });
    let (status, resp) = crate::send(&mut app, json_req("/v1/videos", &sk, body)).await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let task_id = resp["id"].as_str().unwrap().to_owned();
    let held = token_row(&state.pool, token.id).await;
    assert_eq!(
        held.remain_quota,
        Quota(1_000_000 - 50_000),
        "hold at submit"
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{task_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {sk}"))
        .body(Body::empty())
        .unwrap();
    let (status, resp) = crate::send(&mut app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["status"], "failed");
    assert_eq!(resp["error"]["message"], "content policy violation");

    // 全额退款：remain 恢复，used 不变。
    let row = token_row(&state.pool, token.id).await;
    assert_eq!(row.remain_quota, Quota(1_000_000), "full refund");
    assert_eq!(row.used_quota, Quota(0));
    let logs = wait_logs_of(&state.pool, &token).await;
    assert_eq!(logs[0].quota, Quota(0));
    assert!(logs[0].error_message.is_some());
    server.verify().await;
}

/// 他人 token 访问任务 → 404（token 作用域）。
#[tokio::test]
async fn relay_video_task_scoped_to_token() {
    let (_, state) = crate::test_app().await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/videos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "vid-up-3", "status": "queued"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut model = model_row("vid-1", LlmPriceMode::Token, 0.0, 10_000.0, None, None);
    model.model_type = LlmModelType::Video;
    let mut channel = chan(1, &server.uri(), "sk-up-vid", 0, None);
    channel.models = "vid-1".to_owned();
    let (mut app, _r) = relay_router_with_models(&state, vec![channel], vec![model]);

    let (sk, token) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let (sk_other, _t2) = make_llm_token(&state.pool, 1_000_000, false, None).await;
    let (status, resp) = crate::send(
        &mut app,
        json_req(
            "/v1/videos",
            &sk,
            json!({ "model": "vid-1", "prompt": "p", "seconds": 4 }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let task_id = resp["id"].as_str().unwrap().to_owned();

    let req = Request::builder()
        .method("GET")
        .uri(format!("/v1/videos/{task_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {sk_other}"))
        .body(Body::empty())
        .unwrap();
    let (status, _) = crate::send(&mut app, req).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "task hidden from other tokens"
    );
    let _ = token;
}
