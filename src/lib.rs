//! 博客系统核心库 (hello-axum)
//!
//! 基于 Rust + Axum + SQLite 构建的功能完整的博客系统。
//! 架构分层：Handler → Service → Model，中间件处理横切关注点。
//!
//! # 模块结构
//!
//! - `config` — 应用配置，从环境变量加载
//! - `db` — SQLite 连接池初始化
//! - `errors` — 统一错误处理（AppError）、响应格式（ApiResponse）、校验翻译
//! - `handlers` — axum 路由处理器（薄层：提取参数、调用 service、返回响应）
//! - `middleware` — JWT 认证、国际化 locale、IP 限流
//! - `models` — 数据结构定义和数据库查询
//! - `services` — 业务逻辑层
//! - `utils` — 分页、Markdown 渲染等通用工具

#![deny(unsafe_code)]

pub mod config;
pub mod db;
pub mod errors;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod services;
pub mod utils;

use config::app::AppConfig;
use sqlx::SqlitePool;
use std::sync::Arc;

// 初始化 i18n 国际化支持。
// 从项目根目录的 `locales/` 文件夹加载翻译文件（YAML 格式），
// 当请求的 locale 找不到对应翻译时，回退到 `en`。
rust_i18n::i18n!("locales", fallback = "en");

/// 应用全局共享状态，通过 axum State 注入到每个请求。
///
/// 每个请求 handler 可以通过 `State(state): State<AppState>` 获取。
/// `config` 使用 `Arc` 包装以便在多个请求间零成本共享。
#[derive(Clone)]
pub struct AppState {
    /// SQLite 异步连接池
    pub pool: SqlitePool,
    /// 应用配置（Arc 包装，共享只读）
    pub config: Arc<AppConfig>,
}
