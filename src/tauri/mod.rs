//! Tauri 桌面应用适配层
//!
//! 将 rust-blog 的 Service 层暴露为 Tauri Commands，
//! 前端通过 `invoke("command_name", { args })` 调用。
//!
//! # 架构
//!
//! ```text
//! JS invoke → Tauri Command → Service (纯业务逻辑，与 HTTP 模式共享) → Repository → DB
//! ```

pub mod commands;
pub mod setup;

use crate::AppState;

/// Tauri 管理状态（包装 AppState）
pub struct AppManagedState(pub AppState);

/// 统一错误序列化（Tauri command 返回 Result<T, String>）
#[allow(dead_code)]
fn err_to_string(e: crate::errors::app_error::AppError) -> String {
    e.to_string()
}
