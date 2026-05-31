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
pub mod publish;
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

/// Registers all concrete Handlers with the Registry
pub fn register_all(
    registry: &mut JobHandlerRegistry,
    pool: Pool,
    config: Arc<AppConfig>,
    search: Arc<dyn SearchEngine>,
    cache: Arc<dyn CacheStore>,
    email_sender: Arc<dyn EmailSender>,
    sms_sender: Arc<dyn SmsSender>,
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
            pool,
            config.clone(),
        )),
    );
    registry.register(
        "db_backup",
        Box::new(db_backup::DbBackupHandler::new(config)),
    );
}
