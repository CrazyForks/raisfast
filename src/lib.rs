//! raisfast 全栈开发底座核心库 (raisfast)
//!
//! 基于 Rust + Axum 构建的高性能全栈开发底座，支持 `SQLite` / `PostgreSQL` / `MySQL`。
//! 架构分层：Handler → Service → Repository → Model → DB。
//!
//! 同时支持两种运行模式：
//! - **server** — 独立 HTTP 服务器（Axum）
//! - **tauri** — Tauri 桌面应用后端（共享 Service 层）

#![deny(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

#[macro_use]
mod macros;

pub mod app;
pub mod aspects;
pub mod audit;
pub mod cache;
pub mod commands;
pub mod config;
pub mod constants;
pub mod content_type;
pub mod db;
pub mod dto;
pub mod errors;
pub mod eventbus;
pub mod graphql;
pub mod handlers;
pub mod middleware;
pub mod models;
pub mod notifier;
pub mod oauth;
pub mod plugins;
pub mod protocols;
pub mod repositories;
pub mod search;
pub mod server;
pub mod services;
pub mod storage;
pub mod utils;
pub mod webhook;
pub mod worker;
pub mod workflow;

pub mod admin_spa;

#[cfg(feature = "tauri")]
pub mod tauri;

#[inline]
pub(crate) fn _brand() -> String {
    let k0: u8 = 0x5A;
    let k1: u8 = 0xA5;
    let p0 = utils::tz::_B0;
    let p1 = utils::id::_B1;
    let mut v = Vec::with_capacity(p0.len() + p1.len());
    for b in p0 {
        v.push(b ^ k0);
    }
    for b in p1 {
        v.push(b ^ k1);
    }
    String::from_utf8(v).unwrap_or_default()
}

use app::ServiceRegistry;
use audit::AuditService;
use config::app::AppConfig;
use content_type::ContentTypeRegistry;
use db::Pool;
use eventbus::EventBus;
use notifier::{EmailSender, SmsSender};
use oauth::OAuthProviderRegistry;
use plugins::PluginManager;
use repositories::{
    CategoryRepository, CommentRepository, MediaRepository, PostRepository, RefreshTokenRepository,
    TagRepository, UserRepository, WalletRepository,
};
use search::SearchEngine;
use services::options::OptionsService;
use services::rbac::RbacService;
use services::tenant::TenantService;
use std::sync::Arc;
use storage::Storage;
use webhook::WebhookService;
use workflow::WorkflowService;

pub use cache::CacheStore;

rust_i18n::i18n!("locales", fallback = "en");

/// 应用全局共享状态
///
/// 通过 axum `State` 注入到所有 handler。所有 Repository 以 trait object 形式存储，
/// 支持运行时替换实现（缓存装饰器、mock 等）。
#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub jwt_decoding_key: jsonwebtoken::DecodingKey,
    pub plugins: Arc<PluginManager>,
    pub eventbus: EventBus,
    pub post_repo: Arc<dyn PostRepository>,
    pub user_repo: Arc<dyn UserRepository>,
    pub category_repo: Arc<dyn CategoryRepository>,
    pub tag_repo: Arc<dyn TagRepository>,
    pub comment_repo: Arc<dyn CommentRepository>,
    pub media_repo: Arc<dyn MediaRepository>,
    pub refresh_token_repo: Arc<dyn RefreshTokenRepository>,
    pub wallet_repo: Arc<dyn WalletRepository>,
    pub search: Arc<dyn SearchEngine>,
    pub content_type_registry: Arc<ContentTypeRegistry>,
    pub aspect_engine: Arc<crate::aspects::engine::AspectEngine>,
    pub protocol_registry: Arc<crate::protocols::ProtocolRegistry>,
    pub options: Arc<OptionsService>,
    pub rbac: Arc<RbacService>,
    pub tenant: Arc<TenantService>,
    pub audit: Arc<AuditService>,
    pub webhook: Arc<WebhookService>,
    pub workflow: Arc<WorkflowService>,
    pub storage: Arc<dyn Storage>,
    pub cache: Arc<dyn CacheStore>,
    pub cms_cache: Arc<dashmap::DashMap<String, (serde_json::Value, std::time::Instant)>>,
    pub oauth_registry: Arc<OAuthProviderRegistry>,
    pub email_sender: Arc<dyn EmailSender>,
    pub sms_sender: Arc<dyn SmsSender>,
    pub route_registry: Arc<Vec<crate::server::RouteInfo>>,
    pub services: ServiceRegistry,
}

/// 构建 AppState（HTTP 服务器和 Tauri 共享）
pub async fn build_app_state(
    config: &AppConfig,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<AppState> {
    let pool = crate::db::connection::init_pool(&config.database_url, config.db_pool_size).await?;
    crate::db::connection::ensure_schema(&pool).await?;
    let eventbus = EventBus::new(256);

    let sqlx_repo = crate::repositories::SqlxPostRepository::new(pool.clone());
    let cache: Arc<dyn crate::cache::CacheStore> = Arc::new(crate::cache::MemoryCache::new());
    let post_repo: Arc<dyn PostRepository> = Arc::new(
        crate::repositories::CachedPostRepository::new(sqlx_repo, cache.clone(), None),
    );

    let user_repo: Arc<dyn UserRepository> =
        Arc::new(crate::repositories::SqlxUserRepository::new(pool.clone()));
    let category_repo: Arc<dyn crate::repositories::CategoryRepository> = Arc::new(
        crate::repositories::SqlxCategoryRepository::new(pool.clone()),
    );
    let tag_repo: Arc<dyn crate::repositories::TagRepository> =
        Arc::new(crate::repositories::SqlxTagRepository::new(pool.clone()));
    let comment_repo: Arc<dyn crate::repositories::CommentRepository> = Arc::new(
        crate::repositories::SqlxCommentRepository::new(pool.clone()),
    );
    let media_repo: Arc<dyn crate::repositories::MediaRepository> =
        Arc::new(crate::repositories::SqlxMediaRepository::new(pool.clone()));
    let refresh_token_repo: Arc<dyn crate::repositories::RefreshTokenRepository> = Arc::new(
        crate::repositories::SqlxRefreshTokenRepository::new(pool.clone()),
    );

    let wallet_repo: Arc<dyn crate::repositories::WalletRepository> =
        Arc::new(crate::repositories::SqlxWalletRepository::new(pool.clone()));

    let search: Arc<dyn SearchEngine> = build_search_engine(config);

    let mut protocol_registry = crate::protocols::ProtocolRegistry::new();
    protocol_registry.register_from_inventory();
    let protocol_registry = Arc::new(protocol_registry);

    let aspect_engine = Arc::new(crate::aspects::engine::AspectEngine::new());
    protocol_registry.register_aspects_into(&aspect_engine);
    tracing::info!(
        "aspect engine initialized with {} aspect(s), {} protocol(s)",
        aspect_engine.aspects().len(),
        protocol_registry.names().len()
    );

    let reserved = config.builtins.reserved_route_segments();
    let protocol_names: Vec<&str> = protocol_registry.names();
    let ct_registry = Arc::new(ContentTypeRegistry::load_from_dir(
        std::path::Path::new(&config.content_type_dir),
        &config.rule_engine,
        &reserved,
        &protocol_names,
        &protocol_registry,
    )?);
    ct_registry.set_protected_tables(config.builtins.protected_tables());

    {
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        for schema in ct_registry.all() {
            repo.migrate(&schema, &protocol_registry).await?;
        }
    }

    let plugin_manager = PluginManager::new_with_options(
        Arc::new(config.clone()),
        crate::plugins::PluginManagerOptions {
            pool: Some(pool.clone()),
            event_bus: Some(eventbus.clone()),
        },
    )
    .await;

    let options_repo: Arc<dyn crate::repositories::OptionsRepository> = Arc::new(
        crate::repositories::SqlxOptionsRepository::new(pool.clone()),
    );
    let options_service =
        Arc::new(OptionsService::new(options_repo, config.builtin_tenantable).await);

    let rbac_repo: Arc<dyn crate::repositories::RbacRepository> =
        Arc::new(crate::repositories::SqlxRbacRepository::new(pool.clone()));
    let rbac_service = Arc::new(RbacService::new(rbac_repo));

    let tenant_repo: Arc<dyn crate::repositories::TenantRepository> =
        Arc::new(crate::repositories::SqlxTenantRepository::new(pool.clone()));
    let tenant_service = Arc::new(TenantService::new(tenant_repo));
    let audit_service = Arc::new(crate::audit::AuditService::new(pool.clone()));
    let webhook_service = Arc::new(crate::webhook::WebhookService::new(pool.clone()));

    let storage = crate::storage::create_storage(config)?;

    let services = ServiceRegistry::new();
    services.insert(post_repo.clone());
    services.insert(user_repo.clone());
    services.insert(category_repo.clone());
    services.insert(tag_repo.clone());
    services.insert(comment_repo.clone());
    services.insert(media_repo.clone());
    services.insert(refresh_token_repo.clone());
    services.insert(wallet_repo.clone());
    services.insert(search.clone());
    services.insert(aspect_engine.clone());
    services.insert(protocol_registry.clone());
    services.insert(ct_registry.clone());
    services.insert(options_service.clone());
    services.insert(rbac_service.clone());
    services.insert(tenant_service.clone());
    services.insert(audit_service.clone());
    services.insert(webhook_service.clone());
    services.insert(cache.clone());
    services.insert(storage.clone());

    let state = AppState {
        pool: pool.clone(),
        config: Arc::new(config.clone()),
        jwt_decoding_key: jsonwebtoken::DecodingKey::from_secret(config.jwt_secret.as_bytes()),
        plugins: plugin_manager,
        eventbus: eventbus.clone(),
        post_repo,
        user_repo,
        category_repo,
        tag_repo,
        comment_repo,
        media_repo,
        refresh_token_repo,
        wallet_repo,
        search,
        content_type_registry: ct_registry,
        aspect_engine,
        protocol_registry,
        options: options_service,
        rbac: rbac_service,
        tenant: tenant_service,
        audit: audit_service,
        webhook: webhook_service.clone(),
        workflow: Arc::new(WorkflowService::new(pool.clone())),
        storage,
        cache: cache.clone(),
        cms_cache: Arc::new(dashmap::DashMap::new()),
        oauth_registry: Arc::new(build_oauth_registry(config)),
        email_sender: crate::notifier::build_email_sender(config),
        sms_sender: crate::notifier::build_sms_sender(config),
        route_registry: Arc::new(Vec::new()),
        services,
    };

    crate::server::spawn_event_subscriber(
        eventbus.clone(),
        state.plugins.clone(),
        shutdown_rx.clone(),
    );
    crate::server::spawn_audit_subscriber(
        eventbus.clone(),
        state.audit.clone(),
        state.tenant.clone(),
        shutdown_rx.clone(),
    );
    crate::server::spawn_webhook_subscriber(eventbus.clone(), state.webhook.clone(), shutdown_rx);

    Ok(state)
}

/// 构建搜索引擎实例
pub fn build_search_engine(config: &AppConfig) -> Arc<dyn SearchEngine> {
    match config.search_engine.as_str() {
        #[cfg(feature = "search-tantivy")]
        "tantivy" => match crate::search::TantivyEngine::open(&config.search_index_dir) {
            Ok(engine) => {
                tracing::info!(
                    "search engine: tantivy (index: {})",
                    config.search_index_dir
                );
                Arc::new(engine)
            }
            Err(e) => {
                tracing::error!("failed to open tantivy index: {e}, falling back to noop");
                Arc::new(crate::search::NoopSearchEngine)
            }
        },
        _ => Arc::new(crate::search::NoopSearchEngine),
    }
}

/// 构建 OAuth Provider 注册表
pub fn build_oauth_registry(config: &AppConfig) -> OAuthProviderRegistry {
    let mut registry = OAuthProviderRegistry::new();
    if let Some(gh) = &config.oauth.github {
        registry.register(Box::new(crate::oauth::github::GitHubProvider::new(
            gh.client_id.clone(),
            gh.client_secret.clone(),
        )));
        tracing::info!("OAuth provider registered: github");
    }
    if let Some(google) = &config.oauth.google {
        registry.register(Box::new(crate::oauth::google::GoogleProvider::new(
            google.client_id.clone(),
            google.client_secret.clone(),
        )));
        tracing::info!("OAuth provider registered: google");
    }
    if let Some(wechat) = &config.oauth.wechat {
        registry.register(Box::new(crate::oauth::wechat::WechatProvider::new(
            wechat.app_id.clone(),
            wechat.app_secret.clone(),
            config.base_url.clone(),
        )));
        tracing::info!("OAuth provider registered: wechat");
    }
    registry
}
