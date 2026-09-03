#![cfg(feature = "db-postgres")]
//! Runtime smoke for the AI agent DB layer against a live PostgreSQL.
//!
//! No LLM involved — validates ai_* tables, crud inserts, JSON/bool binds and
//! the hand-written queries on a real backend.
//!
//! ```bash
//! RAISFAST_DB=postgres RAISFAST_TEST_DB_URL=postgres://postgres:postgres@localhost:5432/raisfast_test \
//!   cargo test -p raisfast --test ai_runtime --no-default-features \
//!     --features "db-postgres plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system integration-stream integration-imap"
//! ```

use raisfast::agent::memory_sql::ScopedMemory;
use raisfast::agent::models::{ai_agent, ai_memory, ai_message, ai_session};
use raisfast_agent::Memory;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::{SystemTime, UNIX_EPOCH};

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
    format!("t_ai_{n}")
}

#[tokio::test]
async fn ai_models_roundtrip() {
    let pool = test_pool();
    let tenant_id = tenant();

    // agent
    let agent = ai_agent::create_agent(
        &pool,
        Some(&tenant_id),
        None,
        "helper",
        "you are a test agent",
        "openai_compat",
        "test-model",
        None,
        vec!["memory_store".to_string()],
        true,
        None,
    )
    .await
    .expect("create agent");
    let fetched = ai_agent::find_agent_by_id(&pool, agent.id, Some(&tenant_id))
        .await
        .expect("find agent");
    assert_eq!(fetched.name, "helper");

    // session + status
    let session = ai_session::create_session(&pool, Some(&tenant_id), agent.id, agent.id, "smoke")
        .await
        .expect("create session");
    ai_session::set_session_status(&pool, session.id, Some(&tenant_id), "running")
        .await
        .expect("set running");
    ai_session::set_session_status(&pool, session.id, Some(&tenant_id), "open")
        .await
        .expect("set open");

    // messages with JSON/bool binds + cursor
    let mut seq = ai_message::next_seq(&pool, session.id, Some(&tenant_id))
        .await
        .expect("next seq");
    assert_eq!(seq, 1, "fresh session starts at seq 1");
    ai_message::append_message(
        &pool,
        Some(&tenant_id),
        &ai_message::AiMessageIn {
            session_id: session.id,
            seq,
            role: "user".into(),
            kind: "chat".into(),
            content: "hi".into(),
            tool_calls: None,
            tool_call_id: None,
            tool_name: None,
            tool_success: None,
            tool_error: None,
            tool_elapsed_ms: None,
            tool_truncated: None,
            reasoning_content: None,
            usage: None,
        },
    )
    .await
    .expect("append user");
    seq += 1;
    ai_message::append_message(
        &pool,
        Some(&tenant_id),
        &ai_message::AiMessageIn {
            session_id: session.id,
            seq,
            role: "assistant".into(),
            kind: "assistant_tool_calls".into(),
            content: String::new(),
            tool_calls: Some(serde_json::json!([{
                "id": "call_1", "name": "memory_store",
                "arguments": "{\"key\":\"nickname\",\"content\":\"小明\"}"
            }])),
            tool_call_id: None,
            tool_name: None,
            tool_success: None,
            tool_error: None,
            tool_elapsed_ms: None,
            tool_truncated: None,
            reasoning_content: None,
            usage: Some(serde_json::json!({"input": 10, "output": 5})),
        },
    )
    .await
    .expect("append assistant tool call");
    seq += 1;
    ai_message::append_message(
        &pool,
        Some(&tenant_id),
        &ai_message::AiMessageIn {
            session_id: session.id,
            seq,
            role: "tool".into(),
            kind: "tool_result".into(),
            content: "已记住 nickname".into(),
            tool_calls: None,
            tool_call_id: Some("call_1".into()),
            tool_name: Some("memory_store".into()),
            tool_success: Some(true),
            tool_error: None,
            tool_elapsed_ms: Some(12),
            tool_truncated: None,
            reasoning_content: None,
            usage: None,
        },
    )
    .await
    .expect("append tool result");

    let rows = ai_message::list_messages_after(&pool, session.id, Some(&tenant_id), Some(0), 10)
        .await
        .expect("list messages");
    assert_eq!(rows.len(), 3, "user + assistant + tool");
    assert_eq!(rows[0].content, "hi");
    assert_eq!(
        rows[1].usage,
        Some(serde_json::json!({"input": 10, "output": 5}))
    );
    assert_eq!(rows[2].tool_name.as_deref(), Some("memory_store"));

    ai_session::advance_last_seq(&pool, session.id, Some(&tenant_id), seq)
        .await
        .expect("advance cursor");
    let sess = ai_session::find_session_by_id(&pool, session.id, Some(&tenant_id))
        .await
        .expect("find session again");
    assert_eq!(sess.last_seq, seq);

    // memory via ScopedMemory + keyword recall
    let memory = ScopedMemory::new(pool.clone(), Some(tenant_id.clone()), agent.id);
    memory
        .store("nickname", "小明")
        .await
        .expect("memory store");
    let hits = memory.recall(Some("小明"), 5).await.expect("memory recall");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].key, "nickname");
    let recent = ai_memory::recall_memories(&pool, agent.id, Some(&tenant_id), None, 5)
        .await
        .expect("model recall");
    assert!(!recent.is_empty());
    assert!(memory.forget("nickname").await.expect("memory forget"));
    assert!(
        !memory
            .forget("nickname")
            .await
            .expect("memory forget again")
    );

    // cleanup
    ai_agent::delete_agent(&pool, agent.id, Some(&tenant_id))
        .await
        .expect("delete agent");
}
