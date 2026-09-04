//! Conversation session model (`ai_sessions`): an agent + owner conversation
//! with a durable cursor (`last_seq`) and transient `status` (`open`/`running`).
//! Multi-tenant (tenant_id filter).

use serde::{Deserialize, Serialize};

use crate::db::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

/// One conversation session.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct AiSession {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub agent_id: SnowflakeId,
    pub user_id: SnowflakeId,
    pub title: String,
    pub status: String,
    pub meta: Option<serde_json::Value>,
    pub last_seq: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_active_at: Timestamp,
}

/// Create a session and return it.
#[allow(clippy::too_many_arguments)]
pub async fn create_session(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
    user_id: SnowflakeId,
    title: &str,
) -> AppResult<AiSession> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "ai_sessions",
        [
            "id" => id,
            "agent_id" => agent_id,
            "user_id" => user_id,
            "title" => title,
            "status" => "open",
            "last_seq" => 0i64,
            "created_at" => &now,
            "updated_at" => &now,
            "last_active_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_session_by_id(pool, id, tenant_id).await
}

/// Find a session by id (tenant-scoped).
pub async fn find_session_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<AiSession> {
    let result: AiSession = raisfast_derive::crud_find_one!(
        pool,
        "ai_sessions",
        AiSession,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// List sessions of an agent, most recently active first.
pub async fn list_sessions(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    agent_id: SnowflakeId,
) -> AppResult<Vec<AiSession>> {
    let sql = format!(
        "SELECT id, tenant_id, agent_id, user_id, title, status, meta, last_seq, \
         created_at, updated_at, last_active_at FROM ai_sessions \
         WHERE agent_id = {}{} ORDER BY last_active_at DESC",
        crate::db::Driver::ph(1),
        tenant_filter(tenant_id, 2)
    );
    let mut q = sqlx::query_as::<_, AiSession>(crate::db::safe_sql(&sql)).bind(agent_id);
    if let Some(tid) = tenant_id {
        q = q.bind(tid);
    }
    Ok(q.fetch_all(pool).await?)
}

/// Set status (e.g. `running` ↔ `open`). Fails if zero rows affected.
pub async fn set_session_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
    status: &str,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "ai_sessions",
        bind: ["status" => status, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "ai_session")
}

/// Replace the session `meta` JSON (e.g. durable context-fold state `ctx`).
pub async fn update_session_meta(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
    meta: serde_json::Value,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "ai_sessions",
        bind: ["meta" => meta, "updated_at" => &now],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "ai_session")
}

/// Idempotent cursor advance: only ever moves forward.
pub async fn advance_last_seq(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
    new_seq: i64,
) -> AppResult<()> {
    let sql = format!(
        "UPDATE ai_sessions SET last_seq = {}, updated_at = {}, last_active_at = {} \
         WHERE id = {} AND last_seq < {}{}",
        crate::db::Driver::ph(1),
        crate::db::Driver::now_fn(),
        crate::db::Driver::now_fn(),
        crate::db::Driver::ph(2),
        crate::db::Driver::ph(3),
        tenant_filter(tenant_id, 4)
    );
    let mut q = sqlx::query(crate::db::safe_sql(&sql))
        .bind(new_seq) // $1 last_seq = ?
        .bind(id) // $2 id = ?
        .bind(new_seq); // $3 last_seq < ?
    if let Some(tid) = tenant_id {
        q = q.bind(tid); // $4 tenant_id = ?
    }
    let _ = q.execute(pool).await?;
    Ok(())
}

/// `" AND tenant_id = {ph}"` for `Some`, `""` for `None`.
fn tenant_filter(tenant_id: Option<&str>, start_index: usize) -> String {
    tenant_id
        .map(|_| format!(" AND tenant_id = {}", crate::db::Driver::ph(start_index)))
        .unwrap_or_default()
}
