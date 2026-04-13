//! 统一错误处理模块
//!
//! 本模块为整个应用提供统一的错误处理基础设施，包含以下子模块：
//!
//! - [`app_error`]：定义 `AppError` 枚举及其 HTTP 响应转换逻辑
//! - [`response`]：定义统一的 JSON 响应格式（`ApiResponse`、`PaginatedData`）
//! - [`validation`]：桥接 `validator` crate 与 i18n 翻译的输入校验工具
//!
//! 所有 handler 层的错误均通过 `AppError` 统一转换为 HTTP 响应，
//! 确保客户端收到一致的 JSON 错误结构。

pub mod app_error;
pub mod response;
pub mod validation;
