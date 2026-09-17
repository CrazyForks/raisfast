//! `kb_images` table — document images awaiting/holding VLM recognition
//! (kb-image-recognition-design §3 D5).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbImage {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub kb_id: SnowflakeId,
    pub doc_id: SnowflakeId,
    /// Containing text chunk (external images whose markdown position maps
    /// into a chunk); `None` = document-level (embedded, no position).
    pub chunk_id: Option<SnowflakeId>,
    /// 1-based source page (docreader filenames carry `_pN_`; NULL
    /// unknown).
    pub page: Option<i64>,
    /// Storage key of the saved bytes (`embedded` source); `None` for
    /// external URL images the VLM fetches itself.
    pub storage_key: Option<String>,
    pub mime_type: String,
    pub bytes: Option<i64>,
    /// `embedded` (bytes in storage) | `external` (http URL).
    pub source: String,
    pub original_url: Option<String>,
    pub caption: Option<String>,
    pub ocr_text: Option<String>,
    /// `pending` | `done` | `failed` | `skipped`.
    pub status: String,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Register one image found during parse.
#[derive(Debug, Clone)]
pub struct CreateKbImageCmd {
    pub kb_id: SnowflakeId,
    pub doc_id: SnowflakeId,
    pub chunk_id: Option<SnowflakeId>,
    pub page: Option<i64>,
    pub storage_key: Option<String>,
    pub mime_type: String,
    pub bytes: Option<i64>,
    pub source: &'static str,
    pub original_url: Option<String>,
}

pub async fn insert_image(
    pool: &crate::db::Pool,
    cmd: &CreateKbImageCmd,
    tenant_id: &str,
) -> AppResult<KbImage> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_images",
        [
            "id" => id,
            "kb_id" => cmd.kb_id,
            "doc_id" => cmd.doc_id,
            "chunk_id" => cmd.chunk_id,
            "page" => cmd.page,
            "storage_key" => cmd.storage_key.as_deref(),
            "mime_type" => cmd.mime_type.as_str(),
            "bytes" => cmd.bytes,
            "source" => cmd.source,
            "original_url" => cmd.original_url.as_deref(),
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    find_image_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("kb image insert returned no row")))
}

pub async fn find_image_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbImage>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_images",
        KbImage,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}

/// Retryable images of one document (recognition job work list): pending
/// plus failed — transient channel errors retry on job re-run.
pub async fn find_pending_by_doc(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
    _tenant_id: &str,
) -> AppResult<Vec<KbImage>> {
    let sql = format!(
        "SELECT * FROM kb_images WHERE doc_id = {} AND status IN ('pending', 'failed') \
         ORDER BY {} ASC",
        crate::db::Driver::ph(1),
        "id"
    );
    let rows = sqlx::query_as::<_, KbImage>(crate::db::safe_sql(&sql))
        .bind(i64::from(doc_id))
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok(rows)
}

/// Write back recognition results and mark done.
pub async fn set_image_result(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    caption: Option<&str>,
    ocr_text: Option<&str>,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_images",
        bind: [
            "caption" => caption,
            "ocr_text" => ocr_text,
            "status" => "done",
            "error" => Option::<&str>::None,
            "updated_at" => now
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// Mark failed (recognition error; the document itself stays ready).
pub async fn set_image_failed(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    error: &str,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_images",
        bind: ["status" => "failed", "error" => error, "updated_at" => now],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

/// All images of one document (admin listing).
pub async fn list_images_by_doc(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Vec<KbImage>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_images",
        KbImage,
        where: ("doc_id", doc_id),
        order_by: "id ASC",
        tenant: Some(tenant_id)
    )?)
}

/// Wipe a document's images (re-parse idempotency, doc delete).
pub async fn delete_images_by_doc(pool: &crate::db::Pool, doc_id: SnowflakeId) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_images",
        where: ("doc_id", doc_id)
    )?;
    Ok(())
}

/// Refresh the containing chunk's `image_info` display column after
/// recognition (bounded caption preview; full text lives here + the
/// standalone image chunk).
pub async fn set_chunk_image_info(
    pool: &crate::db::Pool,
    chunk_id: SnowflakeId,
    image_info: Option<Value>,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_chunks",
        bind: ["image_info" => image_info, "updated_at" => now],
        where: ("id", chunk_id)
    )?;
    Ok(())
}

/// One dynamic WHERE term for the admin image listing.
enum ListBind<'a> {
    Bigint(i64),
    Str(&'a str),
}

/// Admin listing: newest first, kb/doc/status filters, names joined for
/// display. Same dynamic-WHERE shape as `document::list_documents`.
pub async fn list_images(
    pool: &crate::db::Pool,
    tenant_id: &str,
    kb_id: Option<SnowflakeId>,
    doc_id: Option<SnowflakeId>,
    status: Option<&str>,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<KbImageListRow>, i64)> {
    let mut clauses = vec![format!("i.tenant_id = {}", Driver::ph(1))];
    let mut binds: Vec<ListBind> = vec![ListBind::Str(tenant_id)];
    if let Some(v) = kb_id {
        let n = binds.len() + 1;
        clauses.push(format!("i.kb_id = {}", Driver::ph(n)));
        binds.push(ListBind::Bigint(i64::from(v)));
    }
    if let Some(v) = doc_id {
        let n = binds.len() + 1;
        clauses.push(format!("i.doc_id = {}", Driver::ph(n)));
        binds.push(ListBind::Bigint(i64::from(v)));
    }
    if let Some(v) = status {
        let n = binds.len() + 1;
        clauses.push(format!("i.status = {}", Driver::ph(n)));
        binds.push(ListBind::Str(v));
    }
    let where_sql = clauses.join(" AND ");

    let count_stmt = format!(
        "SELECT {} FROM kb_images i WHERE {where_sql}",
        Driver::cast_int("COUNT(*)")
    );
    let count_sql = crate::db::safe_sql(&count_stmt);
    let mut count_query = sqlx::query_scalar::<_, i64>(count_sql);
    for b in &binds {
        count_query = match b {
            ListBind::Str(v) => count_query.bind(*v),
            ListBind::Bigint(v) => count_query.bind(*v),
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
        "SELECT i.*, k.name AS kb_name, d.title AS doc_title FROM kb_images i \
         LEFT JOIN kb_knowledge_bases k ON k.id = i.kb_id \
         LEFT JOIN kb_documents d ON d.id = i.doc_id \
         WHERE {where_sql} ORDER BY i.id DESC LIMIT {} OFFSET {}",
        Driver::ph(binds.len() + 1),
        Driver::ph(binds.len() + 2)
    );
    let data_sql = crate::db::safe_sql(&data_stmt);
    let mut data_query = sqlx::query_as::<_, KbImageListRow>(data_sql);
    for b in &binds {
        data_query = match b {
            ListBind::Str(v) => data_query.bind(*v),
            ListBind::Bigint(v) => data_query.bind(*v),
        };
    }
    let rows: Vec<KbImageListRow> = data_query
        .bind(page_size)
        .bind(offset)
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?;
    Ok((rows, total))
}

/// `kb_images` row + display names for the admin list.
#[derive(Debug, Serialize, Clone, sqlx::FromRow)]
pub struct KbImageListRow {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub kb_id: SnowflakeId,
    pub doc_id: SnowflakeId,
    pub chunk_id: Option<SnowflakeId>,
    pub page: Option<i64>,
    pub storage_key: Option<String>,
    pub mime_type: String,
    pub bytes: Option<i64>,
    pub source: String,
    pub original_url: Option<String>,
    pub caption: Option<String>,
    pub ocr_text: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub kb_name: Option<String>,
    pub doc_title: Option<String>,
}
