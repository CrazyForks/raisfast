//! M5-A instance test: a real agent with an enabled skill talks to a mock LLM;
//! we assert what actually reached the model (system skills section, tool list).
//!
//! Runs serially on PG like other AI tests.

#![cfg(feature = "db-postgres")]

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use raisfast::agent::service as ai_service;
use raisfast::config::app::AiConfig;
use serde_json::json;
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
    format!("t_sk_{n}")
}

/// Boot a mock OpenAI-compatible server that records every request body.
async fn mock_llm(requests: Arc<Mutex<Vec<String>>>) -> String {
    let app = axum::Router::new().route(
        "/v1/chat/completions",
        axum::routing::post(move |Json(value): Json<serde_json::Value>| async move {
            requests.lock().unwrap().push(value.to_string());
            Json(json!({
                "choices": [{ "message": { "role": "assistant", "content": "ok" } }],
                "usage": { "prompt_tokens": 10, "completion_tokens": 2 }
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/v1")
}

fn write_skill(root: &std::path::Path, tenant: &str, name: &str, body: &str) {
    let dir = root.join("tenants").join(tenant).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let mut f = std::fs::File::create(dir.join("SKILL.md")).unwrap();
    let _ = f.write_all(
        format!(
            "---\nname: {name}\ndescription: Use when the user asks for {name}.\n---\n{body}\n"
        )
        .as_bytes(),
    );
}

fn ai() -> AiConfig {
    // base_url is patched below after starting the mock; this is only a placeholder
    // holder pattern to avoid constructing in each branch.
    AiConfig {
        enabled: true,
        base_url: None,
        api_key: Some("k".into()),
        model: None,
        timeout_secs: 10,
        broadcast_events: false,
        ..AiConfig::default()
    }
}

#[tokio::test]
async fn agent_with_skill_full_and_compact() {
    // Point skill loading at a throwaway dir (single-threaded test binary).
    let skills_dir = tempfile::tempdir().unwrap();
    let skills_path = skills_dir.path().to_str().unwrap().to_string();
    unsafe {
        std::env::set_var("RAISFAST_SKILLS_DIR", &skills_path);
    }

    let pool = test_pool();
    let t = tenant();
    write_skill(
        skills_dir.path(),
        &t,
        "ship",
        "SHIP_STEPS\n1. run tests\n2. deploy",
    );
    let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let base = mock_llm(requests.clone()).await;

    // Full mode (default)
    let mut a = ai();
    a.base_url = Some(base.clone());
    let agent_full = ai_service::create_agent(
        &pool,
        Some(t.clone()),
        None,
        "skillful".into(),
        "you help ops".into(),
        "openai_compat".into(),
        "mock".into(),
        None,
        vec![],
        true,
        Some(json!({ "skill_bundles": ["ship"] })),
    )
    .await
    .unwrap();
    let sess_full =
        ai_service::create_session(&pool, Some(t.clone()), agent_full.id, agent_full.id, "x")
            .await
            .unwrap();
    ai_service::run_turn(&pool, &a, &agent_full, sess_full.id, "please ship today")
        .await
        .unwrap();
    let first = requests.lock().unwrap().last().cloned().unwrap();
    assert!(
        first.contains("## Available Skills"),
        "skills section present"
    );
    assert!(first.contains("ship"), "skill listed");
    assert!(
        first.contains("SHIP_STEPS"),
        "Full mode inlines instructions"
    );
    assert!(
        !first.contains("\"read_skill\""),
        "no read_skill tool needed in Full"
    );

    // Compact mode
    let agent_compact = ai_service::create_agent(
        &pool,
        Some(t.clone()),
        None,
        "skillful-compact".into(),
        "you help ops".into(),
        "openai_compat".into(),
        "mock".into(),
        None,
        vec![],
        true,
        Some(json!({ "skill_bundles": ["ship"], "skills_mode": "compact" })),
    )
    .await
    .unwrap();
    let sess_compact = ai_service::create_session(
        &pool,
        Some(t.clone()),
        agent_compact.id,
        agent_compact.id,
        "x",
    )
    .await
    .unwrap();
    ai_service::run_turn(&pool, &a, &agent_compact, sess_compact.id, "ship it")
        .await
        .unwrap();
    let second = requests.lock().unwrap().last().cloned().unwrap();
    assert!(
        second.contains("read_skill"),
        "read_skill hint/tool present in Compact"
    );
    assert!(
        second.contains("Skill summaries are preloaded"),
        "compact preamble"
    );
    assert!(
        !second.contains("SHIP_STEPS"),
        "Compact does not inline instructions"
    );

    // Fixture regression: import Claude-style and zeroclaw-style SKILL.md
    // samples into the temp store and assert they load with correct metadata.
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skills");
    for entry in std::fs::read_dir(&fixtures).unwrap() {
        let entry = entry.unwrap();
        if !entry.path().is_dir() {
            continue;
        }
        raisfast::agent::skills::import::import_skill(
            &entry.path(),
            skills_dir.path(),
            "platform",
            None,
            false,
        )
        .expect("fixture skill importable");
    }
    let all = raisfast::agent::skills::load_skills(skills_dir.path(), Some(&t), &["*".to_string()]);
    let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"ship"), "manual skill loaded");
    assert!(
        names.contains(&"code-review"),
        "claude-style fixture imported"
    );
    assert!(
        names.contains(&"zeroclaw-ops"),
        "zeroclaw-style fixture imported"
    );
    let zops = all.iter().find(|s| s.name == "zeroclaw-ops").unwrap();
    assert!(zops.always, "zeroclaw `always: true` parsed");
    assert!(
        zops.description.contains("ZeroClaw"),
        "frontmatter description kept"
    );

    // cleanup
    raisfast::agent::models::ai_agent::delete_agent(&pool, agent_full.id, Some(&t))
        .await
        .unwrap();
    raisfast::agent::models::ai_agent::delete_agent(&pool, agent_compact.id, Some(&t))
        .await
        .unwrap();
}
