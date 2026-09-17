//! `kb_wiki_pages` table — distilled, human-governed knowledge pages
//! (kb-technical-design §7).
//!
//! Lifecycle: `draft` (LLM distillation output, needs human review per P1)
//! → `published` (indexed into retrieval) → `stale` (a source document
//! changed; awaiting re-distillation) / `archived` (rejected or retired).
//! Versioning reuses `content_revisions` with `content_type='kb_wiki_page'`
//! (decision D2); line-level diffs via `similar` (§7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbWikiPage {
    pub id: SnowflakeId,
    pub kb_id: SnowflakeId,
    pub tenant_id: String,
    pub title: String,
    pub slug: String,
    pub folder: Option<String>,
    /// `draft` | `published` | `stale` | `archived`.
    pub status: String,
    pub content: String,
    pub summary: Option<String>,
    pub linked_page_ids: Option<Value>,
    pub current_revision: i64,
    pub reviewed_by: Option<SnowflakeId>,
    pub created_by: Option<SnowflakeId>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create-page command (repo `commands::*Cmd` pattern).
#[derive(Debug, Clone)]
pub struct CreateWikiPageCmd {
    pub kb_id: SnowflakeId,
    pub title: String,
    pub slug: String,
    pub content: String,
    pub summary: Option<String>,
    pub linked_page_ids: Option<Value>,
    pub created_by: Option<SnowflakeId>,
}

pub async fn create_page(
    pool: &crate::db::Pool,
    cmd: &CreateWikiPageCmd,
    tenant_id: &str,
) -> AppResult<KbWikiPage> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_wiki_pages",
        [
            "id" => id,
            "kb_id" => cmd.kb_id,
            "title" => cmd.title.as_str(),
            "slug" => cmd.slug.as_str(),
            "status" => "draft",
            "content" => cmd.content.as_str(),
            "summary" => cmd.summary.as_deref(),
            "linked_page_ids" => cmd.linked_page_ids.clone(),
            "created_by" => cmd.created_by,
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    find_page_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("wiki page insert returned no row")))
}

pub async fn find_page_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbWikiPage>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_wiki_pages",
        KbWikiPage,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}

/// One dynamic WHERE term: clause fragment + its bind value
/// (same shape as `kb_run::list_runs` / `document::list_documents`).
enum Bind<'a> {
    Str(&'a str),
    Bigint(i64),
}

/// Tenant-scoped wiki page listing. `kb_id`/`status` are optional filters —
/// both absent lists every page of the tenant (admin "全部" view).
pub async fn list_pages(
    pool: &crate::db::Pool,
    kb_id: Option<SnowflakeId>,
    status: Option<&str>,
    page: i64,
    page_size: i64,
    tenant_id: &str,
) -> AppResult<(Vec<KbWikiPage>, i64)> {
    let mut clauses: Vec<String> = vec![format!("tenant_id = {}", Driver::ph(1))];
    let mut binds: Vec<Bind> = vec![Bind::Str(tenant_id)];
    if let Some(v) = kb_id {
        let n = binds.len() + 1;
        clauses.push(format!("kb_id = {}", Driver::ph(n)));
        binds.push(Bind::Bigint(i64::from(v)));
    }
    if let Some(v) = status {
        let n = binds.len() + 1;
        clauses.push(format!("status = {}", Driver::ph(n)));
        binds.push(Bind::Str(v));
    }
    let where_sql = clauses.join(" AND ");

    let count_stmt = format!(
        "SELECT {} FROM kb_wiki_pages WHERE {where_sql}",
        Driver::cast_int("COUNT(*)")
    );
    let count_sql = crate::db::safe_sql(&count_stmt);
    let mut count_query = sqlx::query_scalar::<_, i64>(count_sql);
    for b in &binds {
        count_query = match b {
            Bind::Str(v) => count_query.bind(*v),
            Bind::Bigint(v) => count_query.bind(*v),
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
        "SELECT * FROM kb_wiki_pages WHERE {where_sql} ORDER BY updated_at DESC LIMIT {} OFFSET {}",
        Driver::ph(binds.len() + 1),
        Driver::ph(binds.len() + 2)
    );
    let data_sql = crate::db::safe_sql(&data_stmt);
    let mut data_query = sqlx::query_as::<_, KbWikiPage>(data_sql);
    for b in &binds {
        data_query = match b {
            Bind::Str(v) => data_query.bind(*v),
            Bind::Bigint(v) => data_query.bind(*v),
        };
    }
    let rows: Vec<KbWikiPage> = data_query
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok((rows, total))
}

pub async fn update_page_content(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    content: &str,
    summary: Option<&str>,
    status: &str,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_wiki_pages",
        bind: ["content" => content, "summary" => summary, "status" => status, "updated_at" => now],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// Re-distill upsert: rewrite a known-slug page as a fresh draft
/// (title/content/summary/links; status back to `draft` for re-approval —
/// 人审闸门语义). Old indexed chunks keep serving the last approved content
/// until re-publish. Same slug refreshes, never duplicates
/// [抄WK:wiki_ingest_cite.go known-slug union 语义].
pub async fn update_page_draft(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    title: &str,
    content: &str,
    summary: Option<&str>,
    linked_page_ids: Option<Value>,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_wiki_pages",
        bind: [
            "title" => title,
            "content" => content,
            "summary" => summary,
            "linked_page_ids" => linked_page_ids,
            "status" => "draft",
            "updated_at" => now
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// Publish: bump revision, record reviewer.
pub async fn publish_page(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    reviewed_by: SnowflakeId,
    _tenant_id: &str,
) -> AppResult<i64> {
    let now = crate::utils::tz::now_utc();
    // First publish keeps revision 1; re-publishes (edits, re-distills) bump.
    let sql = format!(
        "UPDATE kb_wiki_pages SET status = 'published', \
         current_revision = CASE WHEN status = 'published' THEN current_revision + 1 ELSE current_revision END, \
         reviewed_by = {}, updated_at = {} WHERE id = {}",
        Driver::ph(1),
        Driver::ph(2),
        Driver::ph(3)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(i64::from(reviewed_by))
        .bind(now)
        .bind(i64::from(id))
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    let row: (i64,) = sqlx::query_as(crate::db::safe_sql(&format!(
        "SELECT {} FROM kb_wiki_pages WHERE id = {}",
        Driver::cast_int("current_revision"),
        Driver::ph(1)
    )))
    .bind(i64::from(id))
    .fetch_one(pool)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok(row.0)
}

/// Mark pages depending on a document as stale (incremental invalidation,
/// §7: source change → downstream visible).
pub async fn mark_stale_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<Vec<i64>> {
    let sql = format!(
        "UPDATE kb_wiki_pages SET status = 'stale', updated_at = {} WHERE status = 'published' \
         AND id IN (SELECT page_id FROM kb_wiki_sources WHERE doc_id = {})",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(crate::utils::tz::now_utc())
        .bind(i64::from(doc_id))
        .execute(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    let sql = format!(
        "SELECT {} FROM kb_wiki_pages WHERE status = 'stale' AND id IN \
         (SELECT page_id FROM kb_wiki_sources WHERE doc_id = {})",
        Driver::cast_int("id"),
        Driver::ph(1)
    );
    let ids: Vec<i64> = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .bind(i64::from(doc_id))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok(ids)
}

/// Change only the status (archive/stale transitions that keep content).
pub async fn set_page_status(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    status: &str,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_wiki_pages",
        bind: ["status" => status, "updated_at" => now],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn delete_pages_by_kb(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_wiki_pages",
        where: ("kb_id", kb_id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}
