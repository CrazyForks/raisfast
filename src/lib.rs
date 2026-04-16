//! 博客系统核心库 (rust-blog)
//!
//! 基于 Rust + Axum 构建的功能完整的博客系统，支持 `SQLite` / `PostgreSQL` / `MySQL`。
//! 架构分层：Handler → Service → Repository → Model → DB。

#![deny(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

pub mod audit;
pub mod cache;
pub mod commands;
pub mod config;
pub mod content_type;
pub mod db;
pub mod errors;
pub mod eventbus;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod plugins;
pub mod repositories;
pub mod search;
pub mod services;
pub mod utils;
pub mod webhook;
pub mod worker;

use audit::AuditService;
use config::app::AppConfig;
use content_type::ContentTypeRegistry;
use db::Pool;
use eventbus::EventBus;
use plugins::PluginManager;
use repositories::{
    CategoryRepository, CommentRepository, MediaRepository, PostRepository, RefreshTokenRepository,
    TagRepository, UserRepository,
};
use search::SearchEngine;
use services::options::OptionsService;
use services::rbac::RbacService;
use services::tenant::TenantService;
use std::sync::Arc;
use webhook::WebhookService;

rust_i18n::i18n!("locales", fallback = "en");

/// 应用全局共享状态
///
/// 通过 axum `State` 注入到所有 handler。所有 Repository 以 trait object 形式存储，
/// 支持运行时替换实现（缓存装饰器、mock 等）。
#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub plugins: Arc<PluginManager>,
    pub eventbus: EventBus,
    pub post_repo: Arc<dyn PostRepository>,
    pub user_repo: Arc<dyn UserRepository>,
    pub category_repo: Arc<dyn CategoryRepository>,
    pub tag_repo: Arc<dyn TagRepository>,
    pub comment_repo: Arc<dyn CommentRepository>,
    pub media_repo: Arc<dyn MediaRepository>,
    pub refresh_token_repo: Arc<dyn RefreshTokenRepository>,
    pub search: Arc<dyn SearchEngine>,
    pub content_type_registry: Arc<ContentTypeRegistry>,
    pub options: Arc<OptionsService>,
    pub rbac: Arc<RbacService>,
    pub tenant: Arc<TenantService>,
    pub audit: Arc<AuditService>,
    pub webhook: Arc<WebhookService>,
}
