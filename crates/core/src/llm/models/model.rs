//! `llm_models` model — model directory + pricing (design §5.4).

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::{Timestamp, now_utc};

define_enum!(
    LlmModelType {
        Chat = "chat",
        Embedding = "embedding",
        Rerank = "rerank",
        Asr = "asr",
        Vlm = "vlm",
        Image = "image",
    }
);

define_enum!(
    LlmPriceMode {
        Ratio = "ratio",
        PerCall = "per_call",
    }
);

define_enum!(
    LlmModelStatus {
        Active = "active",
        Disabled = "disabled",
    }
);

/// One model-directory row: type, pricing and params metadata.
#[derive(Debug, Serialize, Deserialize, Clone, sqlx::FromRow)]
pub struct LlmModel {
    pub id: SnowflakeId,
    pub tenant_id: Option<String>,
    pub name: String,
    pub model_type: LlmModelType,
    pub price_mode: LlmPriceMode,
    pub model_ratio: f64,
    pub completion_ratio: f64,
    pub cache_ratio: Option<f64>,
    pub cache_write_ratio: Option<f64>,
    pub call_price: Option<f64>,
    pub params: Option<serde_json::Value>,
    pub status: LlmModelStatus,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

/// Payload for creating / updating a directory row.
#[derive(Debug)]
pub struct ModelChanges {
    pub name: String,
    pub model_type: LlmModelType,
    pub price_mode: LlmPriceMode,
    pub model_ratio: f64,
    pub completion_ratio: f64,
    pub cache_ratio: Option<f64>,
    pub cache_write_ratio: Option<f64>,
    pub call_price: Option<f64>,
    pub params: Option<serde_json::Value>,
    pub status: LlmModelStatus,
}

/// Create a directory row and return it.
pub async fn create_model(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    m: ModelChanges,
) -> AppResult<LlmModel> {
    let id = crate::utils::id::new_snowflake_id();
    let now = now_utc();
    raisfast_derive::crud_insert!(
        pool,
        "llm_models",
        [
            "id" => id,
            "name" => m.name,
            "model_type" => m.model_type.as_str(),
            "price_mode" => m.price_mode.as_str(),
            "model_ratio" => m.model_ratio,
            "completion_ratio" => m.completion_ratio,
            "cache_ratio" => m.cache_ratio,
            "cache_write_ratio" => m.cache_write_ratio,
            "call_price" => m.call_price,
            "params" => m.params,
            "status" => m.status.as_str(),
            "created_at" => &now,
            "updated_at" => &now
        ],
        tenant: tenant_id
    )?;
    find_by_id(pool, id, tenant_id)
        .await?
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("llm_model insert vanished")))
}

/// Find a directory row by id (tenant-scoped).
pub async fn find_by_id(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<Option<LlmModel>> {
    let result: Option<LlmModel> = raisfast_derive::crud_find!(
        pool,
        "llm_models",
        LlmModel,
        where: ("id", id),
        tenant: tenant_id
    )?;
    Ok(result)
}

/// Find a directory row by (tenant, name).
pub async fn find_by_name(
    pool: &crate::db::Pool,
    tenant_id: &str,
    name: &str,
) -> AppResult<Option<LlmModel>> {
    let result: Option<LlmModel> = raisfast_derive::crud_find!(
        pool,
        "llm_models",
        LlmModel,
        where: ("name", name),
        tenant: Some(tenant_id)
    )?;
    Ok(result)
}

/// List directory rows of a tenant, optionally filtered by type (filtering
/// happens in memory — directory scale is small and `crud_list!` has no
/// WHERE section).
pub async fn list_models(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    model_type: Option<&LlmModelType>,
) -> AppResult<Vec<LlmModel>> {
    let rows: Vec<LlmModel> = raisfast_derive::crud_list!(
        pool,
        "llm_models",
        LlmModel,
        order_by: "name",
        tenant: tenant_id
    )?;
    Ok(match model_type {
        Some(t) => rows.into_iter().filter(|r| r.model_type == *t).collect(),
        None => rows,
    })
}

/// Update a directory row.
pub async fn update_model(
    pool: &crate::db::Pool,
    tenant_id: Option<&str>,
    id: SnowflakeId,
    m: ModelChanges,
) -> AppResult<()> {
    let now = now_utc();
    let result = raisfast_derive::crud_update!(
        pool,
        "llm_models",
        bind: [
            "name" => m.name,
            "model_type" => m.model_type.as_str(),
            "price_mode" => m.price_mode.as_str(),
            "model_ratio" => m.model_ratio,
            "completion_ratio" => m.completion_ratio,
            "cache_ratio" => m.cache_ratio,
            "cache_write_ratio" => m.cache_write_ratio,
            "call_price" => m.call_price,
            "params" => m.params,
            "status" => m.status.as_str(),
            "updated_at" => &now
        ],
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_model")
}

/// Delete a directory row by id (tenant-scoped).
pub async fn delete_model(
    pool: &crate::db::Pool,
    id: SnowflakeId,
    tenant_id: Option<&str>,
) -> AppResult<()> {
    let result = raisfast_derive::crud_delete!(
        pool,
        "llm_models",
        where: ("id", id),
        tenant: tenant_id
    )?;
    AppError::expect_affected(&result, "llm_model")
}
