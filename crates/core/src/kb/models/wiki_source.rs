//! `kb_wiki_sources` table — page ↔ source-document provenance spans
//! (kb-technical-design §7, trust chain layer 2).

use serde::{Deserialize, Serialize};

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbWikiSource {
    pub id: SnowflakeId,
    pub page_id: SnowflakeId,
    pub page_revision: i64,
    pub doc_id: SnowflakeId,
    pub chunk_id: Option<SnowflakeId>,
    pub span_start: i64,
    pub span_end: i64,
    pub created_at: Timestamp,
}

/// Record a page↔doc provenance link (doc-level granularity in v1).
pub async fn link_source(
    pool: &crate::db::Pool,
    page_id: SnowflakeId,
    page_revision: i64,
    doc_id: SnowflakeId,
) -> AppResult<()> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_wiki_sources",
        [
            "id" => id,
            "page_id" => page_id,
            "page_revision" => page_revision,
            "doc_id" => doc_id,
            "span_start" => 0_i64,
            "span_end" => 0_i64,
            "created_at" => now
        ]
    )?;
    Ok(())
}

/// Pages (ids) that cite a document — stale marking input.
pub async fn find_page_ids_by_doc(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
) -> AppResult<Vec<i64>> {
    let rows = raisfast_derive::crud_find_all!(
        pool,
        "kb_wiki_sources",
        KbWikiSource,
        where: ("doc_id", doc_id)
    )?;
    Ok(rows.into_iter().map(|r| i64::from(r.page_id)).collect())
}

/// Drop provenance links of a deleted document (its pages are marked
/// stale separately — the links themselves must not outlive the doc).
pub async fn delete_sources_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_wiki_sources",
        where: ("doc_id", doc_id)
    )?;
    Ok(())
}

/// Drop every provenance link of a KB (the table has no kb_id column;
/// resolve via the KB's pages and documents before those rows vanish).
pub async fn delete_sources_by_kb(pool: &crate::db::Pool, kb_id: SnowflakeId) -> AppResult<()> {
    let sql = format!(
        "DELETE FROM kb_wiki_sources WHERE page_id IN (SELECT id FROM kb_wiki_pages WHERE kb_id = {}) \
         OR doc_id IN (SELECT id FROM kb_documents WHERE kb_id = {})",
        Driver::ph(1),
        Driver::ph(2)
    );
    sqlx::query(crate::db::safe_sql(&sql))
        .bind(i64::from(kb_id))
        .bind(i64::from(kb_id))
        .execute(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    Ok(())
}
