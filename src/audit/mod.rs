//! 审计日志模块
//!
//! 持久化记录管理员操作（谁做了什么、什么时候）。
//! EventBus 订阅者自动将事件写入 `audit_log` 表。

pub mod handler;
pub mod model;
pub mod service;

pub use service::AuditService;
