//! App Bundle MVP-M2 e2e tests: install→enable→disable→uninstall lifecycle,
//! precheck conflicts, CAS 409, kill -9 self-heal, seed idempotency, drain
//! dead-lettering (test matrix from plans/mvp-plan.md MVP-M2).

use super::*;

fn admin_token() -> String {
    let id = uuid::Uuid::now_v7().to_string();
    make_token(&id, 1, raisfast::models::user::UserRole::Admin)
}

// ── bundle fixture ──────────────────────────────────────────────────

struct BundleSpec {
    id: String,
    version: String,
    /// Inject a seed-row typo (unknown field) — fails at step 7.
    bad_seed_field: bool,
}

impl BundleSpec {
    fn new(prefix: &str) -> Self {
        Self {
            id: format!("{prefix}-{}", &uuid::Uuid::now_v7().to_string()[..8]),
            version: "0.1.0".into(),
            bad_seed_field: false,
        }
    }

    fn table(&self) -> String {
        format!("{}_tickets", self.id.replace('-', "_"))
    }

    fn app_toml(&self) -> String {
        format!(
            "[app]\nid = \"{}\"\nname = \"Demo App\"\nversion = \"{}\"\npermissions = \
             [\"content-types:rw\", \"ingress:channels\", \"http:*\"]\n",
            self.id, self.version
        )
    }

    fn ct_toml(&self) -> String {
        format!(
            "[content_type]\nname = \"Ticket\"\nsingular = \"ticket\"\nplural = \"tickets\"\n\
             table = \"{}\"\ngroup = \"{}\"\n\n[fields.title]\ntype = \"text\"\n\n\
             [fields.status]\ntype = \"text\"\n",
            self.table(),
            self.id
        )
    }

    fn channel_json(&self) -> String {
        format!(
            "{{\"channel_key\":\"{}-widget\",\"provider\":\"generic-challenge\",\
             \"display_name\":\"Widget\",\"mode\":\"push\",\"transport\":\"http1\",\
             \"framing\":\"raw\",\"codec\":\"json\",\"verify_kind\":\"challenge\",\
             \"target_type\":\"ticket\"}}",
            self.id
        )
    }

    fn api_client_json(&self) -> String {
        format!(
            "{{\"client_key\":\"{}-llm\",\"base_url\":\"http://llm.invalid/v1\",\
             \"auth\":{{\"kind\":\"none\"}},\"ops\":{{\"chat\":{{\"method\":\"POST\",\
             \"path\":\"/chat\"}}}}}}",
            self.id
        )
    }

    fn options_json(&self) -> String {
        format!(
            "[{{\"key\":\"app.{}.enabled\",\"value\":true,\"type\":\"boolean\"}}]",
            self.id
        )
    }

    fn roles_json(&self) -> String {
        "[{\"name\":\"operator\",\"permissions\":[\"content-types:rw\"]}]".to_string()
    }

    fn data_json(&self) -> String {
        let typo = if self.bad_seed_field {
            ",\"typo_field\":\"x\""
        } else {
            ""
        };
        format!(
            "{{\"table\":\"{}\",\"rows\":[{{\"seed_key\":\"welcome\",\
             \"title\":\"Welcome\",\"status\":\"open\"{typo}}}]}}",
            self.table()
        )
    }

    /// Build the `.rafapp` bytes (hash-manifested zip).
    fn pack(&self) -> Vec<u8> {
        let entries: Vec<(&str, String)> = vec![
            ("app.toml", self.app_toml()),
            ("content-types/ticket.toml", self.ct_toml()),
            ("channels/widget.json", self.channel_json()),
            ("api-clients/llm.json", self.api_client_json()),
            ("seeds/options.json", self.options_json()),
            ("seeds/roles.json", self.roles_json()),
            ("seeds/data.json", self.data_json()),
        ];
        let mut manifest = String::new();
        for (path, content) in &entries {
            manifest.push_str(&format!(
                "{}  {}\n",
                raisfast::apps::package::hash_bytes(content.as_bytes()),
                path
            ));
        }
        let manifest = manifest.trim_end().to_string();

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = Default::default();
            for (path, content) in &entries {
                writer.start_file(*path, opts).unwrap();
                std::io::Write::write_all(&mut writer, content.as_bytes()).unwrap();
            }
            writer.start_file("META/manifest.sha256", opts).unwrap();
            std::io::Write::write_all(&mut writer, manifest.as_bytes()).unwrap();
            writer.finish().unwrap();
        }
        buf.into_inner()
    }
}

fn multipart_package(package: &[u8], options: Option<&str>, token: &str) -> Request<Body> {
    let boundary = "----rafapp";
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"package\"; \
             filename=\"app.rafapp\"\r\nContent-Type: application/zip\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(package);
    if let Some(opts) = options {
        body.extend_from_slice(
            format!(
                "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"options\"\r\n\r\n\
                 {opts}\r\n--{boundary}--\r\n"
            )
            .as_bytes(),
        );
    } else {
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    }
    Request::builder()
        .method("POST")
        .uri("/api/v1/admin/apps/install")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

fn preview_request(package: &[u8], token: &str) -> Request<Body> {
    let req = multipart_package(package, None, token);
    let (mut parts, body) = req.into_parts();
    parts.uri = "/api/v1/admin/apps/install-preview".parse().unwrap();
    Request::from_parts(parts, body)
}

async fn install(app: &mut axum::Router, spec: &BundleSpec) -> Value {
    let tok = admin_token();
    let (status, body) = send(app, multipart_package(&spec.pack(), None, &tok)).await;
    assert!(
        status == StatusCode::CREATED,
        "install failed: {status} {body:?}"
    );
    body["data"].clone()
}

async fn table_exists(pool: &raisfast::db::Pool, table: &str) -> bool {
    raisfast::db::Driver::table_exists(pool, table).await
}

async fn scalar_i64(pool: &raisfast::db::Pool, sql: &str) -> Option<i64> {
    sqlx::query_scalar::<_, i64>(raisfast::db::safe_sql(sql))
        .fetch_optional(pool)
        .await
        .unwrap()
}

// ── lifecycle ───────────────────────────────────────────────────────

#[tokio::test]
async fn apps_install_lifecycle_end_to_end() {
    let (mut app, state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("life");

    // install-preview first: clean report
    let (status, body) = send(&mut app, preview_request(&spec.pack(), &tok)).await;
    assert!(status.is_success(), "preview: {status} {body:?}");
    assert!(body["data"]["conflicts"].as_array().unwrap().is_empty());

    // install
    let data = install(&mut app, &spec).await;
    assert_eq!(data["status"], "installed");
    assert_eq!(data["pending_credentials"].as_array().unwrap().len(), 1);

    // materialized artifacts
    assert!(table_exists(&state.pool, &spec.table()).await);
    assert!(raisfast::db::Driver::has_column(&state.pool, &spec.table(), "seed_key").await);
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM {} WHERE seed_key = 'welcome'",
                spec.table()
            )
        )
        .await,
        Some(1),
        "seed row present"
    );
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM options WHERE option_key = 'app.{}.enabled'",
                spec.id
            )
        )
        .await,
        Some(1),
        "option seed present"
    );

    // channel seeded disabled
    let enabled: Option<bool> = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT enabled FROM itg_channels WHERE channel_key = '{}-widget'",
        spec.id
    )))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(enabled, Some(false), "seed channels land disabled");

    // channel seeded app-owned (channel-app-ownership.md §2)
    let app_id: Option<String> = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT app_id FROM itg_channels WHERE channel_key = '{}-widget'",
        spec.id
    )))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert_eq!(
        app_id.as_deref(),
        Some(spec.id.as_str()),
        "seeded channels carry the owning app_id"
    );

    // detail: every step carries an undo descriptor (§4.2 discipline)
    let (status, body) = send(
        &mut app,
        get_auth(&format!("/api/v1/admin/apps/{}", spec.id), &tok),
    )
    .await;
    assert!(status.is_success(), "detail: {status} {body:?}");
    let steps = body["data"]["install_log"]["steps"]
        .as_array()
        .expect("steps");
    assert!(!steps.is_empty(), "install log recorded");
    assert!(
        steps.iter().all(|s| s.get("undo").is_some()),
        "every step has an undo"
    );

    // enable → channels on
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/enable", spec.id),
            json!({}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "enable: {status} {body:?}");
    let enabled: bool = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT enabled FROM itg_channels WHERE channel_key = '{}-widget'",
        spec.id
    )))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(enabled, "enable turns channels on");

    // disable → channels off, app disabled
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/disable", spec.id),
            json!({}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "disable: {status} {body:?}");
    let enabled: bool = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT enabled FROM itg_channels WHERE channel_key = '{}-widget'",
        spec.id
    )))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    assert!(!enabled, "disable turns channels off");

    // uninstall keep_data → row gone, data kept
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/uninstall", spec.id),
            json!({"keep_data": true}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "uninstall: {status} {body:?}");
    assert!(
        scalar_i64(
            &state.pool,
            &format!("SELECT COUNT(*) FROM apps WHERE app_id = '{}'", spec.id)
        )
        .await
            == Some(0),
        "apps row deleted"
    );
    assert!(
        table_exists(&state.pool, &spec.table()).await,
        "keep-data: physical table survives"
    );
    assert!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM itg_channels WHERE channel_key = '{}-widget'",
                spec.id
            )
        )
        .await
            == Some(0),
        "channel recovered"
    );

    // reinstall over residual tables → re-attach path succeeds
    let data = install(&mut app, &spec).await;
    assert_eq!(data["status"], "installed");
    assert!(
        data["reattach_tables"]
            .as_array()
            .is_some_and(|t| !t.is_empty()),
        "re-attach detected"
    );
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM {} WHERE seed_key = 'welcome'",
                spec.table()
            )
        )
        .await,
        Some(1),
        "no duplicate seed rows after re-attach reinstall"
    );

    // uninstall purge → table dropped
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/uninstall", spec.id),
            json!({"keep_data": false}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "purge uninstall: {status} {body:?}");
    assert!(
        !table_exists(&state.pool, &spec.table()).await,
        "keep_data=false drops the table"
    );
}

#[tokio::test]
async fn apps_install_preview_reports_conflicts() {
    let (mut app, _state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("conf");

    install(&mut app, &spec).await;

    // same bundle again → app-exists blocking conflict
    let (status, body) = send(&mut app, preview_request(&spec.pack(), &tok)).await;
    assert!(status.is_success());
    let conflicts = body["data"]["conflicts"].as_array().unwrap().clone();
    assert!(
        conflicts
            .iter()
            .any(|c| c["code"] == "app-exists" && c["severity"] == "block"),
        "conflicts: {conflicts:?}"
    );

    // install attempt → 409 with the summary
    let (status, body) = send(&mut app, multipart_package(&spec.pack(), None, &tok)).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "dup install: {status} {body:?}"
    );
}

#[tokio::test]
async fn apps_busy_status_rejected_409() {
    let (mut app, state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("busy");
    install(&mut app, &spec).await;

    // simulate an in-flight install (kill -9 residue)
    sqlx::query(raisfast::db::safe_sql(&format!(
        "UPDATE apps SET status = 'installing' WHERE app_id = '{}'",
        spec.id
    )))
    .execute(&state.pool)
    .await
    .unwrap();

    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/enable", spec.id),
            json!({}),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "busy: {status} {body:?}");
    // NOTE: message detail rides on the i18n layer (`AppError::Conflict`),
    // which currently renders raw keys (locales lack locale roots) — the
    // authoritative reason is `GET /admin/apps/{id}` (`status` + last_error).
}

#[tokio::test]
async fn apps_kill9_self_heal_rolls_back() {
    let (mut app, state) = test_app().await;
    let spec = BundleSpec::new("heal");
    install(&mut app, &spec).await;

    // Corrupt: pretend the process died mid-install.
    sqlx::query(raisfast::db::safe_sql(&format!(
        "UPDATE apps SET status = 'installing' WHERE app_id = '{}'",
        spec.id
    )))
    .execute(&state.pool)
    .await
    .unwrap();

    // "Restart": a fresh registry over the same pool replays compensation.
    let test_protocols = Arc::new({
        let mut reg = raisfast::protocols::ProtocolRegistry::new();
        reg.register(raisfast::protocols::ownable::OwnableProtocol);
        reg.register(raisfast::protocols::timestampable::TimestampableProtocol);
        reg
    });
    let content_registry = Arc::new(raisfast::content_type::ContentTypeRegistry::new());
    let config = Arc::new(test_config());
    let registry = raisfast::apps::AppRegistry::init(
        state.pool.clone(),
        config,
        content_registry,
        test_protocols,
    )
    .await
    .expect("re-init");

    let row = registry.detail(&spec.id).await.expect("row exists");
    assert_eq!(row.status, "rolled_back");
    assert!(row.last_error.is_some());

    // Compensation undid the materialized artifacts.
    assert!(
        !table_exists(&state.pool, &spec.table()).await,
        "installing自愈回滚 drops the CT table"
    );
    assert!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM itg_channels WHERE channel_key = '{}-widget'",
                spec.id
            )
        )
        .await
            == Some(0),
        "channel compensated"
    );
}

#[tokio::test]
async fn apps_seed_idempotency_and_option_preservation() {
    let (mut app, state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("seed");

    install(&mut app, &spec).await;

    // User edits the seeded option.
    sqlx::query(raisfast::db::safe_sql(&format!(
        "UPDATE options SET value = 'false' WHERE option_key = 'app.{}.enabled'",
        spec.id
    )))
    .execute(&state.pool)
    .await
    .unwrap();

    // Uninstall compensates the option seed; reinstall recreates it exactly
    // once (no duplicates) — the DO-NOTHING upgrade semantics have their own
    // unit test in seeder.rs.
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/uninstall", spec.id),
            json!({"keep_data": true}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "uninstall: {status} {body:?}");
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM options WHERE option_key = 'app.{}.enabled'",
                spec.id
            )
        )
        .await,
        Some(0),
        "uninstall removes the app's option rows"
    );

    install(&mut app, &spec).await;
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM options WHERE option_key = 'app.{}.enabled'",
                spec.id
            )
        )
        .await,
        Some(1),
        "reinstall seeds the option exactly once"
    );
}

#[tokio::test]
async fn apps_drain_dead_letters_on_timeout() {
    let (mut app, state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("drain");
    install(&mut app, &spec).await;

    // In-flight envelope: a receipt stuck in `received` for the app channel.
    let channel_id: i64 = sqlx::query_scalar(raisfast::db::safe_sql(&format!(
        "SELECT id FROM itg_channels WHERE channel_key = '{}-widget'",
        spec.id
    )))
    .fetch_one(&state.pool)
    .await
    .unwrap();
    sqlx::query(raisfast::db::safe_sql(&format!(
        "INSERT INTO itg_receipts (id, channel_id, external_id, kind, payload_hash, status) \
         VALUES ({}, {}, 'ext-1', 'event', 'h', 'received')",
        raisfast::utils::id::new_id(),
        channel_id
    )))
    .execute(&state.pool)
    .await
    .unwrap();

    // Uninstall drains; the stuck receipt must be dead-lettered, not lost.
    let (status, body) = send(
        &mut app,
        post_json_auth(
            &format!("/api/v1/admin/apps/{}/uninstall", spec.id),
            json!({"keep_data": true}),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "uninstall: {status} {body:?}");

    let (receipt_status, steps): (String, Option<Value>) =
        sqlx::query_as("SELECT status, steps FROM itg_receipts WHERE external_id = 'ext-1'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(receipt_status, "dead", "drained:timeout dead-letter");
    let note = steps
        .as_ref()
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.last())
        .and_then(|s| s.get("note"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert_eq!(note, "drained:timeout");
}

#[tokio::test]
async fn apps_permission_scope_required() {
    let (mut app, _state) = test_app().await;
    // Plain reader token → forbidden.
    let id = uuid::Uuid::now_v7().to_string();
    let tok = make_token(&id, 2, raisfast::models::user::UserRole::Reader);
    let (status, _body) = send(&mut app, get_auth("/api/v1/admin/apps", &tok)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn apps_install_failure_rolls_back_clean() {
    let (mut app, state) = test_app().await;
    let mut spec = BundleSpec::new("fail");
    // Seed row references an unknown field → package/precheck pass, step 7
    // (seeds) fails → every earlier step must be compensated.
    spec.bad_seed_field = true;
    let tok = admin_token();

    let (status, body) = send(&mut app, multipart_package(&spec.pack(), None, &tok)).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "injected install failure: {status} {body:?}"
    );

    // apps row lands in rolled-back with the reason.
    let (row_status, last_error): (String, Option<String>) =
        sqlx::query_as(raisfast::db::safe_sql(&format!(
            "SELECT status, last_error FROM apps WHERE app_id = '{}'",
            spec.id
        )))
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(row_status, "rolled_back");
    assert!(last_error.is_some_and(|e| e.contains("seed")));

    // Zero orphans: table dropped, channel gone, role gone, option gone.
    assert!(!table_exists(&state.pool, &spec.table()).await);
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM itg_channels WHERE channel_key = '{}-widget'",
                spec.id
            )
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!("SELECT COUNT(*) FROM roles WHERE name = '{}'", spec.id)
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM options WHERE option_key = 'app.{}.enabled'",
                spec.id
            )
        )
        .await,
        Some(0)
    );
    assert_eq!(
        scalar_i64(
            &state.pool,
            &format!(
                "SELECT COUNT(*) FROM app_ct_refs WHERE app_id = '{}'",
                spec.id
            )
        )
        .await,
        Some(0)
    );
}

// ── channel app ownership (channel-app-ownership.md §2/§4.1) ────────

#[tokio::test]
async fn channel_app_ownership_admin_api() {
    let (mut app, _state) = test_app().await;
    let tok = admin_token();
    let spec = BundleSpec::new("own");
    let data = install(&mut app, &spec).await;
    assert_eq!(data["status"], "installed");

    // Platform channel (no app_id) — stays platform-owned.
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/integration/channels",
            json!({
                "channel_key": "platform-http",
                "provider": "generic-hmac",
                "mode": "push", "transport": "http1", "framing": "raw", "codec": "json",
                "verify_kind": "challenge",
                "target_type": "ingress_note",
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "platform channel: {status} {body:?}");
    assert_eq!(body["data"]["app_id"], serde_json::Value::Null);

    // App-owned channel (app_id = installed app) — accepted.
    let (status, body) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/integration/channels",
            json!({
                "channel_key": "own-app-ch",
                "provider": "generic-hmac",
                "mode": "push", "transport": "http1", "framing": "raw", "codec": "json",
                "verify_kind": "challenge",
                "target_type": "ingress_note",
                "app_id": spec.id,
            }),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "app channel: {status} {body:?}");
    assert_eq!(body["data"]["app_id"], spec.id.clone());

    // Unknown app_id → rejected.
    let (status, _) = send(
        &mut app,
        post_json_auth(
            "/api/v1/admin/integration/channels",
            json!({
                "channel_key": "ghost-app-ch",
                "provider": "generic-hmac",
                "mode": "push", "transport": "http1", "framing": "raw", "codec": "json",
                "verify_kind": "challenge",
                "target_type": "ingress_note",
                "app_id": "no-such-app",
            }),
            &tok,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "unknown app_id rejected");

    // ?app= filter: only that app's channels (seeded widget + app-owned), not
    // the platform channel.
    let (status, body) = send(
        &mut app,
        get_auth(
            &format!("/api/v1/admin/integration/channels?app={}", spec.id),
            &tok,
        ),
    )
    .await;
    assert!(status.is_success(), "filter: {status} {body:?}");
    let items = body["data"].as_array().unwrap();
    assert!(!items.is_empty(), "app filter returns rows");
    assert!(
        items.iter().all(|c| c["app_id"] == spec.id.clone()),
        "all filtered rows belong to the app"
    );
    assert!(
        items.iter().all(|c| c["channel_key"] != "platform-http"),
        "platform channel excluded"
    );
    assert!(
        items
            .iter()
            .any(|c| c["channel_key"] == format!("{}-widget", spec.id)),
        "seeded widget channel present"
    );

    // ?app=none → only platform/global channels (app_id NULL).
    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/integration/channels?app=none", &tok),
    )
    .await;
    assert!(status.is_success(), "none filter: {status} {body:?}");
    let items = body["data"].as_array().unwrap();
    assert!(
        items.iter().any(|c| c["channel_key"] == "platform-http"),
        "platform channel present under app=none"
    );
    assert!(
        items.iter().all(|c| c["app_id"] == serde_json::Value::Null),
        "app=none returns only app_id-null rows"
    );

    // Unfiltered list includes the platform channel (app_id null).
    let (status, body) = send(
        &mut app,
        get_auth("/api/v1/admin/integration/channels", &tok),
    )
    .await;
    assert!(status.is_success());
    let items = body["data"].as_array().unwrap();
    assert!(
        items.iter().any(|c| c["channel_key"] == "platform-http"),
        "unfiltered list includes platform channels"
    );
}
