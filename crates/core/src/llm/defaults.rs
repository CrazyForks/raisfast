//! Default-model option plumbing (design §10.2).
//!
//! The agent / flows / kb subsystems resolve their model through
//! `LlmRouter::call(...).chat(None)` / `.embed(None)`, which falls back to the
//! tenant options `llm.default_chat_model` / `llm.default_embedding_model`.
//! Those options are validated here before they are written so a typo or a
//! modality mismatch surfaces at config time (400) instead of the first call.

use serde::Serialize;

use crate::errors::app_error::{AppError, AppResult};
use crate::llm::models::model::{LlmModel, LlmModelStatus, LlmModelType};

/// Option key for the default chat/vlm model.
pub const CHAT_MODEL_KEY: &str = "llm.default_chat_model";
/// Option key for the default embedding model.
pub const EMBEDDING_MODEL_KEY: &str = "llm.default_embedding_model";
/// Option key for the default image model (media-nodes.md §2.1).
pub const IMAGE_MODEL_KEY: &str = "llm.default_image_model";
/// Option key for the default speech (TTS) model (media-nodes.md §3.1).
pub const SPEECH_MODEL_KEY: &str = "llm.default_speech_model";

/// The resolved default models (a value of `null` means unset).
#[derive(Debug, Clone, Serialize)]
pub struct LlmDefaults {
    pub chat_model: Option<String>,
    pub embedding_model: Option<String>,
    pub image_model: Option<String>,
    pub speech_model: Option<String>,
}

/// Extract a model name from an option value. `null` / blank clears the
/// default (returns `None`); any other non-string value is rejected.
fn model_name_from_value(value: &serde_json::Value) -> AppResult<Option<String>> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) if s.trim().is_empty() => Ok(None),
        serde_json::Value::String(s) => Ok(Some(s.trim().to_owned())),
        _ => Err(AppError::BadRequest(
            "default model must be a string".to_owned(),
        )),
    }
}

/// Read a string option, tenant scope first then global (mirrors
/// `LlmRouter::resolve_default`'s resolution chain).
async fn read_string(
    pool: &crate::db::Pool,
    key: &str,
    tenant: Option<&str>,
) -> AppResult<Option<String>> {
    for scope in [tenant, None] {
        if let Some(row) = crate::models::options::find_by_key(pool, key, scope).await?
            && let Some(v) = row.value.as_str()
            && !v.trim().is_empty()
        {
            return Ok(Some(v.trim().to_owned()));
        }
    }
    Ok(None)
}

/// Resolve both default models for a tenant.
pub async fn read(pool: &crate::db::Pool, tenant: Option<&str>) -> AppResult<LlmDefaults> {
    Ok(LlmDefaults {
        chat_model: read_string(pool, CHAT_MODEL_KEY, tenant).await?,
        embedding_model: read_string(pool, EMBEDDING_MODEL_KEY, tenant).await?,
        image_model: read_string(pool, IMAGE_MODEL_KEY, tenant).await?,
        speech_model: read_string(pool, SPEECH_MODEL_KEY, tenant).await?,
    })
}

/// Validate a write to one of the llm default option keys *before* it lands.
///
/// Non-llm keys (and `null` / blank values that clear a default) are ignored so
/// the generic options writer can call this unconditionally. The model must
/// exist in the tenant's directory and be an enabled row of the expected
/// modality; embedding models must also declare `params.dimension`.
pub async fn validate_option(
    pool: &crate::db::Pool,
    tenant: Option<&str>,
    key: &str,
    value: &serde_json::Value,
) -> AppResult<()> {
    let expected: &[LlmModelType] = match key {
        CHAT_MODEL_KEY => &[LlmModelType::Chat, LlmModelType::Vlm],
        EMBEDDING_MODEL_KEY => &[LlmModelType::Embedding],
        IMAGE_MODEL_KEY => &[LlmModelType::Image],
        SPEECH_MODEL_KEY => &[LlmModelType::Tts],
        _ => return Ok(()),
    };
    let Some(model) = model_name_from_value(value)? else {
        return Ok(());
    };
    let rows = crate::llm::models::model::list_models(pool, tenant, None).await?;
    let found = rows.iter().find(|r| r.name == model).ok_or_else(|| {
        AppError::BadRequest(format!(
            "unknown model `{model}`: create it in the model directory first \
             (Model Directory → New Model)"
        ))
    })?;
    if found.status != LlmModelStatus::Active {
        return Err(AppError::BadRequest(format!(
            "model `{model}` is disabled; enable it in the model directory first"
        )));
    }
    if !expected.contains(&found.model_type) {
        return Err(AppError::BadRequest(format!(
            "model `{model}` is a {} model, not a {} model",
            found.model_type.as_str(),
            expected[0].as_str()
        )));
    }
    if key == EMBEDDING_MODEL_KEY {
        has_dimension(found).then_some(()).ok_or_else(|| {
            AppError::BadRequest(format!(
                "embedding model `{model}` has no params.dimension; set it in the \
                 model directory (required for vector stores)"
            ))
        })?;
    }
    Ok(())
}

/// Whether a directory row declares a positive `params.dimension`.
fn has_dimension(row: &LlmModel) -> bool {
    row.params
        .as_ref()
        .and_then(|p| p.get("dimension"))
        .and_then(serde_json::Value::as_u64)
        .map(|d| d > 0)
        .unwrap_or(false)
}
