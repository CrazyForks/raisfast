//! transform node + `{expr}` ValueExpr scenarios (dev-docs/workflow/
//! transform-node.md): cleaning functions in input maps / end outputs, and
//! the no-code assignments node reshaping data between steps.

use super::helpers::*;
use raisfast::flows::model::{self, Flow, FlowInstance, FlowVersion};
use raisfast::flows::run;
use raisfast::utils::tz::now_utc;
use serde_json::json;

async fn seed(pool: &raisfast::db::Pool, def: serde_json::Value) -> i64 {
    let now = now_utc();
    let flow_id = raisfast::utils::id::new_snowflake_id();
    model::insert_flow(
        pool,
        &Flow {
            id: flow_id,
            tenant_id: "default".into(),
            name: format!("transform-{}", raisfast::utils::id::new_id()),
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
            trigger_payload: Some(json!({"name": " Bo Zhang ", "price": 199, "phone": "13800001111", "missing": null})),
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

type SnowflakeId = raisfast::types::snowflake_id::SnowflakeId;

#[tokio::test]
async fn transform_node_cleans_and_flattens() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let def = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "clean",
                "transform",
                json!({"assignments": [
                    {"key": "display_name", "value": {"expr": "trim({{#start.name#}})"}},
                    {"key": "phone_masked", "value": {"expr":
                        "concat(replace({{#start.phone#}}, substring({{#start.phone#}}, 0, 7), \"*****\"))"}},
                    {"key": "price_yuan", "value": {"expr": "{{#start.price#}} / 100"}},
                    {"key": "fallback", "value": {"expr": "default({{#start.missing#}}, \"n/a\")"}},
                    {"key": "raw_copy", "value": {"ref": ["start", "price"]}}
                ]})
            ),
            node(
                "end",
                "end",
                json!({"outputs": [
                    {"key": "name", "value": {"ref": ["clean", "display_name"]}},
                    {"key": "phone", "value": {"ref": ["clean", "phone_masked"]}},
                    {"key": "price", "value": {"ref": ["clean", "price_yuan"]}},
                    {"key": "fb", "value": {"ref": ["clean", "fallback"]}},
                    {"key": "raw", "value": {"ref": ["clean", "raw_copy"]}}
                ]})
            )
        ]),
        json!([edge("start", "out", "clean"), edge("clean", "out", "end")]),
    );
    let iid = SnowflakeId::from(seed(&pool, def).await);
    run::execute_instance(&pool, iid, &det).await.unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
    let outs = done.outputs.unwrap();
    assert_eq!(outs["name"], "Bo Zhang");
    assert_eq!(outs["phone"], "*****1111");
    assert_eq!(outs["price"], json!(1.99));
    assert_eq!(outs["fb"], "n/a");
    assert_eq!(outs["raw"], json!(199));
}

#[tokio::test]
async fn expr_works_in_end_outputs_directly() {
    // The language layer serves consumers WITHOUT the node: end outputs and
    // input maps get {expr} for free (C1.2 third state wired).
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let def = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "end",
                "end",
                json!({"outputs": [
                    {"key": "shout", "value": {"expr": "upper(trim({{#start.name#}}))"}},
                    {"key": "total", "value": {"expr": "{{#start.price#}} * 2"}},
                    {"key": "mask", "value": {"expr": "concat(substring({{#start.phone#}}, 0, 3), \"****\")"}}
                ]})
            )
        ]),
        json!([edge("start", "out", "end")]),
    );
    let iid = SnowflakeId::from(seed(&pool, def).await);
    run::execute_instance(&pool, iid, &det).await.unwrap();
    let done = model::find_instance_by_id(&pool, iid).await.unwrap();
    assert_eq!(done.status, "success");
    let outs = done.outputs.unwrap();
    assert_eq!(outs["shout"], "BO ZHANG");
    assert_eq!(outs["total"], json!(398));
    assert_eq!(outs["mask"], "138****");
}

#[test]
fn transform_validation_rules() {
    use raisfast::flows::graph;
    let mk = |assignments| {
        def_of(
            json!([
                node("start", "start", json!({})),
                node("t", "transform", json!({"assignments": assignments})),
                node("end", "end", json!({"outputs": []}))
            ]),
            json!([edge("start", "out", "t"), edge("t", "out", "end")]),
        )
    };
    // empty → 400
    assert!(graph::load_definition(&mk(json!([]))).is_err());
    // duplicate keys → 400
    assert!(
        graph::load_definition(&mk(json!([
            {"key": "a", "value": {"literal": 1}},
            {"key": "a", "value": {"literal": 2}}
        ])))
        .unwrap_err()
        .to_string()
        .contains("重复")
    );
    // bad key → 400
    assert!(
        graph::load_definition(&mk(json!([
            {"key": "1st", "value": {"literal": 1}}
        ])))
        .unwrap_err()
        .to_string()
        .contains("key")
    );
    // non-string expr → 400
    assert!(
        graph::load_definition(&mk(json!([
            {"key": "ok", "value": {"expr": 42}}
        ])))
        .is_err()
    );
    // valid passes
    assert!(
        graph::load_definition(&mk(json!([
            {"key": "ok", "value": {"expr": "1 + 1"}},
            {"key": "ref", "value": {"ref": ["start", "price"]}},
            {"key": "lit", "value": {"literal": "x"}}
        ])))
        .is_ok()
    );
}

#[tokio::test]
async fn transform_bad_expr_fails_instance_with_reason() {
    let pool = super::super::test_pool().await;
    let det = Deterministic::new();
    let def = def_of(
        json!([
            node("start", "start", json!({})),
            node(
                "t",
                "transform",
                json!({"assignments": [
                    {"key": "x", "value": {"expr": "upper({{#start.price#}})"}}
                ]})
            ),
            node("end", "end", json!({"outputs": []}))
        ]),
        json!([edge("start", "out", "t"), edge("t", "out", "end")]),
    );
    let iid = SnowflakeId::from(seed(&pool, def).await);
    // Authoring-visible errors fail fast (same path as branch config
    // errors): the Err propagates instead of burning the instance.
    let err = run::execute_instance(&pool, iid, &det).await.unwrap_err();
    assert!(err.to_string().contains("upper"), "reason: {err}");
}
