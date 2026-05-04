//! 工具模块
//!
//! 提供项目中通用的工具函数和类型，包括：
//!
//! - **pagination** — 分页参数的提取与校验
//! - **markdown** — Markdown 转 HTML 的渲染管线（含 XSS 防护）

pub mod auth;
pub mod crypto;
pub mod id;
pub mod markdown;
pub mod pagination;
pub mod tz;
