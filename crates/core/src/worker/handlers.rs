//! Concrete job Handler implementations
//!
//! Each Handler encapsulates its required external dependencies (Pool, Config),
//! and is registered with the `JobHandlerRegistry` in `register_all()`.
//!
//! **Builtin cron handlers** self-register via the `register_cron_handler!` macro
//! (inventory-based, same pattern as `register_protocol!`).
//!
//! Adding a new builtin cron handler means creating `worker/handlers/<name>.rs`
//! with a `pub const META`, the handler impl, and a `register_cron_handler!(...)`
//! line at the bottom, then adding one `pub mod <name>;` below. No changes to
//! `register_all()` are needed.

use std::sync::Arc;

use crate::cache::CacheStore;
use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::notifier::{EmailSender, SmsSender};
use crate::search::SearchEngine;
use crate::worker::JobHandlerRegistry;
use crate::worker::handler::HandlerMeta;

/// Aggregated dependencies passed to every builtin cron handler factory.
pub struct HandlerDeps {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub search: Arc<dyn SearchEngine>,
    pub cache: Arc<dyn CacheStore>,
    pub email_sender: Arc<dyn EmailSender>,
    pub sms_sender: Arc<dyn SmsSender>,
    pub plugins: Arc<crate::plugins::PluginManager>,
    pub emitter: crate::event::EventEmitter,
}

/// Inventory entry for a builtin cron handler. Handlers self-register via
/// [`register_cron_handler!`].
pub struct CronHandlerEntry {
    pub meta: &'static HandlerMeta,
    pub factory: fn(&HandlerDeps) -> Box<dyn crate::worker::JobHandler>,
}

inventory::collect!(CronHandlerEntry);

/// Self-registration macro for builtin cron handlers.
///
/// Usage at the bottom of a handler file:
/// ```ignore
/// register_cron_handler!(META, |deps| Box::new(MyHandler::new(deps.pool.clone())));
/// ```
#[macro_export]
macro_rules! register_cron_handler {
    ($meta:expr, $factory:expr) => {
        ::inventory::submit! {
            $crate::worker::handlers::CronHandlerEntry {
                meta: $meta,
                factory: $factory,
            }
        }
    };
}

pub mod agent_run;
pub mod cache;
pub mod cron_ping;
pub mod db_backup;
pub mod email;
pub mod email_verification;
pub mod flow_run;
pub mod ingress_orphan_scan;
pub mod ingress_pull;
pub mod ingress_retry;
pub mod itg_egress_cleanup;
pub mod order_expire;
pub mod payment_expire;
pub mod payment_reconcile;
pub mod payment_retry;
pub mod publish;
pub mod script;
pub mod search_index;
pub mod sitemap;
pub mod sms;
#[cfg(feature = "cron-system")]
pub mod system;
pub mod thumbnail;
pub mod wallet_outbox;
pub mod webhook;

/// Registers all concrete Handlers with the Registry
pub fn register_all(deps: HandlerDeps) -> JobHandlerRegistry {
    let mut registry = JobHandlerRegistry::new();
    let HandlerDeps {
        pool,
        config,
        search,
        cache,
        email_sender,
        sms_sender,
        plugins,
        emitter,
    } = deps;

    // Clones for the inventory loop (single-use values moved into handlers above).
    let loop_search = search.clone();
    let loop_cache = cache.clone();
    let loop_email = email_sender.clone();
    let loop_sms = sms_sender.clone();
    let loop_emitter = emitter.clone();
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

    registry.register_with_meta(
        flow_run::META.id,
        Box::new(flow_run::FlowRunHandler::new(pool.clone(), plugins.clone())),
        &flow_run::META,
    );

    registry.register_with_meta(
        agent_run::META.id,
        Box::new(agent_run::AgentRunHandler::new(
            pool.clone(),
            config.clone(),
            emitter.clone(),
        )),
        &agent_run::META,
    );

    // ── Cron handlers: collected from inventory self-registration ───────────
    // Every `register_cron_handler!(...)` call in a handler file is collected here.
    for entry in inventory::iter::<CronHandlerEntry> {
        let deps = HandlerDeps {
            pool: pool.clone(),
            config: config.clone(),
            search: loop_search.clone(),
            cache: loop_cache.clone(),
            email_sender: loop_email.clone(),
            sms_sender: loop_sms.clone(),
            plugins: plugins.clone(),
            emitter: loop_emitter.clone(),
        };
        let handler = (entry.factory)(&deps);
        registry.register_with_meta(entry.meta.id, handler, entry.meta);
    }

    // ── Cron sandbox script handler (no meta — not a menu item) ────────────
    registry.register(
        "run_script",
        Box::new(script::ScriptJobHandler::new(
            plugins.clone(),
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

    // ── Cron system script handler (feature-gated) ─────────────────────
    #[cfg(feature = "cron-system")]
    registry.register(
        "run_system",
        Box::new(system::SystemJobHandler::new(pool.clone(), config.clone())),
    );

    registry
}
