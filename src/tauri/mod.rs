//! Tauri desktop application adapter layer
//!
//! Exposes the raisfast Service layer as Tauri Commands,
//! invoked by the frontend via `invoke("command_name", { args })`.
//!
//! # Architecture
//!
//! ```text
//! JS invoke → Tauri Command → Service (pure business logic, shared with HTTP mode) → Repository → DB
//! ```

pub mod commands;
pub mod setup;

use crate::AppState;

/// Tauri managed state (wraps AppState)
pub struct AppManagedState(pub AppState);

/// Unified error serialization (Tauri commands return Result<T, String>)
#[allow(dead_code)]
fn err_to_string(e: crate::errors::app_error::AppError) -> String {
    e.to_string()
}
