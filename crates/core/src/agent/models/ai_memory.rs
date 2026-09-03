//! Agent long-term memory model (`ai_memories`). Ownership scope = (tenant,
//! agent); `superseded_by IS NULL` is the live-row predicate.

use serde::{Deserialize, Serialize};

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// Row/column list used by hand-written reads (kept in one place).
const MEMORY_COLS: &str = "id, tenant_id, agent_id, session_id, mem_key, content, category, \
    importance, superseded_by, created_at, updated_at";

/// One memory row.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AiMemory {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub agent_id: SnowflakeId,
    pub session_id: Option<SnowflakeId>,
    #[sqlx(rename = "mem_key")]
    pub key: String,
    pub content: String,
    pub category: String,
    pub importance: f64,
    pub superseded_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Find a memory row by (agent, key).
pub async fn find_memory_by_key(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<Option<AiMemory>> {
    let sql = format!(
        "SELECT {MEMORY_COLS} FROM ai_memories WHERE agent_id = {} AND mem_key = {}{}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        tenant_filter(tenant_id, 3)
    );
    let mut q = sqlx::query_as::<_, AiMemory>(crate::db::safe_sql(&sql))
        .bind(agent_id)
        .bind(key);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_optional(pool).await?)
}

/// Upsert by (tenant, agent, key): update the live row content or insert.
pub async fn store_memory(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    key: &str,
    content: &str,
    category: &str,
    importance: f64,
) -> AppResult<AiMemory> {
    if let Some(existing) = find_memory_by_key(pool, agent_id, key, tenant_id).await? {
        let now = now_utc();
        let result = raisfast_derive::crud_update!(
            pool,
            "ai_memories",
            bind: ["content" => content, "category" => category, "importance" => importance, "updated_at" => &now],
            where: ("id", existing.id),
            tenant: tenant_id
        )?;
        AppError::expect_affected(&result, "ai_memory")?;
        return Ok(existing);
    }

    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "ai_memories",
        [
            "id" => id,
            "agent_id" => agent_id,
            "session_id" => None::<SnowflakeId>,
            "mem_key" => key,
            "content" => content,
            "category" => category,
            "importance" => importance,
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    match find_memory_by_key(pool, agent_id, key, tenant_id).await? {
        Some(m) => Ok(m),
        None => Err(AppError::not_found("ai_memory")),
    }
}

/// Keyword recall over live rows. `query = None` returns most recent first.
/// Keyword matching is portable `LIKE` (case-sensitive on PG/MySQL); a proper
/// keyword/BM25 rank is a later enhancement.
pub async fn recall_memories(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    tenant_id: Option<&str>,
    query: Option<&str>,
    limit: i64,
) -> AppResult<Vec<AiMemory>> {
    let keyword: Option<String> = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| format!("%{q}%"));

    let mut sql = format!(
        "SELECT {MEMORY_COLS} FROM ai_memories WHERE agent_id = {} AND superseded_by IS NULL",
        crate::db::Driver::ph(1)
    );
    let mut n = 2;
    if tenant_id.is_some() {
        sql.push_str(&format!(" AND tenant_id = {}", crate::db::Driver::ph(n)));
        n += 1;
    }
    if keyword.is_some() {
        sql.push_str(&format!(
            " AND (mem_key LIKE {} OR content LIKE {})",
            crate::db::Driver::ph(n),
            crate::db::Driver::ph(n + 1)
        ));
        n += 2;
    }
    sql.push_str(&format!(
        " ORDER BY updated_at DESC LIMIT {}",
        crate::db::Driver::ph(n)
    ));

    let mut q = sqlx::query_as::<_, AiMemory>(crate::db::safe_sql(&sql)).bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    if let Some(pat) = &keyword {
        q = q.bind(pat.as_str()).bind(pat.as_str());
    }
    Ok(q.bind(limit).fetch_all(pool).await?)
}

/// Delete a memory row by (agent, key). Returns whether a row was removed.
pub async fn forget_memory(
    pool: &crate::db::Pool,
    agent_id: SnowflakeId,
    key: &str,
    tenant_id: Option<&str>,
) -> AppResult<bool> {
    let sql = format!(
        "DELETE FROM ai_memories WHERE agent_id = {} AND mem_key = {}{}",
        crate::db::Driver::ph(1),
        crate::db::Driver::ph(2),
        tenant_filter(tenant_id, 3)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql))
        .bind(agent_id)
        .bind(key);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.execute(pool).await?.rows_affected() > 0)
}

/// `" AND tenant_id = {ph}"` for `Some`, `""` for `None`.
fn tenant_filter(tenant_id: Option<&str>, start_index: usize) -> String {
    tenant_id
        .map(|_| format!(" AND tenant_id = {}", crate::db::Driver::ph(start_index)))
        .unwrap_or_default()
}
