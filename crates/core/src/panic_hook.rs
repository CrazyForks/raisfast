//! Panic hook: writes panic info to a dedicated log file and emits a
//! `system.panic` event to the global [`EventBus`] for webhook delivery.
//!
//! # Log file
//!
//! Panics are written to `{log_dir}/panic_YYYY-MM-DD.log` in a human-readable
//! format (separate from the normal `raisfast_YYYY-MM-DD.log`).
//!
//! # Webhook
//!
//! If [`set_event_bus`] has been called (during server startup), the panic is
//! also emitted as [`Event::SystemPanic`] on the event bus. The existing webhook
//! subscriber ([`crate::server::spawn_webhook_subscriber`]) will deliver it to
//! any subscription matching `"system.panic"`.
//!
//! # Usage
//!
//! ```ignore
//! // In main(), before anything else:
//! panic_hook::install("storage/logs");
//!
//! // Later, after server startup:
//! panic_hook::set_event_bus(eventbus);
//! ```

use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;

use crate::event::Event;
use crate::event::SystemPanicPayload;
use crate::eventbus::EventBus;

/// Global event bus reference — set during server startup.
static EVENT_BUS: OnceLock<EventBus> = OnceLock::new();

/// Log directory captured at hook installation time.
static LOG_DIR: OnceLock<std::sync::Mutex<String>> = OnceLock::new();

/// Register the global event bus so the panic hook can emit events.
///
/// Called from [`crate::build_app_state`] after the `EventBus` is created.
pub fn set_event_bus(bus: EventBus) {
    let _ = EVENT_BUS.set(bus);
}

/// Install the custom panic hook.
///
/// Must be called early in `main()`, before any async runtime work.
/// The `log_dir` is used for `panic_YYYY-MM-DD.log`.
pub fn install(log_dir: &str) {
    let _ = LOG_DIR.set(std::sync::Mutex::new(log_dir.to_string()));

    let default_hook = std::panic::take_hook();

    std::panic::set_hook(Box::new(move |info| {
        // Extract panic details
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_string());

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());

        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let timestamp = crate::utils::tz::now_utc().to_rfc3339();

        // 1. Write to panic log file
        write_panic_log(&message, &location, &backtrace, &timestamp);

        // 2. Emit system.panic event for webhook delivery
        if let Some(bus) = EVENT_BUS.get() {
            bus.emit(Event::SystemPanic(SystemPanicPayload {
                message,
                location,
                backtrace,
                timestamp,
            }));
        }

        // 3. Chain to the default hook (prints to stderr, triggers abort on
        //    double-panic, etc.)
        default_hook(info);
    }));
}

/// Write a panic entry to `panic_YYYY-MM-DD.log` in the configured log dir.
fn write_panic_log(message: &str, location: &str, backtrace: &str, timestamp: &str) {
    let Some(dir_lock) = LOG_DIR.get() else {
        return;
    };
    let dir = dir_lock.lock().unwrap_or_else(|e| e.into_inner()).clone();

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let filename = format!("panic_{date}.log");
    let path = std::path::Path::new(&dir).join(&filename);

    let entry = format!(
        "[{timestamp}] PANIC at {location}\n  message:  {message}\n  backtrace:\n{backtrace}\n{}\n",
        "─".repeat(80)
    );

    if let Err(e) = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .and_then(|mut f| f.write_all(entry.as_bytes()))
    {
        eprintln!("WARN: failed to write panic log to {}: {e}", path.display());
    }
}
