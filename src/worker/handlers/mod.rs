//! 真实任务 Handler 实现
//!
//! 每个 Handler 封装所需的外部依赖（Pool、Config），
//! 在 `register_all()` 统一注册到 `JobHandlerRegistry`。

pub mod cache;
pub mod email;
pub mod publish;
pub mod search_index;
pub mod sitemap;
pub mod thumbnail;
pub mod webhook;

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::search::SearchEngine;
use crate::worker::JobHandlerRegistry;

/// 将所有真实 Handler 注册到 Registry
///
/// 在 `server/mod.rs` 的 `spawn_workers()` 中调用，替换 `LogJobHandler` 占位符。
pub fn register_all(
    registry: &mut JobHandlerRegistry,
    pool: Pool,
    config: Arc<AppConfig>,
    search: Arc<dyn SearchEngine>,
) {
    registry.register(
        "send_welcome_email",
        Box::new(email::SendWelcomeEmailHandler::new(config.clone())),
    );
    registry.register(
        "generate_thumbnail",
        Box::new(thumbnail::GenerateThumbnailHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "scheduled_publish",
        Box::new(publish::ScheduledPublishHandler::new(pool.clone())),
    );
    registry.register(
        "webhook_notify",
        Box::new(webhook::WebhookNotifyHandler::new()),
    );
    registry.register(
        "rebuild_search_index",
        Box::new(search_index::RebuildSearchIndexHandler::new(
            pool.clone(),
            search,
        )),
    );
    registry.register(
        "invalidate_cache",
        Box::new(cache::InvalidateCacheHandler::new()),
    );
    registry.register(
        "generate_sitemap",
        Box::new(sitemap::GenerateSitemapHandler::new(pool, config)),
    );
}
