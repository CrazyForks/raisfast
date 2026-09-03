#![cfg(feature = "db-postgres")]
//! End-to-end smoke for AgentService against a live PostgreSQL + a local mock
//! OpenAI-compatible HTTP provider.
//!
//! Verifies the full turn lifecycle without any external LLM:
//!   two-phase persistence (user first, then assistant + turn:meta),
//!   busy flag set/release, per-call usage on the assistant row, cursor advance.
//!
//! ```bash
//! RAISFAST_TEST_DB_URL=postgres://postgres:postgres@localhost:5432/raisfast_test \
//!   cargo test -p raisfast --test ai_service_smoke --no-default-features \
//!     --features "db-postgres plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system integration-stream integration-imap"
//! ```

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use raisfast::agent::models::ai_message::AiMessage;
use raisfast::agent::service as ai_service;
use raisfast::config::app::AiConfig;
use sqlx::postgres::{PgPool, PgPoolOptions};

fn test_pool() -> PgPool {
    let url = std::env::var("RAISFAST_TEST_DB_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/raisfast_test".into());
    PgPoolOptions::new()
        .max_connections(2)
        .connect_lazy(&url)
        .expect("test pool")
}

fn tenant() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("t_ai_svc_{n}")
}

fn meta_hash(row: &AiMessage) -> String {
    let value: serde_json::Value =
        serde_json::from_str(&row.content).expect("turn:meta content is JSON");
    value["system_hash"]
        .as_str()
        .expect("system_hash present")
        .to_string()
}

/// Boot a mock `/v1/chat/completions` returning a fixed text answer.
async fn mock_llm_server() -> String {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(|| async {
            Json(serde_json::json!({
                "choices": [{
                    "message": { "role": "assistant", "content": "你好，小明（mock 回复）" }
                }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 7 }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

#[tokio::test]
async fn agent_service_turn_end_to_end() {
    let pool = test_pool();
    let tenant_id = tenant();
    let base_url = mock_llm_server().await;

    let agent = ai_service::create_agent(
        &pool,
        Some(tenant_id.clone()),
        None,
        "helper".into(),
        "你是能调用工具的助手。".into(),
        "openai_compat".into(),
        "mock-model".into(),
        None,
        vec![],
        true,
        None,
    )
    .await
    .expect("create agent");
    let owner = agent.id;
    let session =
        ai_service::create_session(&pool, Some(tenant_id.clone()), agent.id, owner, "smoke")
            .await
            .expect("create session");

    let ai = AiConfig {
        enabled: true,
        base_url: Some(base_url),
        api_key: Some("test-key".into()),
        model: None,
        timeout_secs: 10,
        broadcast_events: false,
    };

    let result = ai_service::run_turn(&pool, &ai, &agent, session.id, "你好，记住我叫小明")
        .await
        .expect("run turn");
    assert!(result.text.contains("小明"), "mock answer present");
    assert_eq!(result.iterations, 1);
    assert_eq!(result.tool_calls_made, 0);
    assert!(
        result.messages_appended >= 1,
        "assistant row appended (turn:meta is separate)"
    );

    // busy flag released + cursor advanced
    let sess = ai_service::find_session(&pool, session.id, Some(&tenant_id))
        .await
        .expect("find session");
    assert_eq!(sess.status, "open");
    assert!(
        sess.last_seq >= 3,
        "cursor advanced past user/assistant/turn-meta"
    );

    // transcript rows: user, assistant(usage), meta
    let rows = ai_service::list_messages(&pool, session.id, Some(&tenant_id), None, 10)
        .await
        .expect("list messages");
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].role, "user");
    assert_eq!(rows[1].role, "assistant");
    let usage = rows[1].usage.clone().expect("assistant usage recorded");
    assert_eq!(usage["input"], 12);
    assert_eq!(usage["output"], 7);
    assert_eq!(rows[2].role, "meta");
    assert_eq!(rows[2].kind, "turn:meta");
    let first_hash = meta_hash(&rows[2]);

    // busy released: a sequential second turn runs fine (status back to open)
    let second = ai_service::run_turn(&pool, &ai, &agent, session.id, "第二条").await;
    assert!(second.is_ok(), "session is open again, second turn runs");
    assert!(second.unwrap().text.contains("小明"));

    // system_hash is stable across identical config turns
    let rows2 = ai_service::list_messages(&pool, session.id, Some(&tenant_id), None, 10)
        .await
        .expect("list messages after second turn");
    let last_meta = rows2
        .iter()
        .rfind(|m| m.kind == "turn:meta")
        .expect("meta row");
    assert_eq!(
        meta_hash(last_meta),
        first_hash,
        "system_hash stable across turns"
    );

    // cleanup
    raisfast::agent::models::ai_agent::delete_agent(&pool, agent.id, Some(&tenant_id))
        .await
        .expect("delete agent");
}
