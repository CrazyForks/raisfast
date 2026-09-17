//! `kb_knowledge_bases` table — KB row model and queries
//! (kb-technical-design §2, decision D4).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db::{DbDriver, Driver};
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct KbKnowledgeBase {
    pub id: SnowflakeId,
    pub tenant_id: String,
    pub name: String,
    /// What this KB is for (human note, shown in admin).
    pub description: Option<String>,
    pub slug: String,
    /// `document` | `faq` (wiki is an indexing-strategy flag on document
    /// KBs, decision D4 [抄WK:knowledgebase.go IsWikiEnabled]).
    pub kind: String,
    pub indexing_strategy: Option<Value>,
    pub chunking_config: Option<Value>,
    pub embedding_model: Option<String>,
    pub embedding_dim: Option<i64>,
    /// S5 rerank model for this KB (query-time behavior — mutable, unlike
    /// the pinned embedding config). Empty = fall back to the global
    /// `RAISFAST_KB_RERANK_MODEL` default; neither set = S5 passthrough.
    pub rerank_model: Option<String>,
    /// Per-KB rerank window override (§6.1.4); `None` = global default.
    pub rerank_window: Option<i64>,
    /// Per-KB rerank score floor override; `None` = global default.
    pub rerank_threshold: Option<f64>,
    /// S9 generation model for this KB (WK conversation-level ChatModelID
    /// analog — our ask is stateless and KB-bound, so the KB row is the
    /// mount point). Empty = global `RAISFAST_KB_CHAT_MODEL` default →
    /// tenant default.
    pub chat_model: Option<String>,
    /// Wiki distillation synthesis model for this KB
    /// [抄WK:wiki_ingest_batch.go SynthesisModelID→SummaryModelID 级联].
    /// Empty = global `RAISFAST_KB_DISTILL_MODEL` → tenant default.
    pub distill_model: Option<String>,
    /// VLM image recognition config
    /// [抄WK:knowledgebase.go image_processing_config + VLMConfig 形态]:
    /// `{enabled, model, caption_language, custom_instructions}`.
    /// enabled=false or no resolvable model → recognition off.
    pub image_config: Option<serde_json::Value>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Create-KB command (repo `commands::*Cmd` pattern [抄RF]).
#[derive(Debug, Clone)]
pub struct CreateKbCmd {
    pub name: String,
    /// What this KB is for (human note, shown in admin).
    pub description: Option<String>,
    pub slug: String,
    pub kind: String,
    pub indexing_strategy: Option<Value>,
    pub embedding_model: Option<String>,
    pub embedding_dim: Option<i64>,
    pub rerank_model: Option<String>,
    pub rerank_window: Option<i64>,
    pub rerank_threshold: Option<f64>,
    pub chat_model: Option<String>,
    pub distill_model: Option<String>,
    pub image_config: Option<serde_json::Value>,
}

pub async fn create_kb(
    pool: &crate::db::Pool,
    cmd: &CreateKbCmd,
    tenant_id: &str,
) -> AppResult<KbKnowledgeBase> {
    let (id, now) = (
        crate::utils::id::new_snowflake_id(),
        crate::utils::tz::now_utc(),
    );
    raisfast_derive::crud_insert!(
        pool,
        "kb_knowledge_bases",
        [
            "id" => id,
            "name" => cmd.name.as_str(),
            "description" => cmd.description.as_deref(),
            "slug" => cmd.slug.as_str(),
            "kind" => cmd.kind.as_str(),
            "indexing_strategy" => cmd.indexing_strategy.clone(),
            "embedding_model" => cmd.embedding_model.as_deref(),
            "embedding_dim" => cmd.embedding_dim,
            "rerank_model" => cmd.rerank_model.as_deref(),
            "rerank_window" => cmd.rerank_window,
            "rerank_threshold" => cmd.rerank_threshold,
            "chat_model" => cmd.chat_model.as_deref(),
            "distill_model" => cmd.distill_model.as_deref(),
            "image_config" => cmd.image_config.clone(),
            "created_at" => now
        ],
        tenant: Some(tenant_id)
    )?;
    find_kb_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("kb insert returned no row")))
}

/// Update-KB command — mutable metadata (name/slug/description/status) and
/// the rerank trio (query-time behavior, freely editable). kind and
/// embedding config stay immutable after creation (WeKnora
/// `vector_store_id` precedent: vectors are pinned to model+dim).
#[derive(Debug, Clone)]
pub struct UpdateKbCmd {
    pub name: String,
    pub description: Option<String>,
    pub slug: String,
    pub status: String,
    pub rerank_model: Option<String>,
    pub rerank_window: Option<i64>,
    pub rerank_threshold: Option<f64>,
    pub chat_model: Option<String>,
    pub distill_model: Option<String>,
    pub image_config: Option<serde_json::Value>,
}

pub async fn update_kb(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    cmd: &UpdateKbCmd,
    tenant_id: &str,
) -> AppResult<()> {
    let now = crate::utils::tz::now_utc();
    raisfast_derive::crud_update!(
        pool,
        "kb_knowledge_bases",
        bind: [
            "name" => cmd.name.as_str(),
            "description" => cmd.description.as_deref(),
            "slug" => cmd.slug.as_str(),
            "status" => cmd.status.as_str(),
            "rerank_model" => cmd.rerank_model.as_deref(),
            "rerank_window" => cmd.rerank_window,
            "rerank_threshold" => cmd.rerank_threshold,
            "chat_model" => cmd.chat_model.as_deref(),
            "distill_model" => cmd.distill_model.as_deref(),
            "image_config" => cmd.image_config.clone(),
            "updated_at" => now
        ],
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn find_kb_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: &str,
) -> AppResult<Option<KbKnowledgeBase>> {
    Ok(raisfast_derive::crud_find!(
        pool,
        "kb_knowledge_bases",
        KbKnowledgeBase,
        where: ("id", id),
        tenant: Some(tenant_id)
    )?)
}

pub async fn delete_kb(pool: &crate::db::Pool, id: SnowflakeId, tenant_id: &str) -> AppResult<()> {
    raisfast_derive::crud_delete!(
        pool,
        "kb_knowledge_bases",
        where: ("id", id),
        tenant: Some(tenant_id)
    )?;
    Ok(())
}

pub async fn list_kbs(
    pool: &crate::db::Pool,
    page: i64,
    page_size: i64,
    tenant_id: &str,
) -> AppResult<(Vec<KbKnowledgeBase>, i64)> {
    Ok(raisfast_derive::crud_query_paged!(
        pool,
        KbKnowledgeBase,
        table: "kb_knowledge_bases",
        order_by: "created_at DESC",
        tenant: Some(tenant_id),
        page: page,
        page_size: page_size
    ))
}

/// Batch resolve KB names (chunks list column). Ids missing from the
/// table are silently absent from the map.
pub async fn find_kb_names_by_ids(
    pool: &crate::db::Pool,
    ids: &[i64],
) -> AppResult<std::collections::HashMap<i64, String>> {
    let mut out = std::collections::HashMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let placeholders: Vec<String> = (1..=ids.len()).map(Driver::ph).collect();
    let sql = format!(
        "SELECT id, name FROM kb_knowledge_bases WHERE id IN ({})",
        placeholders.join(", ")
    );
    let mut query = sqlx::query_as::<_, (i64, String)>(crate::db::safe_sql(&sql));
    for id in ids {
        query = query.bind(id);
    }
    for (id, name) in query
        .fetch_all(pool)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!(e.to_string())))?
    {
        out.insert(id, name);
    }
    Ok(out)
}
