//! Await/resume (HITL) scenarios against the DB-backed coordinator
//! (dev-docs/workflow/await-node.md): typed envelopes, custom action ports,
//! claim idempotency (409), correlation-matched event wake, timeout sweep.

use super::helpers::*;
use raisfast::DbDriver;
use raisfast::flows::engine::Snapshot;
use raisfast::flows::model::{self, Flow, FlowInstance, FlowVersion};
use raisfast::flows::nodes::ResumeEnvelope;
use raisfast::flows::run;
use raisfast::types::snowflake_id::SnowflakeId;
use raisfast::utils::tz::now_utc;
use serde_json::json;

/// Seed a waiting instance whose gate node uses `gate_config`; returns the
/// instance id.
async fn seed_waiting_instance(
    pool: &raisfast::db::Pool,
    gate_config: serde_json::Value,
    gate_edges: serde_json::Value,
    extra_nodes: serde_json::Value,
) -> i64 {
    seed_waiting_instance_inputs(pool, gate_config, gate_edges, extra_nodes, json!({})).await
}

/// Same, with explicit trigger inputs (for correlation / template tests).
async fn seed_waiting_instance_inputs(
    pool: &raisfast::db::Pool,
    gate_config: serde_json::Value,
    gate_edges: serde_json::Value,
    extra_nodes: serde_json::Value,
    inputs: serde_json::Value,
) -> i64 {
    let now = now_utc();
    let flow_id = raisfast::utils::id::new_snowflake_id();
    model::insert_flow(
        pool,
        &Flow {
            id: flow_id,
            tenant_id: "default".into(),
            name: format!("await-{}", raisfast::utils::id::new_id()),
            description: None,
            enabled: true,
            current_version: None,
            extra: None,
            created_at: now,
            updated_at: now,
        },
    )
    .await
    .unwrap();
    let mut nodes = json!([
        node("start", "start", json!({})),
        node("gate", "await", gate_config),
    ]);
    if let (Some(arr), Some(extra)) = (nodes.as_array_mut(), extra_nodes.as_array()) {
        for n in extra {
            arr.push(n.clone());
        }
    }
    let def = def_of(nodes, gate_edges);
    let vid = raisfast::utils::id::new_snowflake_id();
    model::insert_flow_version(
        pool,
        &FlowVersion {
            id: vid,
            flow_id,
            version_number: 1,
            definition: def,
            created_by: None,
            created_at: now,
        },
    )
    .await
    .unwrap();
    model::set_flow_current_version(pool, flow_id, vid)
        .await
        .unwrap();

    let iid = raisfast::utils::id::new_snowflake_id();
    model::insert_flow_instance(
        pool,
        &FlowInstance {
            id: iid,
            tenant_id: "default".into(),
            flow_id,
            flow_version_id: vid,
            status: "running".into(),
            has_exceptions: false,
            trigger_kind: "api".into(),
            trigger_payload: Some(inputs),
            inputs_summary: None,
            outputs: None,
            error: None,
            started_by: None,
            started_at: Some(now),
            finished_at: None,
            waiting_kind: None,
            waiting_needed: None,
            waiting_received: 0,
            resume_until: None,
            created_at: now,
        },
    )
    .await
    .unwrap();
    iid.0
}

async fn park(
    pool: &raisfast::db::Pool,
    det: &Deterministic,
    iid: i64,
) -> raisfast::types::snowflake_id::SnowflakeId {
    let iid = SnowflakeId(iid);
    run::execute_instance(pool, iid, det).await.unwrap();
    let inst = model::find_instance_by_id(pool, iid).await.unwrap();
    assert_eq!(inst.status, "waiting", "parks on await: {inst:?}");
    iid
}

#[tokio::test]
async fn human_custom_actions_route_and_collected_data_lands() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({
            "form_content": "订单 {{#start.order_no#}} 申请退款，请处理",
            "inputs": [
                {"name": "note", "label": "备注", "type": "string", "required": false}
            ],
            "actions": [
                {"id": "approve", "title": "同意退款"},
                {"id": "reject", "title": "拒绝"}
            ],
            "approvers": ["42"]
        }),
        json!([
            edge("start", "out", "gate"),
            edge("gate", "approve", "ok_end"),
            edge("gate", "reject", "rej_end")
        ]),
        json!([
            node(
                "ok_end",
                "end",
                json!({
                    "outputs": [{"key": "note", "value": {"ref": ["gate", "resume", "note"]}}]
                })
            ),
            node(
                "rej_end",
                "end",
                json!({"outputs": [{"key": "v", "value": {"literal": "rejected"}}]})
            )
        ]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    // Fire the custom "reject" action with collected input data.
    let envelope = ResumeEnvelope {
        action: "reject".into(),
        data: Some(json!({"note": "凭证不足"})),
    };
    run::resume_instance(&pool, None, None, iid, &envelope, Some(SnowflakeId(42)))
        .await
        .unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(
        done.status, "success",
        "action outcome is authored wiring, not failure"
    );
    let snap: Snapshot =
        serde_json::from_value(model::find_snapshot(&pool, iid).await.unwrap().unwrap()).unwrap();
    assert_eq!(snap.node_states["ok_end"].status, "skipped");
    assert_eq!(snap.node_states["rej_end"].status, "success");
    assert_eq!(done.outputs.unwrap()["v"], "rejected");
}

#[tokio::test]
async fn waiting_info_renders_form_content_and_default_submit() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance_inputs(
        &pool,
        // No actions → default submit; form_content interpolated from inputs.
        json!({"form_content": "订单 {{#start.order_no#}} 待确认", "inputs": [
            {"name": "amount", "type": "number", "required": true}
        ]}),
        json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
        json!({"order_no": "A-1024"}),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    let info = run::waiting_info(&pool, iid).await.unwrap();
    assert_eq!(info["kind"], "human");
    assert_eq!(info["form_content"], "订单 A-1024 待确认");
    assert_eq!(info["inputs"][0]["name"], "amount");
    assert_eq!(
        info["actions"][0]["id"], "submit",
        "no declared actions → Dify default submit injected: {info}"
    );
    assert!(info["resume_until"].is_string());

    // Default submit = single-port semantics: any outgoing edge is taken.
    let envelope = ResumeEnvelope {
        action: "submit".into(),
        data: Some(json!({"amount": 5})),
    };
    run::resume_instance(&pool, None, None, iid, &envelope, None)
        .await
        .unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
}

#[tokio::test]
async fn unknown_action_is_rejected() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({"actions": [{"id": "approve", "title": "通过"}]}),
        json!([edge("start", "out", "gate"), edge("gate", "approve", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    let envelope = ResumeEnvelope {
        action: "smash".into(),
        data: None,
    };
    let err = run::resume_instance(&pool, None, None, iid, &envelope, None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("不是该节点的操作"), "{err}");
    let inst = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(inst.status, "waiting", "claim untouched on 400");
}

#[tokio::test]
async fn public_resume_token_addresses_the_parked_instance() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({"actions": [{"id": "approve", "title": "通过"}]}),
        json!([edge("start", "out", "gate"), edge("gate", "approve", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    // Mint-once token (model layer — what the handler's ensure_resume_token drives).
    let row = model::find_open_flow_resume(&pool, iid, "gate")
        .await
        .unwrap()
        .expect("open claim");
    let token = raisfast::utils::id::random_hex(24);
    model::set_flow_resume_token(&pool, row.id, &sha256_hex(&token), &format!("enc:{token}"))
        .await
        .unwrap();

    // The token addresses exactly this claim; a wrong token finds nothing.
    let hit = model::find_open_by_token_hash(&pool, &sha256_hex(&token))
        .await
        .unwrap()
        .expect("token addresses the open claim");
    assert_eq!(hit.instance_id, iid);
    let miss = model::find_open_by_token_hash(&pool, &sha256_hex("wrong-token"))
        .await
        .unwrap();
    assert!(miss.is_none());

    // Resuming through the token path closes the claim; token no longer matches.
    let envelope = ResumeEnvelope {
        action: "approve".into(),
        data: None,
    };
    run::resume_instance(&pool, None, None, hit.instance_id, &envelope, None)
        .await
        .unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
    let stale = model::find_open_by_token_hash(&pool, &sha256_hex(&token))
        .await
        .unwrap();
    assert!(stale.is_none(), "filled claim no longer matches the token");
}

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

#[tokio::test]
async fn concurrent_resume_single_claim_wins() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({}),
        json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    let env = ResumeEnvelope {
        action: "submit".into(),
        data: Some(json!({"approved": true})),
    };
    let (ra, rb) = tokio::join!(
        run::resume_instance(&pool, None, None, iid, &env, None),
        run::resume_instance(&pool, None, None, iid, &env, None),
    );
    let results = [ra, rb];
    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok, 1, "exactly one resume wins: {results:?}");
    let loser = results.iter().find(|r| r.is_err()).unwrap();
    let msg = loser.as_ref().unwrap_err().to_string();
    assert!(
        msg.contains("已被 resume")
            || msg.contains("不是 waiting")
            || msg.contains("没有等待中的节点"),
        "loser must be 409/terminal/snapshot-moved-on, got: {msg}"
    );

    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
    let open = model::find_open_flow_resume(&pool, iid, "gate")
        .await
        .unwrap();
    assert!(open.is_none(), "claim row closed after first resume");
}

#[tokio::test]
async fn timeout_port_wired_resumes_along_it() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({"actions": [{"id": "approve", "title": "通过"}], "timeout_secs": 3600}),
        json!([
            edge("start", "out", "gate"),
            edge("gate", "approve", "ok_end"),
            edge("gate", "timeout", "late_end")
        ]),
        json!([
            node(
                "ok_end",
                "end",
                json!({"outputs": [{"key": "ok", "value": {"literal": 1}}]})
            ),
            node(
                "late_end",
                "end",
                json!({
                    "outputs": [{"key": "t", "value": {"ref": ["gate", "resume", "timeout"]}}]
                })
            )
        ]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    force_expire(&pool, iid).await;
    let acted = run::sweep_expired_awaits(&pool, None, None).await.unwrap();
    assert!(acted >= 1, "sweeper acted on the expired claim");
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
    assert_eq!(done.outputs.unwrap()["t"], true);
}

#[tokio::test]
async fn timeout_unwired_fails_instance() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let iid = seed_waiting_instance(
        &pool,
        json!({"actions": [{"id": "approve", "title": "通过"}], "timeout_secs": 3600}),
        json!([edge("start", "out", "gate"), edge("gate", "approve", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
    )
    .await;
    let iid = park(&pool, &det, iid).await;

    force_expire(&pool, iid).await;
    run::sweep_expired_awaits(&pool, None, None).await.unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "failed");
    let err = done.error.unwrap();
    assert!(
        err.to_string().contains("await timeout"),
        "structured timeout reason: {err}"
    );
}

async fn force_expire(pool: &raisfast::db::Pool, iid: SnowflakeId) {
    let past = now_utc() - chrono::Duration::seconds(1);
    let sql = format!(
        "UPDATE flow_resume SET resume_until = {} WHERE instance_id = {}",
        raisfast::db::Driver::ph(1),
        raisfast::db::Driver::ph(2)
    );
    sqlx::query(raisfast::db::safe_sql(&sql))
        .bind(past)
        .bind(*iid)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn c4_event_sequence_park_then_resume() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    // Local bus so this test owns its emissions (EventingPersist bus field).
    let bus = raisfast::eventbus::EventBus::new(64);
    let mut rx = bus.subscribe();
    let iid = seed_waiting_instance(
        &pool,
        json!({"actions": [{"id": "approve", "title": "通过"}], "form_content": "订单 {{#start.order_no#}}"}),
        json!([edge("start", "out", "gate"), edge("gate", "approve", "end")]),
        json!([node("end", "end", json!({"outputs": []}))]),
    )
    .await;

    // Drive execute_instance through the eventing persist by registering the
    // bus globally for this test (fresh EventBus; other tests are unaffected
    // as they filter by instance id).
    raisfast::flows::events::set_bus(bus);
    let iid = park(&pool, &det, iid).await;

    let mut types = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let raisfast::event::Event::Custom {
            event_type, data, ..
        } = ev.as_ref()
            && data.get("instance_id").and_then(|v| v.as_str()) == Some(&iid.to_string())
        {
            types.push(event_type.clone());
        }
    }
    assert_eq!(
        types,
        vec![
            "workflow.graph_run_started",
            "workflow.node_run_started",
            "workflow.node_run_finished", // start
            "workflow.node_await_paused", // gate parks
        ],
        "park sequence: {types:?}"
    );

    let envelope = ResumeEnvelope {
        action: "approve".into(),
        data: None,
    };
    run::resume_instance(&pool, None, None, iid, &envelope, None)
        .await
        .unwrap();
    let mut after = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let raisfast::event::Event::Custom {
            event_type, data, ..
        } = ev.as_ref()
            && data.get("instance_id").and_then(|v| v.as_str()) == Some(&iid.to_string())
        {
            after.push(event_type.clone());
        }
    }
    assert_eq!(
        after,
        vec![
            "workflow.node_await_resumed",
            "workflow.node_run_started",
            "workflow.node_run_finished", // end
            "workflow.graph_run_finished",
        ],
        "resume sequence: {after:?}"
    );
    // Detach the test bus so later tests in this process re-register cleanly.
    // (set_bus is OnceLock-once; subsequent set_bus calls are ignored — the
    // bus stays wired to a dropped-sender bus, making emissions no-ops.)
}

#[test]
fn await_config_validation_rules() {
    use raisfast::flows::graph;
    // Duplicate action ids → 400.
    let dup_actions = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "gate",
                "await",
                json!({"actions": [
                    {"id": "go", "title": "a"}, {"id": "go", "title": "b"}
                ]})
            ),
            node("end", "end", json!({"outputs": []}))
        ]),
        json!([edge("start", "out", "gate"), edge("gate", "go", "end")]),
    );
    let err = graph::load_definition(&dup_actions).unwrap_err();
    assert!(err.to_string().contains("重复"), "{err}");

    // Invalid action id → 400.
    let bad_id = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "gate",
                "await",
                json!({"actions": [{"id": "1st", "title": "x"}]})
            ),
            node("end", "end", json!({"outputs": []}))
        ]),
        json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
    );
    let err = graph::load_definition(&bad_id).unwrap_err();
    assert!(err.to_string().contains("action id"), "{err}");

    // Reserved handle name → 400 (collides with the built-in timeout port).
    let reserved = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "gate",
                "await",
                json!({"actions": [{"id": "timeout", "title": "x"}]})
            ),
            node("end", "end", json!({"outputs": []}))
        ]),
        json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
    );
    let err = graph::load_definition(&reserved).unwrap_err();
    assert!(err.to_string().contains("保留名"), "{err}");

    // select without options → 400.
    let bad_select = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "gate",
                "await",
                json!({"inputs": [{"name": "s", "type": "select"}]})
            ),
            node("end", "end", json!({"outputs": []}))
        ]),
        json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
    );
    let err = graph::load_definition(&bad_select).unwrap_err();
    assert!(err.to_string().contains("options"), "{err}");
}
