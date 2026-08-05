//! Event emitter — fan-out events to EventBus + plugin action dispatch.
//!
//! This is the successor to `AspectEngine::emit()`, separated from the aspect
//! system so that services can broadcast events without depending on AspectEngine.
//!
//! `emit(event)` does two things:
//! 1. Broadcasts to `EventBus` (sync — audit log & webhook subscribers receive it)
//! 2. Spawns a fire-and-forget plugin `dispatch_action` (async — JS/Rhai/Lua/WASM)

use std::sync::{Arc, Weak};

use crate::event::Event;
use crate::eventbus::EventBus;
use crate::plugins::PluginManager;

/// Event fan-out helper — the single entry point for services to broadcast events.
///
/// Holds a `Weak<PluginManager>` to avoid a reference cycle (PluginManager →
/// services → EventEmitter → PluginManager). If the plugin manager is being torn
/// down, the action dispatch is silently skipped.
#[derive(Clone)]
pub struct EventEmitter {
    eventbus: EventBus,
    plugins: Weak<PluginManager>,
}

impl EventEmitter {
    /// Creates a new emitter from the shared event bus and plugin manager.
    #[must_use]
    pub fn new(eventbus: EventBus, plugins: &Arc<PluginManager>) -> Self {
        Self {
            eventbus,
            plugins: Arc::downgrade(plugins),
        }
    }

    /// Creates an emitter with no plugin dispatch (eventbus-only).
    /// Useful for tests that don't need plugin hooks.
    #[must_use]
    pub fn eventbus_only(eventbus: EventBus) -> Self {
        Self {
            eventbus,
            plugins: Weak::default(),
        }
    }

    /// Broadcast an event to all subscribers (EventBus + plugin actions).
    ///
    /// - EventBus broadcast is synchronous (audit/webhook subscribers).
    /// - Plugin action dispatch is fire-and-forget (spawned async task).
    pub fn emit(&self, event: Event) {
        // 1. EventBus — audit log, webhook, SSE subscribers
        self.eventbus.emit(event.clone());

        // 2. Plugin actions — fire-and-forget
        if let Some(plugins) = self.plugins.upgrade() {
            let hook_name = event.name();
            let json = serde_json::to_value(&event).unwrap_or_default();
            tokio::spawn(async move {
                plugins.dispatch_action(&hook_name, &json).await;
            });
        }
    }

    /// Returns the underlying event bus (for SSE/streaming subscribers).
    pub fn eventbus(&self) -> &EventBus {
        &self.eventbus
    }
}
