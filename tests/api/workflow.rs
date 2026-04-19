use crate::*;
use serde_json::json;

async fn create_simple_workflow(app: &mut axum::Router, token: &str) -> String {
    let id = uuid::Uuid::now_v7().to_string();
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
    let token = make_token("u1", "admin");

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
    let token = make_token("u1", "admin");

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
    let token = make_token("u1", "admin");

    let id = uuid::Uuid::now_v7().to_string();
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
    let token = make_token("u1", "admin");

    let id = uuid::Uuid::now_v7().to_string();
    let _ = send(
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

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/workflows/{id}/start"),
            json!({"context": {}}),
            &token,
        ),
    )
    .await;
    assert!(status.is_success());
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
    let token = make_token("u1", "admin");

    let id = uuid::Uuid::now_v7().to_string();
    let _ = send(
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
