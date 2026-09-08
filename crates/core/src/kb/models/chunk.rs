//! `kb_chunks` table — retrieval-unit row model (document/faq/wiki_page
//! kinds [抄WK:types/chunk.go ChunkType*]), parent-child linkage, and the
//! embedding BLOB codec (SQL is the vector truth, decision D3).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::AppResult;
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbChunk {
    pub id: SnowflakeId,
    pub kb_id: SnowflakeId,
    pub doc_id: Option<SnowflakeId>,
    pub faq_id: Option<SnowflakeId>,
    pub wiki_page_id: Option<SnowflakeId>,
    /// `document` | `faq` | `wiki_page` [抄WK:types/chunk.go ChunkType*].
    pub kind: String,
    pub parent_id: Option<SnowflakeId>,
    pub seq: i64,
    pub content: String,
    pub breadcrumb: Option<String>,
    pub byte_start: i64,
    pub byte_end: i64,
    pub questions: Option<Value>,
    /// f32-little-endian packed vector (SQL is the truth, D3); `None` on
    /// parent chunks (children carry the embedding, parent-child retrieval).
    pub embedding: Option<Vec<u8>>,
    pub embedding_model: Option<String>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Insert a chunk row and return its id (id allocated by caller so the
/// parent-child ids are known before insert).
#[allow(clippy::too_many_arguments)]
pub async fn insert_chunk(pool: &crate::db::Pool, chunk: &KbChunkInsert) -> AppResult<()> {
    raisfast_derive::crud_insert!(
        pool,
        "kb_chunks",
        [
            "id" => chunk.id,
            "kb_id" => chunk.kb_id,
            "doc_id" => chunk.doc_id,
            "faq_id" => chunk.faq_id,
            "wiki_page_id" => chunk.wiki_page_id,
            "kind" => chunk.kind.as_str(),
            "parent_id" => chunk.parent_id,
            "seq" => chunk.seq,
            "content" => chunk.content.as_str(),
            "breadcrumb" => chunk.breadcrumb.as_deref(),
            "byte_start" => chunk.byte_start,
            "byte_end" => chunk.byte_end,
            "questions" => chunk.questions.clone(),
            "embedding" => chunk.embedding.clone(),
            "embedding_model" => chunk.embedding_model.as_deref(),
            "created_at" => chunk.created_at
        ]
    )?;
    Ok(())
}

/// New chunk row (id/created_at pre-allocated for parent-child linking).
#[derive(Debug, Clone)]
pub struct KbChunkInsert {
    pub id: SnowflakeId,
    pub kb_id: SnowflakeId,
    pub doc_id: Option<SnowflakeId>,
    pub faq_id: Option<SnowflakeId>,
    pub wiki_page_id: Option<SnowflakeId>,
    pub kind: String,
    pub parent_id: Option<SnowflakeId>,
    pub seq: i64,
    pub content: String,
    pub breadcrumb: Option<String>,
    pub byte_start: i64,
    pub byte_end: i64,
    pub questions: Option<Value>,
    pub embedding: Option<Vec<u8>>,
    pub embedding_model: Option<String>,
    pub created_at: crate::utils::tz::Timestamp,
}

pub async fn find_chunks_by_doc(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
) -> AppResult<Vec<KbChunk>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_chunks",
        KbChunk,
        where: ("doc_id", doc_id),
        order_by: "seq ASC"
    )?)
}

pub async fn delete_chunks_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_chunks", where: ("doc_id", doc_id))?;
    Ok(())
}

pub async fn count_chunks_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<i64> {
    let sql = format!(
        "SELECT {} FROM kb_chunks WHERE doc_id = {}",
        Driver::cast_int("COUNT(*)"),
        crate::db::Driver::ph(1)
    );
    let (n,): (i64,) = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(i64::from(doc_id))
        .fetch_one(pool)
        .await?;
    Ok(n)
}

/// Pack f32 vectors as little-endian bytes for the `embedding` BLOB.
pub fn pack_embedding(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for v in vec {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Unpack the `embedding` BLOB back to f32 vectors.
pub fn unpack_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Fetch chunks by unit ids (recall candidate hydration for the QA pipeline).
pub async fn find_chunks_by_ids(pool: &crate::db::Pool, ids: &[i64]) -> AppResult<Vec<KbChunk>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT * FROM kb_chunks WHERE status = 'active' AND id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, KbChunk>(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(pool).await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
    })?;
    Ok(rows)
}

/// Fetch a page's units (publish re-index wipe).
pub async fn find_chunks_by_page(
    pool: &crate::db::Pool,
    page_id: SnowflakeId,
) -> AppResult<Vec<KbChunk>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_chunks",
        KbChunk,
        where: ("wiki_page_id", page_id),
        order_by: "seq ASC"
    )?)
}

/// Delete all units of a page.
pub async fn delete_chunks_by_page(pool: &crate::db::Pool, page_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_chunks", where: ("wiki_page_id", page_id))?;
    Ok(())
}

/// Persist the embedding BLOB on a chunk (SQL is the truth, D3).
pub async fn update_embedding(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    vector: &[f32],
) -> AppResult<()> {
    raisfast_derive::crud_update!(
        pool,
        "kb_chunks",
        bind: ["embedding" => pack_embedding(vector)],
        where: ("id", id)
    )?;
    Ok(())
}

/// Fetch one chunk by id (chunk editing).
pub async fn find_chunk_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
) -> AppResult<Option<KbChunk>> {
    Ok(raisfast_derive::crud_find!(pool, "kb_chunks", KbChunk, where: ("id", id))?)
}

/// Update chunk content (chunk editing; re-embed happens in the service).
pub async fn update_content(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    content: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_chunks",
        bind: ["content" => content, "updated_at" => now],
        where: ("id", id)
    )?;
    Ok(())
}

/// Fetch/delete a FAQ's units (idempotent re-index per FAQ).
pub async fn find_chunks_by_faq(
    pool: &crate::db::Pool,
    faq_id: SnowflakeId,
) -> AppResult<Vec<KbChunk>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_chunks",
        KbChunk,
        where: ("faq_id", faq_id)
    )?)
}

pub async fn delete_chunks_by_faq(pool: &crate::db::Pool, faq_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_chunks", where: ("faq_id", faq_id))?;
    Ok(())
}
