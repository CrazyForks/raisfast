//! Unified error handling module
//!
//! This module provides the unified error handling infrastructure for the entire application,
//! including the following sub-modules:
//!
//! - [`app_error`]: Defines the `AppError` enum and its HTTP response conversion logic
//! - [`response`]: Defines the unified JSON response format (`ApiResponse`, `PaginatedData`)
//! - [`validation`]: Bridges the `validator` crate with i18n translation for input validation
//!
//! All handler-layer errors are uniformly converted to HTTP responses via `AppError`,
//! ensuring clients receive a consistent JSON error structure.

pub mod app_error;
pub mod response;
pub mod validation;
