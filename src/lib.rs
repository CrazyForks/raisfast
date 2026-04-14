//! 博客系统核心库 (rust-blog)
//!
//! 基于 Rust + Axum 构建的功能完整的博客系统，支持 SQLite / PostgreSQL / MySQL。
//! 架构分层：Handler → Service → Model，中间件处理横切关注点。
//!
//! # 模块结构
//!
//! - `config` — 应用配置，从环境变量加载
//! - `db` — 多数据库连接池初始化（SQLite / PostgreSQL / MySQL）
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
pub mod eventbus;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod plugins;
pub mod services;
pub mod utils;
pub mod worker;

use config::app::AppConfig;
use db::Pool;
use eventbus::EventBus;
use plugins::PluginManager;
use std::sync::Arc;

rust_i18n::i18n!("locales", fallback = "en");

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub plugins: Arc<PluginManager>,
    pub eventbus: EventBus,
}
