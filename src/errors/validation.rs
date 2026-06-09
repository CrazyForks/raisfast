//! Input validation and i18n translation bridge
//!
//! This module wraps the `validator` crate's validation logic, automatically translating
//! validation errors into natural language messages for the current locale, replacing
//! the tedious pattern of manually calling `req.validate().map_err(...)`.
//!
//! # Core functions
//!
//! - [`validate`]: Validates any struct implementing `validator::Validate` and translates errors
//! - [`translate_errors`]: Translates `ValidationErrors` into localized error message list
//! - [`translate_field`]: Translates field names into display names for the current locale

use crate::errors::app_error::{AppError, AppResult};
use validator::ValidationErrors;

/// Perform input validation and automatically translate errors
///
/// Executes `validator::Validate::validate()` on the given request struct. Returns `Ok(())`
/// if validation passes. If validation fails, all errors are translated via [`translate_errors`]
/// into localized messages and combined into a single [`AppError::BadRequest`].
///
/// # Parameters
///
/// - `req` — Reference to a request struct implementing the `validator::Validate` trait
///
/// # Returns
///
/// - `Ok(())` — Validation passed
/// - `Err(AppError::BadRequest)` — Validation failed; message is a semicolon-joined string of all errors
///
/// # Replacement pattern
///
/// Replaces the old manual pattern:
///
/// ```ignore
/// // Old approach (tedious and inconsistent)
/// req.validate().map_err(|e| AppError::BadRequest(e.to_string()))?;
///
/// // New approach (concise with i18n support)
/// validate(&req)?;
/// ```
pub fn validate(req: &dyn validator::Validate) -> AppResult<()> {
    match req.validate() {
        Ok(()) => Ok(()),
        Err(errors) => translate_errors(&errors),
    }
}

/// Translate validation errors into localized message list
///
/// Iterates over each field and its error list in `ValidationErrors`, selects the appropriate
/// i18n translation key based on error type, and translates field names via [`translate_field`]
/// into display names for the current locale.
///
/// # i18n key mapping
///
/// | Validation rule | i18n key                   | Parameters                  |
/// |----------------|---------------------------|-----------------------------|
/// | `required`     | `validation.required`     | `field`                     |
/// | `length` (range) | `validation.length_range` | `field`, `min`, `max`       |
/// | `length` (min) | `validation.min_length`   | `field`, `min`              |
/// | `length` (max) | `validation.max_length`   | `field`, `max`              |
/// | `email`        | `validation.email_invalid`| —                           |
/// | other          | `validation.required`     | `field` (fallback)          |
///
/// # Multiple error handling
///
/// When a single field has multiple validation errors, all translated messages are joined
/// with semicolons `;` and returned as a single `BadRequest` description to the client.
fn translate_errors(errors: &ValidationErrors) -> AppResult<()> {
    let locale = crate::middleware::locale::current_locale();
    rust_i18n::set_locale(&locale);
    let mut messages: Vec<String> = Vec::new();

    for (field, field_errors) in errors.field_errors() {
        let field_name = translate_field(field);
        for error in field_errors {
            let msg = match error.code.as_ref() {
                "required" | "length" => {
                    let min = error
                        .params
                        .get("min")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());
                    let max = error
                        .params
                        .get("max")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());
                    let exact = error
                        .params
                        .get("value")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v.to_string());

                    match (min.as_deref(), max.as_deref()) {
                        (Some(min), Some(max)) if min != max => rust_i18n::t!(
                            "validation.length_range",
                            field = field_name,
                            min = min,
                            max = max
                        )
                        .to_string(),
                        (Some(min), Some(_)) => {
                            rust_i18n::t!("validation.min_length", field = field_name, min = min)
                                .to_string()
                        }
                        (Some(min), None) => {
                            rust_i18n::t!("validation.min_length", field = field_name, min = min)
                                .to_string()
                        }
                        (None, Some(max)) => {
                            rust_i18n::t!("validation.max_length", field = field_name, max = max)
                                .to_string()
                        }
                        _ => {
                            if let Some(v) = exact {
                                rust_i18n::t!("validation.min_length", field = field_name, min = v)
                                    .to_string()
                            } else {
                                rust_i18n::t!("validation.required", field = field_name).to_string()
                            }
                        }
                    }
                }
                "email" => rust_i18n::t!("validation.email_invalid").to_string(),
                "password_strength" => rust_i18n::t!("validation.password_strength").to_string(),
                _ => rust_i18n::t!("validation.required", field = field_name).to_string(),
            };
            messages.push(msg);
        }
    }

    Err(AppError::BadRequest(messages.join("; ")))
}

/// Translate a field name to a display name for the current locale
///
/// Looks up the localized translation of the field name via the i18n key `fields.{field_name}`.
/// For example, the field `email` in a Chinese locale looks up the `fields.email` key to get `"邮箱"`.
///
/// # Parameters
///
/// - `field` — Original field name in the struct (e.g. `"email"`, `"password"`)
///
/// # Returns
///
/// - If the `fields.{field}` key exists, returns the translated field name
/// - If the key does not exist (`rust_i18n::t!` falls back to the key name itself),
///   returns the original field name as a fallback
fn translate_field(field: &str) -> String {
    let key = format!("fields.{field}");
    let translated = rust_i18n::t!(&key);
    if translated == key {
        field.to_string()
    } else {
        translated.to_string()
    }
}
