//! KB diagnostics endpoints — health probe + consistency scan + vector
//! rebuild producer (kb-observability-design §5–§7, T3).
//!
//! All admin-only. `/admin/kb/health` never touches `/readyz` (DR7);
//! `/admin/kb/diagnostics` is compute-on-demand with a 10s TTL cache.

use std::collections::HashSet;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use axum::Json;
use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::errors::response::ApiResponse;
use crate::kb::service::KbDeps;
use crate::middleware::auth::AuthUser;
use crate::types::snowflake_id::SnowflakeId;
use crate::worker::JobQueue as _;

fn tenant_of(auth: &AuthUser) -> String {
    auth.tenant_id()
        .unwrap_or(crate::constants::DEFAULT_TENANT)
        .to_string()
}

fn internal(e: impl std::fmt::Display) -> AppError {
    AppError::Internal(anyhow::anyhow!(e.to_string()))
}

fn now_minus(secs: i64) -> crate::utils::tz::Timestamp {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    chrono::DateTime::from_timestamp(now.as_secs() as i64 - secs, 0).unwrap_or_default()
}

// ── /admin/kb/health ───────────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct HealthQuery {
    /// `probe=embed` performs a real 1-token embedding round-trip
    /// (costs one API call; the default probe is config-only).
    probe: Option<String>,
}

/// Component health snapshot (§7). Cheap by default; the embed smoke is
/// explicit opt-in so health checks never cost money by accident.
pub async fn admin_kb_health(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<HealthQuery>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let config = &deps.config;
    let mut components = serde_json::Map::new();

    // vector backend identity + warm state.
    let backend = deps.vector.backend_name().to_string();
    if backend == "bruteforce" {
        let (sql_total, index_total) = warm_counts(&deps).await?;
        let status = if sql_total > 0 && index_total == 0 {
            "degraded" // fully cold (restart) — lazy warm-up heals on first query
        } else {
            "up"
        };
        components.insert(
            "vector".into(),
            json!({
                "backend": backend,
                "status": status,
                "index_units": index_total,
                "sql_units": sql_total,
                "note": "in-memory; lazy warm-up on first dense query (DR10)",
            }),
        );
    } else {
        components.insert(
            "vector".into(),
            json!({ "backend": backend, "status": "up" }),
        );
    }

    // bm25: tantivy index directory present + writable.
    let dir = std::path::Path::new(&config.storage_root_dir).join("kb_search_index");
    let probe = dir.join(".healthprobe");
    let bm25 = match std::fs::write(&probe, b"ok").and_then(|_| std::fs::remove_file(&probe)) {
        Ok(()) => json!({ "status": "up", "dir": dir.display().to_string() }),
        Err(e) => json!({
            "status": "degraded",
            "dir": dir.display().to_string(),
            "error": e.to_string(),
        }),
    };
    components.insert("bm25".into(), bm25);

    // embedder: configured? optional smoke probe.
    let embed_model = config.ai.embedding_model.as_deref().unwrap_or_default();
    let mut embedder = json!({
        "model": if embed_model.is_empty() { Value::Null } else { json!(embed_model) },
        "status": if embed_model.is_empty() { "degraded" } else { "up" },
    });
    if q.probe.as_deref() == Some("embed") {
        match deps.embedder.embed(&["ping"]).await {
            Ok(v) => {
                embedder["probe"] =
                    json!({ "status": "up", "dim": v.first().map_or(0, |e| e.len()) })
            }
            Err(e) => {
                embedder["probe"] = json!({ "status": "down", "error": e.to_string() });
                embedder["status"] = json!("down");
            }
        }
    }
    components.insert("embedder".into(), embedder);

    // chat provider: configured?
    let chat_model = config.ai.model.as_deref().unwrap_or_default();
    components.insert(
        "provider".into(),
        json!({
            "model": if chat_model.is_empty() { Value::Null } else { json!(chat_model) },
            "status": if chat_model.is_empty() { "degraded" } else { "up" },
        }),
    );

    // queue: kb_* job backlog.
    let sql = format!(
        "SELECT status, {} FROM jobs WHERE job_type LIKE 'kb\\_%' ESCAPE '\\' GROUP BY status",
        Driver::cast_int("COUNT(*)")
    );
    let rows: Vec<(String, i64)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .fetch_all(&deps.pool)
        .await
        .map_err(internal)?;
    let mut queue = serde_json::Map::new();
    let mut dead = 0_i64;
    for (status, count) in &rows {
        if status == "dead" {
            dead = *count;
        }
        queue.insert(status.clone(), json!(count));
    }
    queue.insert(
        "status".into(),
        json!(if dead > 0 { "degraded" } else { "up" }),
    );
    components.insert("queue".into(), Value::Object(queue));

    Ok(ApiResponse::success(json!({ "components": components })))
}

/// Sum of (SQL embedded units, in-memory index units) over all KBs — the
/// bruteforce warm check (§7 vector_warm).
async fn warm_counts(deps: &KbDeps) -> AppResult<(i64, u64)> {
    let sql = format!(
        "SELECT {} FROM kb_chunks WHERE embedding IS NOT NULL AND status = 'active'",
        Driver::cast_int("COUNT(*)")
    );
    let sql_total: i64 = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .fetch_one(&deps.pool)
        .await
        .map_err(internal)?;
    let kb_sql = "SELECT id FROM kb_knowledge_bases";
    let kb_ids: Vec<i64> = sqlx::query_scalar(crate::db::safe_sql(kb_sql))
        .fetch_all(&deps.pool)
        .await
        .map_err(internal)?;
    let mut index_total = 0_u64;
    for kb in kb_ids {
        index_total += deps.vector.count(kb).await.unwrap_or(0);
    }
    Ok((sql_total, index_total))
}

// ── /admin/kb/diagnostics (R1–R7, §6) ──────────────────────────────

/// 10s TTL cache for the scan (§6: 现算 + 缓存).
static DIAG_CACHE: LazyLock<RwLock<Option<(Instant, Value)>>> = LazyLock::new(|| RwLock::new(None));

pub async fn admin_kb_diagnostics(
    auth: AuthUser,
    State(state): State<AppState>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let tenant = tenant_of(&auth);
    // Cached copy is tenant-tagged: rules are computed per tenant.
    if let Some((at, cached)) = DIAG_CACHE.read().unwrap_or_else(|p| p.into_inner()).clone()
        && at.elapsed() < Duration::from_secs(10)
        && cached.get("_tenant").and_then(|t| t.as_str()) == Some(tenant.as_str())
    {
        return Ok(ApiResponse::success(cached));
    }
    let deps = state.kb_deps()?;
    let payload = run_scan(&deps, &tenant).await?;
    *DIAG_CACHE.write().unwrap_or_else(|p| p.into_inner()) =
        Some((Instant::now(), payload.clone()));
    Ok(ApiResponse::success(payload))
}

fn rule(id: &str, severity: &str, title_key: &str, items: Vec<Value>) -> Value {
    json!({
        "id": id,
        "severity": severity,
        "title": title_key,
        "count": items.len(),
        "items": items,
    })
}

pub async fn run_scan(deps: &KbDeps, tenant: &str) -> AppResult<Value> {
    let cutoff10 = now_minus(600);
    let cutoff30 = now_minus(1800);

    // Shared: kb_process_document job payloads (for R1/R2 provenance).
    let job_payloads: Vec<String> = sqlx::query_scalar(crate::db::safe_sql(
        "SELECT payload FROM jobs WHERE job_type = 'kb_process_document' ORDER BY created_at DESC LIMIT 1000",
    ))
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let active_job_payloads: Vec<String> = sqlx::query_scalar(crate::db::safe_sql(
        "SELECT payload FROM jobs WHERE job_type = 'kb_process_document' AND status IN ('pending','running') LIMIT 500",
    ))
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let traced_docs: HashSet<i64> = sqlx::query_scalar(crate::db::safe_sql(
        "SELECT DISTINCT doc_id FROM kb_runs WHERE kind = 'ingest_doc' AND doc_id IS NOT NULL",
    ))
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?
    .into_iter()
    .collect();
    let payload_mentions = |doc_id: i64, payloads: &[String]| -> bool {
        let plain = format!("\"doc_id\":\"{doc_id}\"");
        let encoded = format!(
            "\"doc_id\":\"{}\"",
            crate::types::snowflake_id::encode_id(doc_id)
        );
        payloads
            .iter()
            .any(|p| p.contains(&plain) || p.contains(&encoded))
    };

    // R1 事件丢失: pending > 10min, no job payload mention, no run row.
    let r1_rows: Vec<(i64, String, crate::utils::tz::Timestamp)> = sqlx::query_as(
        crate::db::safe_sql(&format!(
            "SELECT id, title, created_at FROM kb_documents WHERE status = 'pending' AND created_at < {} AND tenant_id = {}",
            Driver::ph(1),
            Driver::ph(2)
        )),
    )
    .bind(cutoff10)
    .bind(tenant)
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let r1_items: Vec<Value> = r1_rows
        .iter()
        .filter(|(id, _, _)| !payload_mentions(*id, &job_payloads) && !traced_docs.contains(id))
        .map(|(id, title, created)| {
            json!({ "doc_id": id, "title": title, "created_at": created.to_rfc3339(),
                    "action": "reparse" })
        })
        .collect();

    // R2 中间态僵死: intermediate + stale updated_at + no active job.
    let r2_rows: Vec<(i64, String, String, crate::utils::tz::Timestamp)> =
        sqlx::query_as(crate::db::safe_sql(&format!(
            "SELECT id, title, status, updated_at FROM kb_documents \
             WHERE status IN ('parsing','chunking','embedding','indexing') \
             AND updated_at < {} AND tenant_id = {}",
            Driver::ph(1),
            Driver::ph(2)
        )))
        .bind(cutoff30)
        .bind(tenant)
        .fetch_all(&deps.pool)
        .await
        .map_err(internal)?;
    let r2_items: Vec<Value> = r2_rows
        .iter()
        .filter(|(id, _, _, _)| !payload_mentions(*id, &active_job_payloads))
        .map(|(id, title, status, updated)| {
            json!({ "doc_id": id, "title": title, "status": status,
                    "updated_at": updated.to_rfc3339(), "action": "reparse" })
        })
        .collect();

    // R3 失败文档.
    let r3_rows: Vec<(i64, String, Option<String>, crate::utils::tz::Timestamp)> =
        sqlx::query_as(crate::db::safe_sql(&format!(
            "SELECT id, title, error, updated_at FROM kb_documents \
             WHERE status = 'failed' AND tenant_id = {} ORDER BY updated_at DESC LIMIT 50",
            Driver::ph(1)
        )))
        .bind(tenant)
        .fetch_all(&deps.pool)
        .await
        .map_err(internal)?;
    let r3_items: Vec<Value> = r3_rows
        .iter()
        .map(|(id, title, error, updated)| {
            json!({ "doc_id": id, "title": title, "error": error,
                    "updated_at": updated.to_rfc3339(), "action": "reparse" })
        })
        .collect();

    // R4 半成品: ready docs with NULL embeddings on document chunks
    // (docs still mid-ingest are excluded — their NULLs are transient).
    let r4_rows: Vec<(i64, i64)> = sqlx::query_as(crate::db::safe_sql(&format!(
        "SELECT doc_id, {} FROM kb_chunks \
         WHERE kind = 'document' AND doc_id IS NOT NULL AND embedding IS NULL \
         GROUP BY doc_id LIMIT 100",
        Driver::cast_int("COUNT(*)")
    )))
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let mut r4_items = Vec::new();
    for (doc_id, n) in &r4_rows {
        if doc_status_is(deps, *doc_id, "ready").await? {
            r4_items.push(json!({
                "doc_id": doc_id, "missing_embeddings": n, "action": "reparse"
            }));
        }
    }

    // R5 向量漂移: per active KB, SQL embedded count vs backend count.
    let mut r5_items = Vec::new();
    let kbs: Vec<(i64, String)> = sqlx::query_as(crate::db::safe_sql(&format!(
        "SELECT id, name FROM kb_knowledge_bases WHERE status = 'active' AND tenant_id = {} LIMIT 100",
        Driver::ph(1)
    )))
    .bind(tenant)
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let rebuild_active: Vec<String> = sqlx::query_scalar(crate::db::safe_sql(
        "SELECT payload FROM jobs WHERE job_type = 'kb_rebuild_vector_index' AND status IN ('pending','running') LIMIT 100",
    ))
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    for (kb_id, name) in &kbs {
        let kb_str = kb_id.to_string();
        if rebuild_active.iter().any(|p| p.contains(&kb_str)) {
            continue; // rebuild window (误报规避)
        }
        let sql_n: i64 = sqlx::query_scalar(crate::db::safe_sql(&format!(
            "SELECT {} FROM kb_chunks WHERE kb_id = {} AND embedding IS NOT NULL AND status = 'active'",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        )))
        .bind(*kb_id)
        .fetch_one(&deps.pool)
        .await
        .map_err(internal)?;
        let index_n = deps.vector.count(*kb_id).await.unwrap_or(0);
        if i64::try_from(index_n).unwrap_or(i64::MAX) != sql_n {
            r5_items.push(json!({
                "kb_id": kb_id, "name": name, "sql_units": sql_n, "index_units": index_n,
                "action": "rebuild",
            }));
        }
    }

    // R6 KB 死信.
    let r6_rows: Vec<(i64, String, Option<String>, crate::utils::tz::Timestamp)> =
        sqlx::query_as(crate::db::safe_sql(
            "SELECT id, job_type, error, updated_at FROM jobs \
             WHERE job_type LIKE 'kb\\_%' ESCAPE '\\' AND status = 'dead' \
             ORDER BY updated_at DESC LIMIT 50",
        ))
        .fetch_all(&deps.pool)
        .await
        .map_err(internal)?;
    let r6_items: Vec<Value> = r6_rows
        .iter()
        .map(|(id, jt, error, updated)| {
            json!({ "job_id": id, "job_type": jt, "error": error,
                    "updated_at": updated.to_rfc3339(), "action": "retry" })
        })
        .collect();

    // R7 run 僵死.
    let r7_rows: Vec<(i64, String, crate::utils::tz::Timestamp)> = sqlx::query_as(
        crate::db::safe_sql(&format!(
            "SELECT id, kind, updated_at FROM kb_runs WHERE status = 'running' AND updated_at < {} LIMIT 50",
            Driver::ph(1)
        )),
    )
    .bind(cutoff30)
    .fetch_all(&deps.pool)
    .await
    .map_err(internal)?;
    let r7_items: Vec<Value> = r7_rows
        .iter()
        .map(|(id, kind, updated)| {
            json!({ "run_id": id, "kind": kind, "updated_at": updated.to_rfc3339(),
                    "action": "mark_failed" })
        })
        .collect();

    Ok(json!({
        "_tenant": tenant,
        "generated_at": crate::utils::tz::now_utc().to_rfc3339(),
        "rules": [
            rule("r1_event_lost", "high", "kb.diag.r1", r1_items),
            rule("r2_stuck_intermediate", "high", "kb.diag.r2", r2_items),
            rule("r3_failed_docs", "medium", "kb.diag.r3", r3_items),
            rule("r4_missing_embeddings", "medium", "kb.diag.r4", r4_items),
            rule("r5_vector_drift", "high", "kb.diag.r5", r5_items),
            rule("r6_dead_jobs", "medium", "kb.diag.r6", r6_items),
            rule("r7_stuck_runs", "low", "kb.diag.r7", r7_items),
        ],
    }))
}

async fn doc_status_is(deps: &KbDeps, doc_id: i64, expected: &str) -> AppResult<bool> {
    let status: Option<String> = sqlx::query_scalar(crate::db::safe_sql(&format!(
        "SELECT status FROM kb_documents WHERE id = {}",
        Driver::ph(1)
    )))
    .bind(doc_id)
    .fetch_optional(&deps.pool)
    .await
    .map_err(internal)?;
    Ok(status.as_deref() == Some(expected))
}

// ── POST /admin/kb/vector/rebuild (producer, W2 收口) ─────────────

#[derive(Deserialize)]
pub struct RebuildRequest {
    pub kb_id: SnowflakeId,
}

/// Enqueue a full vector rebuild for one KB (the `KbRebuildVectorIndex`
/// job's first real producer — technical design §10 promised endpoint).
pub async fn admin_rebuild_vector(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<RebuildRequest>,
) -> AppResult<ApiResponse<Value>> {
    auth.ensure_admin()?;
    let deps = state.kb_deps()?;
    let tenant = tenant_of(&auth);
    crate::kb::models::knowledge_base::find_kb_by_id(&deps.pool, req.kb_id, &tenant)
        .await?
        .ok_or_else(|| AppError::NotFound("kb_knowledge_base".into()))?;
    let mut new_job = crate::worker::NewJob::from(crate::worker::Job::KbRebuildVectorIndex {
        kb_id: req.kb_id,
        tenant_id: tenant,
    });
    new_job.priority = -5; // bulk work must not starve online jobs
    let queue = crate::worker::DefaultJobQueue::new(state.pool.clone());
    queue.enqueue(new_job).await?;
    Ok(ApiResponse::success(json!({ "queued": true })))
}

/// Retention sweep used by the `KbRunsCleanup` job (§10) — exported here so
/// the worker handler stays a thin bridge.
pub async fn sweep_runs(pool: &crate::db::Pool, retention_days: i64) -> AppResult<u64> {
    if retention_days <= 0 {
        return Ok(0); // keep forever
    }
    let cutoff = now_minus(retention_days * 86_400);
    let sql = format!(
        "DELETE FROM kb_runs WHERE status != 'running' AND created_at < {}",
        Driver::ph(1)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(cutoff)
        .execute(pool)
        .await
        .map_err(internal)?;
    Ok(result.rows_affected())
}
