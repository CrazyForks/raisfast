//! App Bundle — RaisFast's application primitive: packaging, install,
//! upgrade, uninstall, dependencies, distribution and trust
//! (dev-docs/integration/app-bundle.md; MVP scope per
//! dev-docs/integration/plans/mvp-plan.md MVP-M2).
//!
//! Composition:
//! - [`manifest`] — `app.toml` parsing + semantic validation
//! - [`model`] — `apps` / `app_ct_refs` rows, CAS status machine
//! - [`package`] — `.rafapp` unpack (zip-slip guard) + hash manifest
//! - [`precheck`] — full conflict report + keep-data re-attach
//! - [`installer`] — 8-step materialization + compensation log
//! - [`seeder`] — deterministic `seed_key` upserts
//! - [`uninstaller`] — shared undo executor + drain orchestration
//! - [`registry`] — `AppRegistry` state-machine holder / orchestration
//! - [`admin`] / [`routes`] — admin API
//! - [`pack`] — `raisfast app pack` CLI implementation

pub mod admin;
pub mod installer;
pub mod manifest;
pub mod model;
pub mod pack;
pub mod package;
pub mod precheck;
pub mod registry;
pub mod routes;
pub mod seeder;
pub mod uninstaller;

pub use manifest::AppBundleManifest;
pub use package::AppPackage;
pub use registry::{AppRegistry, InstallOptions};

use std::sync::Arc;

/// Process-wide shared app registry — set once at startup (`build_app_state`),
/// read by handlers and CLI paths. `None` before init.
static SHARED: std::sync::OnceLock<Arc<AppRegistry>> = std::sync::OnceLock::new();

/// Install the shared registry handle (startup only).
pub fn set_shared(registry: Arc<AppRegistry>) {
    let _ = SHARED.set(registry);
}

/// The shared registry handle, if initialized.
#[must_use]
pub fn shared() -> Option<Arc<AppRegistry>> {
    SHARED.get().cloned()
}
