use crate::*;
use serde_json::json;

async fn create_simple_workflow(app: &mut axum::Router, token: &str) -> String {
    let id = raisfast::utils::id::new_id().to_string();
    let (status, body) = send(
        app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Test Workflow",
                "description": "A test workflow",
                "steps": [
                    {"id": "s1", "name": "Step 1", "type": "task", "config": {}, "next": "s2"},
                    {"id": "s2", "name": "Step 2", "type": "task", "config": {}, "next": ""}
                ]
            }),
            token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "create workflow failed: {status} {body:?}"
    );
    id
}

#[tokio::test]
async fn workflow_crud_lifecycle() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = create_simple_workflow(&mut app, &token).await;

    let (status, body) = send(&mut app, get_auth("/api/v1/admin/workflows", &token)).await;
    assert!(status.is_success());
    assert_eq!(body["data"].as_array().unwrap().len(), 1);

    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/workflows/{id}"), &token),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["name"], "Test Workflow");

    let (status, _) = send(
        &mut app,
        delete_auth(&format!("/api/v1/admin/workflows/{id}"), &token),
    )
    .await;
    assert!(status.is_success());
}

#[tokio::test]
async fn workflow_start_and_execute_steps() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = create_simple_workflow(&mut app, &token).await;

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {"title": "Hello"}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "start failed: {status} {body:?}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["status"], "running");
    assert_eq!(body["data"]["current_step"], "s1");

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"step1_result": "ok"}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute step 1 failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["current_step"], "s2");

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"step2_result": "done"}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute step 2 failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["status"], "completed");
    assert!(body["data"]["current_step"].is_null());
}

#[tokio::test]
async fn workflow_branch_condition() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Branch Workflow",
                "steps": [
                    {"id": "decide", "name": "Decide", "type": "branch", "config": {}, "next": [
                        {"condition": {"approved": true}, "step": "publish"},
                        {"step": "reject"}
                    ]},
                    {"id": "publish", "name": "Publish", "type": "task", "config": {}, "next": ""},
                    {"id": "reject", "name": "Reject", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "create failed: {status} {body:?}");

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {"approved": false}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["current_step"], "reject");
}

#[tokio::test]
async fn workflow_cancel_instance() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Cancel Test",
                "steps": [
                    {"id": "s1", "name": "Wait", "type": "await", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "start: {status} {body:?}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/cancel"),
            json!({}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}"),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    assert_eq!(body["data"]["status"], "cancelled");
}

#[tokio::test]
async fn workflow_step_logs() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Log Test",
                "steps": [
                    {"id": "s1", "name": "Step 1", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {"x": 1}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    let _ = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"result": "ok"}}),
            &token,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/logs"),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    let logs = body["data"].as_array().unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0]["step_id"], "s1");
    assert_eq!(logs[0]["status"], "completed");
}

#[tokio::test]
async fn workflow_get_not_found() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        get_auth("/api/v1/admin/workflows/9999999999999", &token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_create_no_steps() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let id = raisfast::utils::id::new_id().to_string();
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({"id": id, "name": "Empty", "steps": []}),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_start_nonexistent() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows/9999999999999/start",
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_execute_nonexistent_instance() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows/instances/9999999999999/execute",
            json!({"output": {}}),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_cancel_already_completed() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Quick",
                "steps": [
                    {"id": "s1", "name": "Done", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (start_status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(start_status.is_success(), "start: {start_status}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    let _ = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/cancel"),
            json!({}),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_execute_not_running() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Quick",
                "steps": [
                    {"id": "s1", "name": "Done", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (start_status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(start_status.is_success(), "start: {start_status}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    let _ = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;

    let (status, _) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workflow_list_instances() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = create_simple_workflow(&mut app, &token).await;

    let _ = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {"x": 1}}),
            &token,
        ),
    )
    .await;

    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/workflows/instances", &token),
    )
    .await;
    assert!(
        status.is_success(),
        "list instances failed: {status} {body:?}"
    );
    let total = body["data"]["total"].as_i64().unwrap_or(0);
    assert!(total >= 1, "expected at least 1 instance, got {total}");
}

#[tokio::test]
async fn workflow_get_instance_not_found() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        get_auth("/api/v1/admin/workflows/instances/9999999999999", &token),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_step_logs_not_found() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        get_auth(
            "/api/v1/admin/workflows/instances/9999999999999/logs",
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_delete_nonexistent() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);
    let (status, _) = send(
        &mut app,
        delete_auth("/api/v1/admin/workflows/9999999999999", &token),
    )
    .await;
    assert!(status.is_success() || status == StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn workflow_parallel_with_join_next() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Parallel Join WF",
                "steps": [
                    {"id": "s1", "name": "Parallel Start", "type": "parallel", "config": {"join_next": "s4"}, "next": ["s2", "s3"]},
                    {"id": "s2", "name": "Branch A", "type": "task", "config": {}, "next": ""},
                    {"id": "s3", "name": "Branch B", "type": "task", "config": {}, "next": ""},
                    {"id": "s4", "name": "After Join", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "create failed: {status} {body:?}");

    let (_, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {"x": 1}}),
            &token,
        ),
    )
    .await;
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(body["data"]["current_step"], "s1");

    // Execute the parallel step — spawns branches s2 and s3, sets current to s2
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"parallel_started": true}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute parallel failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["current_step"], "s2");

    // Complete branch s2 — should advance to s3 (remaining branch)
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"branch_a": "done"}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute branch A failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["current_step"], "s3");

    // Complete branch s3 — all branches done, should join to s4
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"branch_b": "done"}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute branch B failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["current_step"], "s4");

    // Complete the join step — workflow should finish
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"final": true}}),
            &token,
        ),
    )
    .await;
    assert!(
        status.is_success(),
        "execute final step failed: {status} {body:?}"
    );
    assert_eq!(body["data"]["status"], "completed");
    assert!(body["data"]["current_step"].is_null());
}

#[tokio::test]
async fn workflow_parallel_no_join_completes() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Parallel No Join",
                "steps": [
                    {"id": "s1", "name": "Parallel", "type": "parallel", "config": {}, "next": ["s2", "s3"]},
                    {"id": "s2", "name": "Branch A", "type": "task", "config": {}, "next": ""},
                    {"id": "s3", "name": "Branch B", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (start_status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(start_status.is_success(), "start: {start_status}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    // Execute parallel → s2
    let (_, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;
    assert_eq!(body["data"]["current_step"], "s2");

    // Complete s2 → s3
    let (_, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"a": 1}}),
            &token,
        ),
    )
    .await;
    assert_eq!(body["data"]["current_step"], "s3");

    // Complete s3 → no join_next, workflow completes
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {"b": 2}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success(), "{status} {body:?}");
    assert_eq!(body["data"]["status"], "completed");
}

#[tokio::test]
async fn workflow_parallel_step_logs() {
    let (mut app, _) = test_app().await;
    let token = make_token("u1", 1, raisfast::models::user::UserRole::Admin);

    let id = raisfast::utils::id::new_id().to_string();
    let (create_status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/workflows",
            json!({
                "id": id,
                "name": "Parallel Logs",
                "steps": [
                    {"id": "s1", "name": "Parallel", "type": "parallel", "config": {}, "next": ["s2", "s3"]},
                    {"id": "s2", "name": "Branch A", "type": "task", "config": {}, "next": ""},
                    {"id": "s3", "name": "Branch B", "type": "task", "config": {}, "next": ""}
                ]
            }),
            &token,
        ),
    )
    .await;
    assert!(create_status.is_success(), "create: {create_status}");

    let (start_status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(start_status.is_success(), "start: {start_status}");
    let instance_id = body["data"]["id"].as_str().unwrap().to_string();

    // Execute parallel → spawns s2 + s3
    send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;

    // Complete s2
    send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;

    // Complete s3 → done
    send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/execute"),
            json!({"output": {}}),
            &token,
        ),
    )
    .await;

    // Check logs: s1 (completed) + s2 (completed) + s3 (completed)
    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/workflows/instances/{instance_id}/logs"),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
    let logs = body["data"].as_array().unwrap();
    assert!(logs.len() >= 3, "expected >= 3 logs, got {}", logs.len());
    let completed: Vec<_> = logs.iter().filter(|l| l["status"] == "completed").collect();
    assert_eq!(completed.len(), 3);
}
