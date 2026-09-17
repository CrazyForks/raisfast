//! `kb_images` table — document images awaiting/holding VLM recognition
//! (kb-image-recognition-design §3 D5).

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// Pending images of one document (recognition job work list).
pub async fn find_pending_by_doc(
    pool: &crate::db::Pool,
    doc_id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Vec<KbImage>> {
    Ok(raisfast_derive::crud_find_all!(
        pool,
        "kb_images",
        KbImage,
        where: AND(("doc_id", doc_id), ("status", "pending")),
        order_by: "id ASC",
        tenant: Some(tenant_id)
    )?)
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
