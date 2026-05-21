//! Application core error type definitions
//!
//! This module defines the [`AppError`] enum, serving as the unified error type
//! for handler and business logic layers. Each variant maps to a corresponding
//! HTTP status code and supports i18n message translation via `rust_i18n`.
//! Implements [`IntoResponse`] trait for direct use as Axum handler return types.
//!
//! # Error Code Convention
//!
//! Error codes follow the specification in `docs/guide.md`, range 40000–50000:
//!
//! | Variant            | HTTP Status               | Code    |
//! |-------------------|--------------------------|---------|
//! | `BadRequest`      | 400 Bad Request          | 40000   |
//! | `Unauthorized`    | 401 Unauthorized         | 40100   |
//! | `Forbidden`       | 403 Forbidden            | 40300   |
//! | `NotFound`        | 404 Not Found            | 40400   |
//! | `MethodNotAllowed`| 405 Method Not Allowed   | 40500   |
//! | `Conflict`        | 409 Conflict             | 40900   |
//! | `PayloadTooLarge` | 413 Payload Too Large    | 41300   |
//! | `TooManyRequests` | 429 Too Many Requests    | 42900   |
//! | `Internal`        | 500 Internal Server Error| 50000   |
//! | `ServiceUnavailable`| 503 Service Unavailable | 50300   |
//!
//! # i18n Message Format
//!
//! Each variant looks up its translation key via `rust_i18n::t!`, for example:
//! - `BadRequest` → `errors.bad_request`
//! - `Unauthorized` → `errors.unauthorized`
//! - `NotFound` → translates the resource name via `resources.{key}`, then substitutes into `errors.not_found`

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::middleware::locale::current_locale;

/// Application unified error type
///
/// Uses `thiserror` to derive the `Error` trait. Each variant corresponds to
/// a category of HTTP error. Automatically converts to JSON responses via
/// [`IntoResponse`] at handler boundaries.
///
/// # Variant Descriptions
///
/// - [`BadRequest`](AppError::BadRequest): 400 — Invalid request parameters, with description
/// - [`Unauthorized`](AppError::Unauthorized): 401 — No valid authentication credentials provided
/// - [`Forbidden`](AppError::Forbidden): 403 — Authenticated but no permission to access the resource
/// - [`NotFound`](AppError::NotFound): 404 — Requested resource does not exist, with resource identifier
/// - [`Conflict`](AppError::Conflict): 409 — Resource conflict (e.g., unique constraint violation), with message key
/// - [`Internal`](AppError::Internal): 500 — Unexpected internal server error, wrapping `anyhow::Error`
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// 400 Bad Request — Request parameter validation failed or business rule not satisfied
    ///
    /// The attached `String` is the error description, translated via i18n key `errors.bad_request`.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// 401 Unauthorized — No or invalid authentication credentials provided
    #[error("unauthorized")]
    Unauthorized,
    /// 403 Forbidden — Authenticated user lacks permission for this operation
    #[error("forbidden")]
    Forbidden,
    /// 404 Not Found — Requested resource does not exist
    ///
    /// The attached `String` is the resource identifier key (e.g., `"user"`), which is
    /// first translated to a localized resource name via `resources.{key}`,
    /// then substituted into the `errors.not_found` template.
    #[error("not found: {0}")]
    NotFound(String),
    /// 409 Conflict — Resource conflict (e.g., unique constraint violation, duplicate operation)
    ///
    /// The attached `String` is the message key (e.g., `"duplicate_entry"`), which is
    /// translated to a localized message via `messages.{key}`,
    /// then substituted into the `errors.conflict` template.
    #[error("conflict: {0}")]
    Conflict(String),
    /// 405 Method Not Allowed — Request method is not allowed for this resource
    #[error("method not allowed: {0}")]
    MethodNotAllowed(String),
    /// 413 Payload Too Large — Request body exceeds size limit
    #[error("payload too large: {0}")]
    PayloadTooLarge(String),
    /// 429 Too Many Requests — Request rate exceeds throttle threshold
    #[error("too many requests: {0}")]
    TooManyRequests(String),
    /// 503 Service Unavailable — Service temporarily unavailable
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    /// 500 Internal Server Error — Unexpected internal server error
    ///
    /// Automatically converted from `anyhow::Error` via `#[from]`, avoiding manual mapping.
    /// Internal details are hidden from clients; only a generic error message is returned.
    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    /// Convenience constructor for `NotFound` error.
    ///
    /// # Arguments
    ///
    /// - `resource` — Resource name key (e.g., `"post"`), used for i18n translation
    #[must_use]
    pub fn not_found(resource: &str) -> Self {
        AppError::NotFound(resource.into())
    }

    /// Check DELETE/UPDATE affected rows; returns `NotFound` if 0.
    ///
    /// Used in model layer `delete()` and `update_status()` functions,
    /// to avoid manually checking `rows_affected() == 0` each time.
    pub fn expect_affected(result: &crate::db::DbQueryResult, resource: &str) -> AppResult<()> {
        if result.rows_affected() == 0 {
            Err(AppError::NotFound(resource.into()))
        } else {
            Ok(())
        }
    }

    /// Translate the error message based on the current locale
    ///
    /// Each `AppError` variant has a corresponding i18n translation key. This method
    /// looks up the translation based on the `locale` parameter and substitutes
    /// dynamic parameters (e.g., resource name, error description) into the template.
    ///
    /// # Arguments
    ///
    /// - `locale` — Target locale identifier (e.g., `"en"`, `"zh-CN"`), typically obtained from request middleware
    ///
    /// # Returns
    ///
    /// Returns the translated error message string. If the translation key does not exist,
    /// `rust_i18n::t!` falls back to the key name itself.
    fn i18n_message(&self, locale: &str) -> String {
        match self {
            AppError::BadRequest(msg) => {
                rust_i18n::t!("errors.bad_request", locale = locale, message = msg).to_string()
            }
            AppError::Unauthorized => {
                rust_i18n::t!("errors.unauthorized", locale = locale).to_string()
            }
            AppError::Forbidden => rust_i18n::t!("errors.forbidden", locale = locale).to_string(),
            AppError::NotFound(resource_key) => {
                let res_key = format!("resources.{resource_key}");
                let resource = rust_i18n::t!(&res_key, locale = locale);
                rust_i18n::t!("errors.not_found", locale = locale, resource = resource).to_string()
            }
            AppError::Conflict(msg_key) => {
                let msg_key_full = format!("messages.{msg_key}");
                let message = rust_i18n::t!(&msg_key_full, locale = locale);
                rust_i18n::t!("errors.conflict", locale = locale, message = message).to_string()
            }
            AppError::PayloadTooLarge(detail) => {
                let base = rust_i18n::t!("errors.payload_too_large", locale = locale).to_string();
                format!("{base}: {detail}")
            }
            AppError::TooManyRequests(detail) => {
                let base = rust_i18n::t!("errors.too_many_requests", locale = locale).to_string();
                format!("{base}: {detail}")
            }
            AppError::MethodNotAllowed(detail) => {
                let base = rust_i18n::t!("errors.method_not_allowed", locale = locale).to_string();
                format!("{base}: {detail}")
            }
            AppError::ServiceUnavailable(detail) => {
                let base = rust_i18n::t!("errors.service_unavailable", locale = locale).to_string();
                format!("{base}: {detail}")
            }
            AppError::Internal(err) => {
                let base: String = rust_i18n::t!("errors.internal", locale = locale).into();
                if std::env::var("APP_ENV").unwrap_or_default() == "production" {
                    base
                } else {
                    format!("{base}: {err}")
                }
            }
        }
    }
}

/// Convert `AppError` to an Axum HTTP response
///
/// Implements the [`IntoResponse`] trait, enabling `AppError` to be used directly
/// as a handler return type. Conversion flow:
///
/// 1. Determine HTTP status code and business error code (40000–50000 range) based on variant
/// 2. Translate error message via [`i18n_message`](AppError::i18n_message)
/// 3. Log the error (`tracing::error!`)
/// 4. Construct the [`ErrorBody`] JSON response body
///
/// # Response Format
///
/// ```json
/// { "code": 40000, "message": "Error description", "data": null }
/// ```
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, 40000),
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, 40100),
            AppError::Forbidden => (StatusCode::FORBIDDEN, 40300),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, 40400),
            AppError::MethodNotAllowed(_) => (StatusCode::METHOD_NOT_ALLOWED, 40500),
            AppError::Conflict(_) => (StatusCode::CONFLICT, 40900),
            AppError::PayloadTooLarge(_) => (StatusCode::PAYLOAD_TOO_LARGE, 41300),
            AppError::TooManyRequests(_) => (StatusCode::TOO_MANY_REQUESTS, 42900),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, 50000),
            AppError::ServiceUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, 50300),
        };

        let locale = current_locale();
        let message = self.i18n_message(&locale);

        match status {
            StatusCode::BAD_REQUEST
            | StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::NOT_FOUND
            | StatusCode::CONFLICT => {
                tracing::warn!(%code, %message, error = %self, "client error");
            }
            _ => {
                tracing::error!(%code, %message, error = %self, "server error");
            }
        }

        let body = ErrorBody {
            code,
            message,
            data: (),
        };

        (status, Json(body)).into_response()
    }
}

/// Error response body structure
///
/// Maintains the same JSON structure as success responses. The `data` field is
/// always `()` (empty), enabling clients to use unified parsing logic.
///
/// # Serialization Example
///
/// ```json
/// { "code": 40400, "message": "Resource not found", "data": null }
/// ```
#[derive(Serialize)]
struct ErrorBody {
    code: i32,
    message: String,
    data: (),
}

/// Automatic mapping from database errors to `AppError`
///
/// Converts `sqlx::Error` into semantic `AppError` variants, avoiding manual
/// database error handling in business code:
///
/// - `RowNotFound` → [`AppError::NotFound`] (resource identifier defaults to `"resource"`)
/// - `UNIQUE constraint failed` → [`AppError::Conflict`] (message key is `"duplicate_entry"`)
/// - Other database errors → [`AppError::Internal`] (wrapped as `anyhow::Error`)
///
/// # Design Decision
///
/// Database layer details are encapsulated within `AppError`. Handler and service layers
/// can propagate SQL errors as appropriate HTTP responses using the `?` operator.
impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("resource".into()),
            sqlx::Error::Database(ref e) => {
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key value") {
                    tracing::warn!(constraint = ?e.constraint(), "unique constraint violation");
                    AppError::Conflict("duplicate_entry".into())
                } else {
                    AppError::Internal(err.into())
                }
            }
            other => AppError::Internal(other.into()),
        }
    }
}

/// Application-level Result type alias
///
/// All handler and service functions use `AppResult<T>` as their return type,
/// equivalent to `Result<T, AppError>`, simplifying function signatures and
/// ensuring consistent error handling.
pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_convenience() {
        let err = AppError::not_found("post");
        match err {
            AppError::NotFound(r) => assert_eq!(r, "post"),
            _ => panic!("expected NotFound"),
        }
    }

    #[test]
    fn expect_affected_zero_rows() {
        let result = crate::db::DbQueryResult::default();
        let outcome = AppError::expect_affected(&result, "user");
        assert!(outcome.is_err());
        match outcome.unwrap_err() {
            AppError::NotFound(r) => assert_eq!(r, "user"),
            _ => panic!("expected NotFound"),
        }
    }

    #[test]
    fn from_sqlx_row_not_found() {
        let err: AppError = sqlx::Error::RowNotFound.into();
        match err {
            AppError::NotFound(_) => {}
            _ => panic!("expected NotFound"),
        }
    }

    #[test]
    fn from_anyhow() {
        let err = AppError::Internal(anyhow::anyhow!("boom"));
        match err {
            AppError::Internal(e) => assert_eq!(e.to_string(), "boom"),
            _ => panic!("expected Internal"),
        }
    }
}
