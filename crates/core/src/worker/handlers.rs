//! Concrete job Handler implementations
//!
//! Each Handler encapsulates its required external dependencies (Pool, Config),
//! and is registered with the `JobHandlerRegistry` in `register_all()`.

pub mod cache;
pub mod db_backup;
pub mod email;
pub mod email_verification;
pub mod order_expire;
pub mod payment_expire;
pub mod payment_reconcile;
pub mod payment_retry;
pub mod ping;
pub mod publish;
pub mod script;
pub mod search_index;
pub mod sitemap;
pub mod sms;
pub mod thumbnail;
pub mod wallet_outbox;
pub mod webhook;

use std::sync::Arc;

use crate::cache::CacheStore;
use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::notifier::{EmailSender, SmsSender};
use crate::search::SearchEngine;
use crate::worker::JobHandlerRegistry;
use crate::worker::handler::HandlerMeta;

/// Returns metadata for all cron handlers registered with `register_with_meta`.
///
/// Called by the `GET /admin/cron-handlers` endpoint to populate the admin task menu.
/// **NOTE**: Keep this list in sync with the `register_with_meta` calls in `register_all`.
pub fn cron_handler_metas() -> Vec<&'static HandlerMeta> {
    vec![&ping::META]
}

/// Registers all concrete Handlers with the Registry
pub fn register_all(
    registry: &mut JobHandlerRegistry,
    pool: Pool,
    config: Arc<AppConfig>,
    search: Arc<dyn SearchEngine>,
    cache: Arc<dyn CacheStore>,
    email_sender: Arc<dyn EmailSender>,
    sms_sender: Arc<dyn SmsSender>,
    plugins: Arc<crate::plugins::PluginManager>,
) {
    registry.register(
        "send_welcome_email",
        Box::new(email::SendWelcomeEmailHandler::new(
            config.clone(),
            email_sender.clone(),
        )),
    );
    registry.register(
        "send_password_reset_email",
        Box::new(email::SendPasswordResetEmailHandler::new(
            config.clone(),
            email_sender.clone(),
        )),
    );
    registry.register(
        "send_sms_code",
        Box::new(sms::SendSmsCodeHandler::new(sms_sender)),
    );
    registry.register(
        "send_email_verification",
        Box::new(email_verification::SendEmailVerificationHandler::new(
            config.clone(),
            email_sender,
        )),
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
        Box::new(cache::InvalidateCacheHandler::new(cache)),
    );
    registry.register(
        "generate_sitemap",
        Box::new(sitemap::GenerateSitemapHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "expire_payment_orders",
        Box::new(payment_expire::ExpirePaymentOrdersHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "expire_orders",
        Box::new(order_expire::ExpireOrdersHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "retry_payment_callback",
        Box::new(payment_retry::RetryPaymentCallbackHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "reconcile_payments",
        Box::new(payment_reconcile::ReconcilePaymentsHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "process_wallet_outbox",
        Box::new(wallet_outbox::ProcessWalletOutboxHandler::new(
            pool.clone(),
            config.clone(),
        )),
    );
    registry.register(
        "db_backup",
        Box::new(db_backup::DbBackupHandler::new(config.clone())),
    );

    // ── Cron handlers with admin-visible metadata ───────────────────────────
    registry.register_with_meta(
        "ping",
        Box::new(ping::PingHandler::new(config.clone())),
        &ping::META,
    );

    // ── Cron sandbox script handler (no meta — not a menu item) ────────────
    registry.register(
        "run_script",
        Box::new(script::ScriptJobHandler::new(
            plugins,
            pool.clone(),
            // Cron scripts are admin-created → full permissions.
            crate::plugins::Permissions {
                http: vec!["*".to_string()],
                config: vec!["*".to_string()],
                database: vec!["*".to_string()],
                filesystem: vec!["*".to_string()],
                ..Default::default()
            },
        )),
    );
}
