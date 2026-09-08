//! `kb_wiki_sources` table — page ↔ source-document provenance spans
//! (kb-technical-design §7, trust chain layer 2).

use serde::{Deserialize, Serialize};

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
            "span_start" => 0,
            "span_end" => 0,
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
