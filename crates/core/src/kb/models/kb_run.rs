//! `kb_runs` table — one row per KB pipeline execution
//! (kb-observability-design §3.1). Long-running kinds are INSERTed as
//! `running` at start and UPDATEd at finish (DR2); short kinds insert once
//! at finish. Append-only; pruned by the T3 retention sweeper.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// Run kinds (kb-observability-design §3.1).
pub const KIND_INGEST_DOC: &str = "ingest_doc";
pub const KIND_ASK: &str = "ask";
pub const KIND_SEARCH: &str = "search";
pub const KIND_CHUNK_EDIT: &str = "chunk_edit";
pub const KIND_INDEX_FAQ: &str = "index_faq";
pub const KIND_PUBLISH_WIKI: &str = "publish_wiki";
pub const KIND_DISTILL: &str = "distill";
pub const KIND_REBUILD_VECTOR: &str = "rebuild_vector";

/// Run statuses.
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_OK: &str = "ok";
pub const STATUS_FAILED: &str = "failed";
pub const STATUS_DEGRADED: &str = "degraded";

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbRun {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub kind: String,
    pub trigger_src: String,
    pub kb_id: Option<SnowflakeId>,
    pub doc_id: Option<SnowflakeId>,
    pub agent_id: Option<SnowflakeId>,
    pub session_id: Option<SnowflakeId>,
    pub job_id: Option<SnowflakeId>,
    pub attempt: i64,
    pub status: String,
    pub latency_ms: Option<i64>,
    pub error: Option<String>,
    pub config_snapshot: Option<Value>,
    pub stages: Option<Value>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Values for the initial INSERT (long tasks) — finish-only fields are
/// `None` and filled by [`finish_run`].
pub struct NewKbRun {
    pub kind: &'static str,
    pub trigger_src: &'static str,
    pub tenant_id: String,
    pub kb_id: Option<SnowflakeId>,
    pub doc_id: Option<SnowflakeId>,
    pub agent_id: Option<SnowflakeId>,
    pub session_id: Option<SnowflakeId>,
    pub job_id: Option<SnowflakeId>,
    pub attempt: i64,
    pub status: &'static str,
    pub config_snapshot: Option<Value>,
}

/// INSERT a run row and return its id.
pub async fn insert_run(
    pool: &crate::db::Pool,
    run: &NewKbRun,
    stages: Option<&Value>,
    latency_ms: Option<i64>,
    error: Option<&str>,
) -> AppResult<SnowflakeId> {
    let (id, now) = (crate::utils::id::new_snowflake_id(), now_utc());
    raisfast_derive::crud_insert!(
        pool,
        "kb_runs",
        [
            "id" => id,
            "tenant_id" => run.tenant_id.as_str(),
            "kind" => run.kind,
            "trigger_src" => run.trigger_src,
            "kb_id" => run.kb_id,
            "doc_id" => run.doc_id,
            "agent_id" => run.agent_id,
            "session_id" => run.session_id,
            "job_id" => run.job_id,
            "attempt" => run.attempt,
            "status" => run.status,
            "latency_ms" => latency_ms,
            "error" => error,
            "config_snapshot" => run.config_snapshot.as_ref(),
            "stages" => stages,
            "created_at" => now,
            "updated_at" => now
        ]
    )?;
    Ok(id)
}

/// UPDATE a running row to its terminal state (DR2 second hop).
pub async fn finish_run(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
    status: &str,
    latency_ms: Option<i64>,
    error: Option<&str>,
    stages: Option<&Value>,
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool,
        "kb_runs",
        bind: [
            "status" => status,
            "latency_ms" => latency_ms,
            "error" => error,
            "stages" => stages,
            "updated_at" => now_utc()
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// Execution ordinal for non-job paths (DR6: reparse has no jobs.attempts —
/// "how many times has this target run" is the correct semantic there).
pub async fn count_runs(
    pool: &crate::db::Pool,
    kind: &str,
    doc_id: Option<SnowflakeId>,
    kb_id: Option<SnowflakeId>,
) -> AppResult<i64> {
    let mut clauses = vec![format!("kind = {}", Driver::ph(1))];
    if doc_id.is_some() {
        clauses.push(format!("doc_id = {}", Driver::ph(clauses.len() + 1)));
    }
    if kb_id.is_some() {
        clauses.push(format!("kb_id = {}", Driver::ph(clauses.len() + 1)));
    }
    let sql = format!(
        "SELECT {} FROM kb_runs WHERE {}",
        Driver::cast_int("COUNT(*)"),
        clauses.join(" AND ")
    );
    let mut query = sqlx::query_scalar::<_, i64>(crate::db::safe_sql(&sql)).bind(kind);
    if let Some(d) = doc_id {
        query = query.bind(i64::from(d));
    }
    if let Some(k) = kb_id {
        query = query.bind(i64::from(k));
    }
    query
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))
}

/// Optional list filters (admin runs API).
#[derive(Default)]
pub struct RunFilter {
    pub kind: Option<String>,
    pub kb_id: Option<i64>,
    pub doc_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub session_id: Option<i64>,
    pub status: Option<String>,
}

/// One dynamic WHERE term: clause fragment + its bind value.
enum Bind<'a> {
    Str(&'a str),
    Int(i64),
}

fn add_filter<'a>(
    column: &str,
    bind: Bind<'a>,
    clauses: &mut Vec<String>,
    binds: &mut Vec<Bind<'a>>,
) {
    let n = binds.len() + 1;
    clauses.push(format!("{column} = {}", Driver::ph(n)));
    binds.push(bind);
}

/// Paged run list, tenant-scoped, newest first. Filters build a dynamic
/// WHERE with sequential `Driver::ph` placeholders (cross-DB rule 1).
pub async fn list_runs(
    pool: &crate::db::Pool,
    tenant_id: &str,
    filter: &RunFilter,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<KbRun>, i64)> {
    let mut clauses: Vec<String> = vec![format!("tenant_id = {}", Driver::ph(1))];
    let mut binds: Vec<Bind> = vec![Bind::Str(tenant_id)];
    if let Some(v) = filter.kind.as_deref() {
        add_filter("kind", Bind::Str(v), &mut clauses, &mut binds);
    }
    if let Some(v) = filter.kb_id {
        add_filter("kb_id", Bind::Int(v), &mut clauses, &mut binds);
    }
    if let Some(v) = filter.doc_id {
        add_filter("doc_id", Bind::Int(v), &mut clauses, &mut binds);
    }
    if let Some(v) = filter.agent_id {
        add_filter("agent_id", Bind::Int(v), &mut clauses, &mut binds);
    }
    if let Some(v) = filter.session_id {
        add_filter("session_id", Bind::Int(v), &mut clauses, &mut binds);
    }
    if let Some(v) = filter.status.as_deref() {
        add_filter("status", Bind::Str(v), &mut clauses, &mut binds);
    }
    let where_sql = clauses.join(" AND ");

    // `safe_sql` borrows its input — bind the format! temporaries first.
    let count_stmt = format!(
        "SELECT {} FROM kb_runs WHERE {where_sql}",
        Driver::cast_int("COUNT(*)")
    );
    let count_sql = crate::db::safe_sql(&count_stmt);
    let mut count_query = sqlx::query_scalar::<_, i64>(count_sql);
    for b in &binds {
        count_query = match b {
            Bind::Str(v) => count_query.bind(*v),
            Bind::Int(v) => count_query.bind(*v),
        };
    }
    let total: i64 = count_query
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;

    let page = page.max(1);
    let page_size = page_size.clamp(1, 100);
    let offset = (page - 1) * page_size;
    let data_stmt = format!(
        "SELECT * FROM kb_runs WHERE {where_sql} ORDER BY created_at DESC LIMIT {} OFFSET {}",
        Driver::ph(binds.len() + 1),
        Driver::ph(binds.len() + 2)
    );
    let data_sql = crate::db::safe_sql(&data_stmt);
    let mut data_query = sqlx::query_as::<_, KbRun>(data_sql);
    for b in &binds {
        data_query = match b {
            Bind::Str(v) => data_query.bind(*v),
            Bind::Int(v) => data_query.bind(*v),
        };
    }
    let rows: Vec<KbRun> = data_query
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok((rows, total))
}

/// Single run by id (admin detail view), tenant-scoped.
pub async fn find_run_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbRun>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_runs",
        KbRun,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}
