//! `kb_chunks` table — retrieval-unit row model (document/faq/wiki_page
//! kinds [抄WK:types/chunk.go ChunkType*]), parent-child linkage, and the
//! embedding BLOB codec (SQL is the vector truth, decision D3).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
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
    /// Bounded display info of the images attached to this chunk
    /// (caption/ocr preview/url).
    pub image_info: Option<serde_json::Value>,
    /// 1-based source page (PDF via page anchors; NULL unknown).
    pub page: Option<i64>,
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
            "image_info" => chunk.image_info.clone(),
            "page" => chunk.page,
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
    pub image_info: Option<Value>,
    pub page: Option<i64>,
    pub created_at: crate::utils::tz::Timestamp,
}

/// Reader-view types: one page of blocks + the whole-doc page index
/// (质量验证视图，kb-parser-engines §3.3/阅读器).
#[derive(serde::Serialize)]
pub struct ReaderBlock {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub chunk_id: Option<i64>,
    pub breadcrumb: Option<String>,
    pub content: Option<String>,
    pub image_id: Option<i64>,
    pub caption: Option<String>,
    pub ocr_text: Option<String>,
    pub source: Option<String>,
}

#[derive(serde::Serialize)]
pub struct ReaderPageIndex {
    /// `None` = chunks without page anchors (single-page documents).
    pub page: Option<i64>,
    pub chunks: i64,
    pub images: i64,
}

#[derive(serde::Serialize)]
pub struct ReaderPage {
    pub page: Option<i64>,
    pub total_pages: i64,
    pub blocks: Vec<ReaderBlock>,
    pub index: Vec<ReaderPageIndex>,
}

/// Page-scoped reader query: leaf chunks of one page (document kind) in
/// document order, with the page's images interleaved after their anchor
/// chunks; page index for the navigator.
/// First reader page: NULL-page group when present (无锚点文档单页模式),
/// else the smallest anchor page. `None` = empty doc.
pub async fn reader_first_page(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
) -> AppResult<Option<i64>> {
    let sql = format!(
        "SELECT {} FROM (SELECT page FROM kb_chunks WHERE doc_id = {doc} AND kind = 'document' \
         UNION ALL SELECT page FROM kb_images WHERE doc_id = {doc}) t",
        Driver::cast_int("COUNT(*)"),
        doc = Driver::ph(1),
    );
    let has: Option<i64> = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .bind(i64::from(doc_id))
        .fetch_optional(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    if has.is_none() {
        return Ok(None);
    }
    let sql = format!(
        "SELECT MIN(page) FROM (SELECT page FROM kb_chunks WHERE doc_id = {doc} AND kind = 'document' \
         UNION ALL SELECT page FROM kb_images WHERE doc_id = {doc}) t",
        doc = Driver::ph(1),
    );
    // MIN ignores NULLs; NULL-only docs keep the NULL group.
    let min_page: Option<i64> = sqlx::query_scalar(crate::db::safe_sql(&sql))
        .bind(i64::from(doc_id))
        .fetch_one(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok(min_page)
}

pub async fn reader_page(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
    page: Option<i64>,
) -> AppResult<ReaderPage> {
    // Leaf filter: children are retrieval units; parents without children
    // are standalone. Placeholders numbered by appearance order — the
    // leaf subquery's doc_id gets its own bind (一占位一号，防错位).
    let mut clauses = vec![
        format!("doc_id = {}", Driver::ph(1)),
        "kind = 'document'".to_string(),
    ];
    let mut binds: Vec<i64> = vec![i64::from(doc_id)];
    // leaf subquery re-binds the same doc id.
    clauses.push(format!(
        "(parent_id IS NOT NULL OR id NOT IN (SELECT parent_id FROM kb_chunks WHERE doc_id = {} AND parent_id IS NOT NULL))",
        Driver::ph(2)
    ));
    binds.push(i64::from(doc_id));
    match page {
        Some(n) => {
            clauses.push(format!("page = {}", Driver::ph(binds.len() + 1)));
            binds.push(n);
        }
        None => clauses.push("page IS NULL".to_string()),
    }
    let where_sql = clauses.join(" AND ");

    let mut chunks_query = sqlx::query_as::<_, KbChunk>(crate::db::safe_sql(&format!(
        "SELECT * FROM kb_chunks WHERE {where_sql} ORDER BY byte_start, seq"
    )));
    for b in &binds {
        chunks_query = chunks_query.bind(*b);
    }
    let chunks: Vec<KbChunk> = chunks_query
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;

    // 该页图片：docreader 图片页码与锚点 chunk 页码一致（注册时落库）。
    let mut img_sql = format!("SELECT * FROM kb_images WHERE doc_id = {}", Driver::ph(1));
    match page {
        Some(_) => img_sql.push_str(&format!(" AND page = {}", Driver::ph(2))),
        None => img_sql.push_str(" AND page IS NULL"),
    }
    let images: Vec<crate::kb::models::image::KbImage> =
        sqlx::query_as(crate::db::safe_sql(&img_sql))
            .bind(i64::from(doc_id))
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;

    // Anchor map: chunk_id → images (insert after the anchor chunk).
    let mut anchored: std::collections::HashMap<i64, Vec<&crate::kb::models::image::KbImage>> =
        std::collections::HashMap::new();
    let mut loose: Vec<&crate::kb::models::image::KbImage> = Vec::new();
    for img in &images {
        match img.chunk_id {
            Some(cid) => anchored.entry(i64::from(cid)).or_default().push(img),
            None => loose.push(img),
        }
    }

    let mut blocks: Vec<ReaderBlock> = Vec::with_capacity(chunks.len() + images.len());
    for c in &chunks {
        blocks.push(ReaderBlock {
            kind: "text",
            chunk_id: Some(i64::from(c.id)),
            breadcrumb: c.breadcrumb.clone(),
            content: Some(c.content.clone()),
            image_id: None,
            caption: None,
            ocr_text: None,
            source: None,
        });
        if let Some(imgs) = anchored.get(&i64::from(c.id)) {
            for img in imgs {
                blocks.push(ReaderBlock {
                    kind: "image",
                    chunk_id: None,
                    breadcrumb: None,
                    content: None,
                    image_id: Some(i64::from(img.id)),
                    caption: img.caption.clone(),
                    ocr_text: img.ocr_text.clone(),
                    source: Some(img.source.clone()),
                });
            }
        }
    }
    for img in &loose {
        blocks.push(ReaderBlock {
            kind: "image",
            chunk_id: None,
            breadcrumb: None,
            content: None,
            image_id: Some(i64::from(img.id)),
            caption: img.caption.clone(),
            ocr_text: img.ocr_text.clone(),
            source: Some(img.source.clone()),
        });
    }

    // Page index (whole doc, no page filter): chunks per page + images per
    // page. Leaf condition without a page clause; two binds (doc twice for
    // the leaf subquery is folded into one by reusing the outer doc filter
    // — the subquery correlates nothing, so one bind suffices with the doc
    // filter repeated via SQL literal aliasing is avoided; keep two binds).
    let leaf_idx_sql = format!(
        "(parent_id IS NOT NULL OR id NOT IN (SELECT parent_id FROM kb_chunks WHERE doc_id = {} AND parent_id IS NOT NULL))",
        Driver::ph(2)
    );
    let chunk_idx: Vec<(Option<i64>, i64)> = {
        let sql = format!(
            "SELECT page, {} FROM kb_chunks WHERE doc_id = {} AND kind = 'document' AND {leaf_idx_sql} GROUP BY page",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        );
        let q = sqlx::query_as::<_, (Option<i64>, i64)>(crate::db::safe_sql(&sql))
            .bind(i64::from(doc_id));
        q.fetch_all(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?
    };
    let img_idx: Vec<(Option<i64>, i64)> = {
        let sql = format!(
            "SELECT page, {} FROM kb_images WHERE doc_id = {} GROUP BY page",
            Driver::cast_int("COUNT(*)"),
            Driver::ph(1)
        );
        sqlx::query_as::<_, (Option<i64>, i64)>(crate::db::safe_sql(&sql))
            .bind(i64::from(doc_id))
            .fetch_all(pool)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?
    };
    let mut index: std::collections::HashMap<Option<i64>, (i64, i64)> =
        std::collections::HashMap::new();
    for (p, n) in chunk_idx {
        index.entry(p).or_insert((0, 0)).0 += n;
    }
    for (p, n) in img_idx {
        index.entry(p).or_insert((0, 0)).1 += n;
    }
    let mut idx: Vec<ReaderPageIndex> = index
        .into_iter()
        .map(|(page, (c, i))| ReaderPageIndex {
            page,
            chunks: c,
            images: i,
        })
        .collect();
    idx.sort_by_key(|e| e.page.unwrap_or(0));

    let total_pages = idx.iter().filter_map(|e| e.page).max().unwrap_or(1).max(1);
    Ok(ReaderPage {
        page: page.or_else(|| idx.first().and_then(|e| e.page)),
        total_pages,
        blocks,
        index: idx,
    })
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

/// Paged chunk list for the standalone chunks page: optional KB filter.
/// Pagination counts PARENT GROUPS, not raw rows — children are fetched
/// per-page by the caller (`find_children_by_parents`) so a page boundary
/// can never split a parent-child family (the UI folds children under
/// parents; orphan children would silently vanish at raw-row page cuts).
pub async fn list_chunks_paged(
    pool: &crate::db::Pool,
    kb_id: Option<SnowflakeId>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<KbChunk>, i64)> {
    match kb_id {
        Some(kb) => Ok(raisfast_derive::crud_query_paged!(
            pool,
            KbChunk,
            table: "kb_chunks",
            where: AND(("kb_id", kb), ("parent_id", IS_NULL)),
            order_by: "kb_id ASC, doc_id ASC, seq ASC",
            page: page,
            page_size: page_size
        )),
        None => Ok(raisfast_derive::crud_query_paged!(
            pool,
            KbChunk,
            table: "kb_chunks",
            where: ("parent_id", IS_NULL),
            order_by: "kb_id ASC, doc_id ASC, seq ASC",
            page: page,
            page_size: page_size
        )),
    }
}

/// Children of the given parent chunks, bulk (one IN query). Companions of
/// `list_chunks_paged` — the page's families are assembled by the caller.
pub async fn find_children_by_parents(
    pool: &crate::db::Pool,
    parent_ids: &[i64],
) -> AppResult<Vec<KbChunk>> {
    if parent_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders: Vec<String> = (1..=parent_ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT * FROM kb_chunks WHERE parent_id IN ({}) ORDER BY seq ASC",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, KbChunk>(crate::db::safe_sql(&sql));
    for id in parent_ids {
        query = query.bind(id);
    }
    query
        .fetch_all(pool)
        .await
        .map_err(|e| crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string())))
}

/// Per-(doc, kind) chunk census for the index viewer — the SQL truth side
/// every index is reconciled against. `doc_id` is `None` for units without
/// a document (faq/wiki_page); the handler folds those into KB-level totals.
#[derive(Debug, sqlx::FromRow)]
pub struct ChunkKindStat {
    pub doc_id: Option<i64>,
    pub kind: String,
    /// All rows of this (doc, kind) regardless of status.
    pub total: i64,
    /// `status = 'active'` rows — the FTS-indexable set.
    pub active: i64,
    /// Active rows carrying an embedding — the vector-indexable set.
    pub embedded: i64,
}

/// Grouped chunk census for one KB (see [`ChunkKindStat`]).
pub async fn doc_kind_stats(
    pool: &crate::db::Pool,
    kb_id: SnowflakeId,
) -> AppResult<Vec<ChunkKindStat>> {
    let sql = format!(
        "SELECT doc_id, kind, {} AS total, {} AS active, {} AS embedded \
         FROM kb_chunks WHERE kb_id = {} GROUP BY doc_id, kind",
        Driver::cast_int("COUNT(*)"),
        Driver::cast_int("SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END)"),
        Driver::cast_int(
            "SUM(CASE WHEN embedding IS NOT NULL AND status = 'active' THEN 1 ELSE 0 END)"
        ),
        Driver::ph(1)
    );
    let rows: Vec<ChunkKindStat> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(i64::from(kb_id))
        .fetch_all(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    Ok(rows)
}

/// Adjacent parents `(prev, next)` of one parent chunk within its document,/// by `seq` order — the neighbor walk for S7 short-context expansion
/// [抄WK:merge_expand.go PreChunkID/NextChunkID 链语义；declared delta: our
/// chunks carry no sibling-id columns, `(doc_id, seq)` adjacency replaces it].
pub async fn find_parent_neighbors(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
    seq: i64,
) -> AppResult<(Option<KbChunk>, Option<KbChunk>)> {
    let prev_sql = format!(
        "SELECT * FROM kb_chunks WHERE doc_id = {} AND parent_id IS NULL AND seq < {} \
         ORDER BY seq DESC LIMIT 1",
        Driver::ph(1),
        Driver::ph(2)
    );
    let prev = sqlx::query_as::<_, KbChunk>(crate::db::safe_sql(&prev_sql))
        .bind(i64::from(doc_id))
        .bind(seq)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    let next_sql = format!(
        "SELECT * FROM kb_chunks WHERE doc_id = {} AND parent_id IS NULL AND seq > {} \
         ORDER BY seq ASC LIMIT 1",
        Driver::ph(1),
        Driver::ph(2)
    );
    let next = sqlx::query_as::<_, KbChunk>(crate::db::safe_sql(&next_sql))
        .bind(i64::from(doc_id))
        .bind(seq)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
        })?;
    Ok((prev, next))
}

pub async fn delete_chunks_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_chunks", where: ("doc_id", doc_id))?;
    Ok(())
}

pub async fn delete_chunks_by_kb(pool: &crate::db::Pool, kb_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(pool, "kb_chunks", where: ("kb_id", kb_id))?;
    Ok(())
}

/// Children of one parent chunk (document kind links parents; flat elsewhere).
pub async fn find_children_by_parent(
    pool: &crate::db::Pool,
    parent_id: SnowflakeId,
) -> AppResult<Vec<KbChunk>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_chunks",
        KbChunk,
        where: ("parent_id", parent_id),
        order_by: "seq ASC"
    )?)
}

/// Delete chunks by ids (single chunk delete + cascade of its children).
pub async fn delete_chunks_by_ids(pool: &crate::db::Pool, ids: &[i64]) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "DELETE FROM kb_chunks WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    query.execute(pool).await.map_err(|e| {
        crate::errors::app_error::AppError::Internal(anyhow::anyhow!(e.to_string()))
    })?;
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
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
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

#[cfg(test)]
mod reader_sql_tests {
    use super::*;

    #[tokio::test]
    async fn reader_page_sql_parses() {
        let pool = crate::test_pool!();
        let doc_id = SnowflakeId(crate::utils::id::new_id());
        // 空文档：SQL 合法执行，返回空块集（regression：页索引 SQL 曾含
        // 未插值的 `{leaf}` 模板 → SQLite unrecognized token "{"）。
        let page = reader_page(&pool, doc_id, Some(2)).await.unwrap();
        assert!(page.blocks.is_empty());
        assert_eq!(page.total_pages, 1);
        let none_page = reader_page(&pool, doc_id, None).await.unwrap();
        assert!(none_page.blocks.is_empty());
    }
}
