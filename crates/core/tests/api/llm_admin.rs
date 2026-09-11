//! LLM 控制面集成测试 — 管理端（§12）、用户自服务 token、健康 cron
//!（§7.5/§6.3）。数据面（relay 转发链路）见 `llm_relay.rs`。
//!
//! 认证走 `AuthUser`（JWT claims），admin 端点用 admin 角色 JWT，自服务
//! 端点用普通用户 JWT。渠道 key 经 AES 加密落库（密钥 = APP_KEY，
//! `build_test_app` 里安装），DTO 侧只见掩码（§11.4）。

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use wiremock::matchers::{body_partial_json, header as hdr, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use raisfast::llm::models::channel::{self as ch_model, LlmChannelStatus, LlmKeyStatus};
use raisfast::types::snowflake_id::SnowflakeId;

// ── helpers ──────────────────────────────────────────────────────

/// Mount the llm admin/self-service routes (reg_route! registers bare
/// paths; the `/api/v1` prefix comes from the production nest).
fn llm_admin_app(state: &raisfast::AppState) -> axum::Router {
    let mut registry = raisfast::server::RouteRegistry::default();
    let config = state.config.clone();
    axum::Router::new()
        .nest(
            "/api/v1",
            raisfast::llm::handler::routes(&mut registry, &config),
        )
        .with_state(state.clone())
}

/// Create a real user and mint a JWT with the given role.
async fn user_jwt(pool: &raisfast::db::Pool, admin: bool) -> (SnowflakeId, String) {
    let username = format!("llm-ctl-{}", raisfast::utils::id::new_id());
    let cmd = raisfast::commands::CreateUserCmd::new(
        username,
        raisfast::models::user::RegisteredVia::Email,
    );
    let u = raisfast::models::user::create(pool, &cmd, None)
        .await
        .unwrap();
    let role = if admin {
        raisfast::models::user::UserRole::Admin
    } else {
        raisfast::models::user::UserRole::Author
    };
    let jwt = crate::make_token("", u.id.0, role);
    (u.id, jwt)
}

fn admin_req(method_: &str, path: &str, jwt: &str, body: Option<Value>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method_)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {jwt}"));
    let body = match body {
        Some(v) => {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_string(&v).unwrap())
        }
        None => Body::empty(),
    };
    b.body(body).unwrap()
}

fn channel_body(name: &str, base_url: &str) -> Value {
    json!({
        "name": name,
        "provider": "openai",
        "base_url": base_url,
        "models": "gpt-4o, gpt-4o-mini",
        "test_model": "gpt-4o",
        "initial_keys": [
            { "key": "sk-admin-secret-key-aaaa" },
            { "key": "sk-admin-secret-key-bbbb" },
        ]
    })
}

// ── 管理端：渠道 CRUD + 校验 + 掩码（§11.4/§12）──────────────────

#[tokio::test]
async fn admin_channel_crud_validates_and_masks_keys() {
    let (_, state) = crate::test_app().await;
    let (admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let (_reader_id, reader_jwt) = user_jwt(&state.pool, false).await;
    let mut app = llm_admin_app(&state);

    // 非 admin → 403。
    let (status, _) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/channels", &reader_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 校验：空名 / 非法 base_url / 空 models / 空 keys / 空 key 串。
    for (patch, field) in [
        (json!({ "name": "" }), "empty name"),
        (json!({ "base_url": "ftp://x" }), "bad base_url"),
        (json!({ "models": " ,," }), "empty models"),
        (json!({ "initial_keys": [] }), "no keys"),
        (json!({ "initial_keys": [{ "key": "  " }] }), "blank key"),
    ] {
        let mut body = json!({
            "name": "ch", "base_url": "https://x.test/v1", "models": "m",
            "initial_keys": [{ "key": "k" }]
        });
        if let (Some(obj), Some(p)) = (body.as_object_mut(), patch.as_object()) {
            obj.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let (status, resp) = crate::send(
            &mut app,
            admin_req("POST", "/api/v1/admin/llm/channels", &admin_jwt, Some(body)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {resp:?}");
    }

    // 创建：keys 掩码返回，永不出明文（§11.4）。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/channels",
            &admin_jwt,
            Some(channel_body("ctl-ch", "https://up.test/v1")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let created = resp["data"].clone();
    let cid = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["keys"].as_array().unwrap().len(), 2);
    for k in created["keys"].as_array().unwrap() {
        let masked = k["masked_key"].as_str().unwrap();
        assert!(masked.contains('…'), "masked: {masked}");
        assert_ne!(k["status"], "disabled");
        assert!(
            !serde_json::to_string(&created)
                .unwrap()
                .contains("sk-admin-secret"),
            "plaintext must never appear in the DTO"
        );
    }
    assert_eq!(created["models"], "gpt-4o,gpt-4o-mini", "models normalized");

    // 列表 + 详情。
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/channels", &admin_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"].as_str() == Some(&cid))
    );
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"]["name"], "ctl-ch");

    // 更新携带 keys → 忽略（write-only 子资源，§11.5），其余字段生效。
    let mut upd = channel_body("ctl-ch-renamed", "https://up2.test/v1");
    upd["priority"] = json!(7);
    upd["keys"] = json!([{ "key": "sk-should-be-ignored" }]);
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            Some(upd),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"]["name"], "ctl-ch-renamed");
    assert_eq!(
        resp["data"]["keys"].as_array().unwrap().len(),
        2,
        "keys in update body ignored (§11.5)"
    );

    // 删除 → 404。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "DELETE",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let _ = admin_id;
}

// ── 管理端：key 池子资源 + 单 key 开关 + 渠道复活（§6.3/§11.5）────

#[tokio::test]
async fn admin_keys_subresource_toggle_and_channel_revival() {
    let (_, state) = crate::test_app().await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let (_reader_id, reader_jwt) = user_jwt(&state.pool, false).await;
    let mut app = llm_admin_app(&state);

    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/channels",
            &admin_jwt,
            Some(channel_body("pool-ch", "https://pool.test/v1")),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let cid = resp["data"]["id"].as_str().unwrap().to_owned();

    // 整体替换号池（3 把新 key）。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/admin/llm/channels/{cid}/keys"),
            &admin_jwt,
            Some(json!({ "keys": [
                { "key": "sk-pool-new-1" },
                { "key": "sk-pool-new-2" },
                { "key": "sk-pool-new-3", "max_concurrency": 5 },
            ]})),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let keys = resp["data"]["keys"].as_array().unwrap().clone();
    assert_eq!(keys.len(), 3, "pool replaced wholesale");
    assert_eq!(keys[2]["max_concurrency"], 5, "per-key concurrency stored");

    // 空 keys → 400；非 admin → 403。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/admin/llm/channels/{cid}/keys"),
            &admin_jwt,
            Some(json!({ "keys": [] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/admin/llm/channels/{cid}/keys"),
            &reader_jwt,
            Some(json!({ "keys": [{ "key": "sk-x" }] })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // 单 key 禁用（对称于 enable，§12）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/channels/{cid}/keys/0/disable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    let keys = resp["data"]["keys"].as_array().unwrap().clone();
    assert_eq!(keys[0]["status"], "disabled");
    assert_eq!(keys[0]["disabled_reason"], "manual disable");
    assert_eq!(keys[1]["status"], "active", "other keys untouched");

    // 渠道陷入 auto_disabled 后，单 key enable 复活渠道（§6.3）。
    let id = raisfast::types::snowflake_id::parse_id(&cid).unwrap();
    ch_model::update_status(&state.pool, None, id, LlmChannelStatus::AutoDisabled)
        .await
        .unwrap();
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/channels/{cid}/keys/0/enable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(
        resp["data"]["status"], "enabled",
        "auto_disabled channel revived"
    );
    assert_eq!(resp["data"]["keys"][0]["status"], "active");

    // 渠道手动启停（manual_disabled 永不自动恢复，§6.3）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/channels/{cid}/disable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/channels/{cid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(resp["data"]["status"], "manual_disabled");
}

// ── 管理端：渠道连通性测试端点（§7.5 manual）─────────────────────

#[tokio::test]
async fn admin_channel_test_endpoint_probes_and_records() {
    let (_, state) = crate::test_app().await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let mut app = llm_admin_app(&state);

    let ok_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(hdr("authorization", "Bearer sk-admin-secret-key-aaaa"))
        .and(body_partial_json(
            json!({ "model": "gpt-4o", "max_tokens": 1 }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "pong" } }]
        })))
        .expect(1)
        .mount(&ok_server)
        .await;
    let fail_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .expect(1)
        .mount(&fail_server)
        .await;

    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/channels",
            &admin_jwt,
            Some(channel_body("ok-ch", &ok_server.uri())),
        ),
    )
    .await;
    let ok_id = resp["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/channels",
            &admin_jwt,
            Some(channel_body("bad-ch", &fail_server.uri())),
        ),
    )
    .await;
    let bad_id = resp["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(status, StatusCode::OK);

    // 成功探测：test_model + max_tokens=1 + 首把 active key（mock 匹配断言）。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/channels/{ok_id}/test"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"]["ok"], true);
    assert_eq!(resp["data"]["model"], "gpt-4o");
    assert!(resp["data"]["elapsed_ms"].as_i64().is_some());
    ok_server.verify().await;

    // 探测结果落库（response_time/test_time）+ Test 来源日志。
    let row = ch_model::find_by_id(
        &state.pool,
        raisfast::types::snowflake_id::parse_id(&ok_id).unwrap(),
        None,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(row.test_time.is_some(), "test_time recorded");
    assert!(row.response_time.is_some(), "response_time recorded");
    let (items, total) = raisfast::llm::models::log::query_paged(
        &state.pool,
        None,
        &raisfast::llm::models::log::LogFilters {
            channel_id: Some(ok_id.clone()),
            ..Default::default()
        },
        1,
        10,
    )
    .await
    .unwrap();
    assert_eq!(total, 1);
    assert_eq!(items[0].source, raisfast::llm::models::log::LogSource::Test);

    // 失败探测：ok=false，detail 带上游状态码。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/channels/{bad_id}/test"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"]["ok"], false);
    assert!(
        resp["data"]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("500")),
        "detail: {resp:?}"
    );
    fail_server.verify().await;
}

// ── 管理端：模型目录 CRUD + 写时失效（§7.1 改价即时生效）─────────

#[tokio::test]
async fn admin_models_crud_with_immediate_cache_invalidation() {
    let (_, state) = crate::test_app().await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;

    // DB-backed router：invalidate_model_cache → reload 生效。
    let router = raisfast::llm::service::LlmRouter::new(state.pool.clone()).await;
    let mut s = state.clone();
    s.llm_router = router.clone();
    let mut app = llm_admin_app(&s);

    let model_name = format!("ctl-model-{}", raisfast::utils::id::new_id());

    // 创建（per_call 定价）→ 缓存即时可见。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/models",
            &admin_jwt,
            Some(json!({
                "name": model_name, "model_type": "chat",
                "price_mode": "per_call", "call_price": 0.02
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let mid = resp["data"]["id"].as_str().unwrap().to_owned();
    let info = router
        .model_info("default", &model_name)
        .expect("cached after create");
    assert_eq!(info.pricing.call_price, Some(0.02));

    // 非法 model_type / 过滤参数 → 400。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/models",
            &admin_jwt,
            Some(json!({ "name": "x", "model_type": "bogus" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/models?model_type=bogus",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 列表 + 类型过滤。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/models?model_type=chat",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["id"].as_str() == Some(&mid))
    );

    // 改价 → 缓存写时失效，无 5min 旧价窗口（§7.1）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/admin/llm/models/{mid}"),
            &admin_jwt,
            Some(json!({
                "name": model_name, "model_type": "chat",
                "price_mode": "per_call", "call_price": 0.05
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let info = router
        .model_info("default", &model_name)
        .expect("still cached");
    assert_eq!(
        info.pricing.call_price,
        Some(0.05),
        "price change is immediate"
    );

    // ── 开关端点：disable 摘出目录，enable 回归 ──
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/models/{mid}/disable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "disable via switch endpoint");
    assert!(
        router.model_info("default", &model_name).is_none(),
        "disabled model leaves the active directory"
    );
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/admin/llm/models/{mid}/enable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "enable via switch endpoint");
    assert!(
        router.model_info("default", &model_name).is_some(),
        "enabled model re-enters the active directory"
    );

    // 删除 → 目录与缓存同时摘除（唯一名，无 builtin 兜底）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "DELETE",
            &format!("/api/v1/admin/llm/models/{mid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        router.model_info("default", &model_name).is_none(),
        "delisted from cache on delete"
    );
}

// ── 用户自服务 token（§9.2/§12：unlimited 收紧、明文仅一次）──────

#[tokio::test]
async fn user_token_self_service_lifecycle() {
    let (_, state) = crate::test_app().await;
    let (owner_id, owner_jwt) = user_jwt(&state.pool, false).await;
    let (other_id, other_jwt) = user_jwt(&state.pool, false).await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let mut app = llm_admin_app(&state);

    // 创建：明文 key 恰好出现一次。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/llm/tokens",
            &owner_jwt,
            Some(json!({
                "name": "my-key", "remain_quota": 5000, "allowed_models": "gpt-4o"
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let key = resp["data"]["key"].as_str().unwrap().to_owned();
    assert!(key.starts_with("sk-"), "plaintext returned once: {key}");
    let tid = resp["data"]["token"]["id"].as_str().unwrap().to_owned();
    assert_eq!(resp["data"]["token"]["unlimited_quota"], false);

    // 列表：token 视图携带可回显的 key（api_token 模式，UI 掩码+复制）。
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/llm/tokens", &owner_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let list = resp["data"].as_array().unwrap().clone();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0]["key"].as_str(),
        Some(key.as_str()),
        "key retrievable later"
    );
    assert_eq!(list[0]["allowed_models"], "gpt-4o");

    // 非 admin 强设 unlimited → 403（§9.2 防白嫖收紧）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/llm/tokens",
            &owner_jwt,
            Some(json!({ "name": "cheat", "unlimited_quota": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "unlimited is admin-only");
    // admin 可设。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/llm/tokens",
            &admin_jwt,
            Some(json!({ "name": "internal", "unlimited_quota": true })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin may create unlimited");
    assert_eq!(resp["data"]["token"]["unlimited_quota"], true);

    // key 的哈希落库正确：能通过 relay sk- 认证（/v1/models）。
    let (_, s2) = crate::test_app().await;
    let mut relay_app = {
        let mut s = s2.clone();
        // 同一 pool 才有该 token；直接复用 state 的 pool。
        s.pool = state.pool.clone();
        axum::Router::new()
            .merge(raisfast::llm::relay::routes())
            .with_state(s)
    };
    let req = Request::builder()
        .method("GET")
        .uri("/v1/models")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let (status, resp) = crate::send(&mut relay_app, req).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "minted key passes sk- auth: {resp:?}"
    );

    // ── 可选模型列表（白名单选择器数据源，authed）──
    // 注意：本测试的 router 是默认空缓存（无渠道），此断言依赖前文
    // admin_channel_* 用例未注入；改为在 cache-only router 上验证交集逻辑：
    // 用带渠道的 router 重建 app 后查询。
    let (_, state_m) = crate::test_app().await;
    let mut ch = raisfast::llm::models::channel::LlmChannel {
        id: SnowflakeId(1),
        tenant_id: Some("default".to_owned()),
        name: "m-ch".to_owned(),
        provider: "openai".to_owned(),
        base_url: "http://m.test".to_owned(),
        api_keys: serde_json::json!([]),
        key_mode: ch_model::LlmKeyMode::Polling,
        status: LlmChannelStatus::Enabled,
        models: "gpt-4o,gpt-4o-mini,not-in-directory".to_owned(),
        model_mapping: None,
        priority: 0,
        weight: 0,
        channel_groups: "default".to_owned(),
        auto_ban: true,
        param_override: None,
        header_override: None,
        config: None,
        used_quota: 0,
        cost_mode: ch_model::LlmCostMode::Usage,
        cost_discount: 1.0,
        monthly_cost: None,
        test_model: None,
        test_time: None,
        response_time: None,
        created_at: raisfast::utils::tz::now_utc(),
        updated_at: raisfast::utils::tz::now_utc(),
    };
    ch.api_keys = ch_model::keys_value(&[raisfast::llm::models::channel::LlmKeyEntry {
        key: "sk-x".to_owned(),
        status: LlmKeyStatus::Active,
        disabled_reason: None,
        disabled_at: None,
        max_concurrency: None,
    }]);
    let mut s_m = state_m.clone();
    s_m.pool = state.pool.clone();
    s_m.llm_router = raisfast::llm::service::LlmRouter::from_cache_for_test(
        raisfast::llm::cache::ChannelCache::build(vec![ch], vec![]),
    );
    let mut app_m = llm_admin_app(&s_m);
    let (status, resp) = crate::send(
        &mut app_m,
        admin_req("GET", "/api/v1/llm/models", &owner_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let names = |key: &str| -> Vec<String> {
        resp["data"][key]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|m| m.as_str().map(str::to_owned))
            .collect()
    };
    // 可选 = 渠道模型 ∩ 激活目录；未定价 = 目录外（选择器置灰展示）。
    assert_eq!(names("models"), vec!["gpt-4o", "gpt-4o-mini"]);
    assert_eq!(names("unpriced"), vec!["not-in-directory"]);

    // 更新自己的 token（改名/额度/禁用）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/llm/tokens/{tid}"),
            &owner_jwt,
            Some(json!({ "name": "renamed", "remain_quota": 999, "enabled": false })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/llm/tokens", &owner_jwt, None),
    )
    .await;
    let row = &resp["data"][0];
    assert_eq!(row["name"], "renamed");
    assert_eq!(row["remain_quota"], 999.0);
    assert_eq!(row["status"], "disabled");

    // 他人更新 → 403（所有权）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            &format!("/api/v1/llm/tokens/{tid}"),
            &other_jwt,
            Some(json!({ "name": "hijack" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "ownership enforced");

    // 删除自己的 token。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "DELETE",
            &format!("/api/v1/llm/tokens/{tid}"),
            &owner_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/llm/tokens", &owner_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"].as_array().unwrap().len(), 0);
    let _ = (owner_id, other_id);
}

// ── Admin token 管理：代建（指定用户）/ 列表回填用户名 / 开关 ──────

#[tokio::test]
async fn admin_token_administration_and_toggle() {
    let (_, state) = crate::test_app().await;
    let (owner_id, owner_jwt) = user_jwt(&state.pool, false).await;
    let (_other_id, other_jwt) = user_jwt(&state.pool, false).await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let owner = raisfast::models::user::find_by_id(&state.pool, owner_id, None)
        .await
        .unwrap()
        .expect("owner exists");
    let mut app = llm_admin_app(&state);

    // 非 admin 访问 admin 列表 → 403。
    let (status, _) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/tokens", &owner_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "admin list is admin-only");

    // admin 代建：指定 user_id → token 归属该用户。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/tokens",
            &admin_jwt,
            Some(json!({
                "name": "for-owner", "user_id": owner_id.to_string(), "remain_quota": 100
            })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let tid = resp["data"]["token"]["id"].as_str().unwrap().to_owned();
    assert_eq!(
        resp["data"]["token"]["user_id"].as_str(),
        Some(owner_id.to_string().as_str()),
        "token belongs to the target user"
    );

    // 不存在的 user_id → 400。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            "/api/v1/admin/llm/tokens",
            &admin_jwt,
            Some(json!({ "name": "ghost", "user_id": "4194304" })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown user rejected");

    // admin 列表：按 tid 过滤（共享库不做全量断言），username 已回填。
    let find_row = |resp: &Value| -> Value {
        resp["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"].as_str() == Some(tid.as_str()))
            .cloned()
            .expect("token present in list")
    };
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/tokens", &admin_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        find_row(&resp)["username"].as_str(),
        Some(owner.username.as_str()),
        "owner username resolved"
    );

    // username 子串过滤：命中 / 未命中 / 精确 user_id。
    let prefix = &owner.username[..owner.username.len().min(8)];
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/tokens?username={prefix}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !resp["data"].as_array().unwrap().is_empty(),
        "username substring matches"
    );
    assert!(
        resp["data"].as_array().unwrap().iter().all(|r| r
            ["username"]
            .as_str()
            .is_some_and(|n| n.to_lowercase().contains(&prefix.to_lowercase()))),
        "every returned row matches the username filter"
    );
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/tokens?username=no-such-user-xyz",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        resp["data"].as_array().unwrap().len(),
        0,
        "unknown username yields empty list"
    );
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/tokens?username={owner_id}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        resp["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"].as_str() == Some(tid.as_str())),
        "exact user_id filter matches"
    );

    // 所有者 disable（开关端点，authed）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/llm/tokens/{tid}/disable"),
            &owner_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/llm/tokens", &owner_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(find_row(&resp)["status"], "disabled");

    // 他人 enable → 403（所有权）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/llm/tokens/{tid}/enable"),
            &other_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "ownership enforced on toggle"
    );

    // admin 跨所有权 enable + 删除（旁路所有权）。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "POST",
            &format!("/api/v1/llm/tokens/{tid}/enable"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin bypasses ownership");
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "DELETE",
            &format!("/api/v1/llm/tokens/{tid}"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin may delete any token");
}

// ── Logs 用户名过滤（logs 页）───────────────────────────────────

#[tokio::test]
async fn admin_logs_username_filter() {
    let (_, state) = crate::test_app().await;
    let (owner_id, _owner_jwt) = user_jwt(&state.pool, false).await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let owner = raisfast::models::user::find_by_id(&state.pool, owner_id, None)
        .await
        .unwrap()
        .expect("owner exists");

    // 唯一 model 名做行标记（共享库不做全量断言）。
    let marker = format!("usr-flt-{}", raisfast::utils::id::new_id());
    raisfast::llm::models::log::insert_log(
        &state.pool,
        raisfast::llm::models::log::NewLog {
            user_id: Some(owner_id),
            source: raisfast::llm::models::log::LogSource::Relay,
            model_name: marker.clone(),
            quota: raisfast::types::quota::Quota(1_000_000),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let mut app = llm_admin_app(&state);
    let has_marker = |resp: &Value| -> bool {
        resp["data"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["model_name"].as_str() == Some(marker.as_str()))
    };

    // 用户名子串过滤 → 命中。
    let prefix = &owner.username[..owner.username.len().min(8)];
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/logs?username={prefix}&page_size=100"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert!(has_marker(&resp), "username substring filter hits");
    // 精确 user_id 过滤同样命中。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            &format!("/api/v1/admin/llm/logs?username={owner_id}&page_size=100"),
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(has_marker(&resp), "exact user id filter hits");
    // 未命中用户名 → 空。
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs?username=no-such-user-xyz",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["data"]["total"], 0, "unknown username yields empty");
}

// ── 健康 cron（§7.5/§6.3）：探活恢复仅作用于被测 key ─────────────

#[tokio::test]
async fn health_cron_recovers_only_probed_ok_channels() {
    let (_, state) = crate::test_app().await;
    let ok_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{ "message": { "content": "pong" } }]
        })))
        .expect(1)
        .mount(&ok_server)
        .await;

    // A：auto_disabled + key 禁用，指向上游已恢复的 mock → 应复活。
    let a = insert_channel(&state.pool, &ok_server.uri(), "sk-recovered").await;
    ch_model::update_key_status(
        &state.pool,
        None,
        a.id,
        0,
        LlmKeyStatus::Disabled,
        Some("arrears"),
    )
    .await
    .unwrap();
    ch_model::update_status(&state.pool, None, a.id, LlmChannelStatus::AutoDisabled)
        .await
        .unwrap();

    // B：auto_disabled + key 禁用，指向死端口 → 保持禁用。
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_uri = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let b = insert_channel(&state.pool, &dead_uri, "sk-still-dead").await;
    ch_model::update_key_status(
        &state.pool,
        None,
        b.id,
        0,
        LlmKeyStatus::Disabled,
        Some("arrears"),
    )
    .await
    .unwrap();
    ch_model::update_status(&state.pool, None, b.id, LlmChannelStatus::AutoDisabled)
        .await
        .unwrap();

    let handler = raisfast::worker::handlers::llm_health::LlmHealthHandler::new(
        state.pool.clone(),
        state.config.clone(),
    );
    use raisfast::worker::JobHandler as _;
    let job = raisfast::worker::Job::Custom {
        job_type: "llm_channel_health".to_owned(),
        payload: serde_json::Value::Null,
    };
    handler.handle(&job).await.expect("health sweep");
    ok_server.verify().await;

    // A：仅被测 key 复活 + 渠道回 enabled（§6.3）。
    let row = ch_model::find_by_id(&state.pool, a.id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, LlmChannelStatus::Enabled, "recovered");
    let entries = ch_model::parse_keys(&row);
    assert_eq!(entries[0].status, LlmKeyStatus::Active);
    assert_eq!(
        entries[0].disabled_reason.as_deref(),
        None,
        "recovery clears the disable reason"
    );

    // B：探活失败 → 维持 auto_disabled + key 禁用。
    let row = ch_model::find_by_id(&state.pool, b.id, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, LlmChannelStatus::AutoDisabled, "still down");
    assert_eq!(ch_model::parse_keys(&row)[0].status, LlmKeyStatus::Disabled);
}

// ── 计费分组倍率端点（pricing.md §2）────────────────────────────

#[tokio::test]
async fn admin_group_ratios_crud_and_validation() {
    let (_, state) = crate::test_app().await;
    let (_admin_id, admin_jwt) = user_jwt(&state.pool, true).await;
    let mut app = llm_admin_app(&state);

    // 默认值（seed）：default = 1.0。
    let (status, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/group-ratios", &admin_jwt, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"]["ratios"]["default"], 1.0);

    // 替换为两个分组。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            "/api/v1/admin/llm/group-ratios",
            &admin_jwt,
            Some(json!({ "ratios": { "default": 1.0, "vip": 1.5 } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, resp) = crate::send(
        &mut app,
        admin_req("GET", "/api/v1/admin/llm/group-ratios", &admin_jwt, None),
    )
    .await;
    assert_eq!(resp["data"]["ratios"]["vip"], 1.5);

    // 非法输入：空 map / 负系数 → 400。
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            "/api/v1/admin/llm/group-ratios",
            &admin_jwt,
            Some(json!({ "ratios": {} })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "PUT",
            "/api/v1/admin/llm/group-ratios",
            &admin_jwt,
            Some(json!({ "ratios": { "bad": -1.0 } })),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Daily logs stats endpoint: zero-filled window + USD charge/cost/profit.
#[tokio::test]
async fn admin_logs_stats_daily_totals() {
    let (_, state) = crate::test_app().await;
    let (admin_id, admin_jwt) = user_jwt(&state.pool, true).await;

    // Two relay logs today: charge 800,000 quota ($0.8), cost 320,000 ($0.32).
    for (model, quota, cost) in [
        ("gpt-4o", 300_000i64, 120_000i64),
        ("gpt-4o-mini", 500_000, 200_000),
    ] {
        raisfast::llm::models::log::insert_log(
            &state.pool,
            raisfast::llm::models::log::NewLog {
                tenant_id: Some("default".to_owned()),
                user_id: Some(admin_id),
                source: raisfast::llm::models::log::LogSource::Relay,
                model_name: model.to_owned(),
                prompt_tokens: 100,
                completion_tokens: 50,
                quota: raisfast::types::quota::Quota(quota),
                cost_quota: raisfast::types::quota::Quota(cost),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    let mut app = llm_admin_app(&state);
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?days=7",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    // Day window is a continuous 7 days regardless of logged days.
    assert_eq!(resp["data"]["group_by"], "day");
    assert_eq!(resp["data"]["data"].as_array().unwrap().len(), 7);
    assert_eq!(resp["data"]["totals"]["requests"], 2);
    assert_eq!(resp["data"]["totals"]["prompt_tokens"], 200);
    assert_eq!(resp["data"]["totals"]["completion_tokens"], 100);
    assert_eq!(resp["data"]["totals"]["charge_usd"], 0.8);
    assert_eq!(resp["data"]["totals"]["cost_usd"], 0.32);
    assert_eq!(resp["data"]["totals"]["profit_usd"], 0.48);

    // Group by model: sorted by charge DESC (gpt-4o-mini $0.5 first).
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?group_by=model&days=7",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let rows = resp["data"]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["label"], "gpt-4o-mini");
    assert_eq!(rows[0]["charge_usd"], 0.5);
    assert_eq!(rows[1]["label"], "gpt-4o");
    assert_eq!(rows[1]["charge_usd"], 0.3);
    // Totals identical across groupings.
    assert_eq!(resp["data"]["totals"]["charge_usd"], 0.8);

    // Group by user: label resolves to the username.
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?group_by=user&days=7",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    let rows = resp["data"]["data"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["charge_usd"], 0.8);
    assert_ne!(rows[0]["label"], "");

    // Explicit window that excludes today → empty totals but still a valid day series.
    let (status, resp) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?start=2020-01-01&end=2020-01-03",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {resp:?}");
    assert_eq!(resp["data"]["totals"]["requests"], 0);
    assert_eq!(resp["data"]["data"].as_array().unwrap().len(), 3);

    // Invalid group_by / inverted range → 400.
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?group_by=bogus",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = crate::send(
        &mut app,
        admin_req(
            "GET",
            "/api/v1/admin/llm/logs/stats?start=2026-02-01&end=2026-01-01",
            &admin_jwt,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Plain-text single-key channel row directly via the model layer.
async fn insert_channel(
    pool: &raisfast::db::Pool,
    base_url: &str,
    key: &str,
) -> raisfast::llm::models::channel::LlmChannel {
    let entries = vec![raisfast::llm::models::channel::LlmKeyEntry {
        key: key.to_owned(),
        status: LlmKeyStatus::Active,
        disabled_reason: None,
        disabled_at: None,
        max_concurrency: None,
    }];
    ch_model::create_channel(
        pool,
        None,
        ch_model::NewChannel {
            name: format!("cron-{}", raisfast::utils::id::new_id()),
            provider: "openai".to_owned(),
            base_url: base_url.to_owned(),
            api_keys: ch_model::keys_value(&entries),
            key_mode: ch_model::LlmKeyMode::Polling,
            models: "gpt-4o".to_owned(),
            model_mapping: None,
            priority: 0,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            cost_mode: ch_model::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
        },
    )
    .await
    .unwrap()
}
