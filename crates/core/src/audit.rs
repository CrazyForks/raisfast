//! Audit log module
//!
//! Persistently records admin operations (who did what, when).
//! EventBus subscribers automatically write events to the `audit_log` table.

pub mod handler;
pub mod model;
pub mod service;

pub use service::AuditService;
