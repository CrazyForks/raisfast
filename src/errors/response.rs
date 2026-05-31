//! Unified JSON response format definitions
//!
//! This module defines the unified response structure for all API endpoints, ensuring
//! clients always receive a consistent JSON format:
//!
//! ```json
//! { "code": 0, "message": "Operation successful", "data": { ... } }
//! ```
//!
//! Includes the following core types:
//!
//! - [`ApiResponse`]: Generic response wrapper supporting success and error responses
//! - [`PaginatedData`]: Pagination data envelope for list endpoints

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Unified API response structure
///
/// All API endpoints use this structure to wrap return data, ensuring consistent response format.
///
/// # Fields
///
/// - `code` — Business status code (`0` for success, `40000`–`50000` for various errors)
/// - `message` — Human-readable status description (supports i18n multi-language translation)
/// - `data` — Response payload data; `None` for error responses
///
/// # Type Parameters
///
/// - `T` — Serialization type for response data, must implement [`Serialize`]
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: Option<T>,
}

impl<T: Serialize> ApiResponse<T> {
    /// Construct a success response
    ///
    /// Creates a success response with `code` set to `0`, the `data` field containing
    /// the provided data, and `message` translated via the i18n key `messages.success`
    /// to the current locale's "success" message.
    ///
    /// # Parameters
    ///
    /// - `data` — Business data to return to the client
    ///
    /// # Returns
    ///
    /// Returns a complete [`ApiResponse`] instance that can be used directly as an Axum handler
    /// return value (via the [`IntoResponse`] implementation).
    pub fn success(data: T) -> Self {
        let locale = crate::middleware::locale::current_locale();
        rust_i18n::set_locale(&locale);
        let message = rust_i18n::t!("messages.success").to_string();
        Self {
            code: 0,
            message,
            data: Some(data),
        }
    }
}

/// Pagination data envelope
///
/// Used as response data wrapper for list endpoints, containing pagination metadata
/// and the current page's data list. Clients can use this to implement pagination,
/// calculate total pages, etc.
///
/// # Fields
///
/// - `items` — Data list for the current page
/// - `total` — Total number of records matching the query
/// - `page` — Current page number (1-based)
/// - `page_size` — Number of records per page
///
/// # Example
///
/// ```ignore
/// let paginated = PaginatedData {
///     items: posts,
///     total: 100,
///     page: 1,
///     page_size: 20,
/// };
/// Ok(ApiResponse::success(paginated))
/// ```
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct PaginatedData<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

/// Converts `ApiResponse` into an Axum HTTP response
///
/// Serializes to JSON and sets `Content-Type: application/json`.
/// The HTTP status code for success responses defaults to `200 OK`.
impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}
