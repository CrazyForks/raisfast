//! Await background infra (await-node.md §5): the timeout sweeper.
//!
//! Event wake was REMOVED in v4 (await-node.md §12.3): the await node is
//! trigger-agnostic — whoever cares (admin UI, an external system holding the
//! public resume URL, a flow_trigger listener, a plugin) calls the resume
//! endpoint themselves. No bus scanning, no correlation matching.

use std::sync::Arc;
use std::time::Duration;

use crate::integration::IntegrationPlane;
use crate::plugins::PluginManager;

use super::run;

const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Spawn the await timeout sweeper (cheap when nothing is parked — one
/// indexed scan per minute).
pub fn spawn(
    pool: crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SWEEP_INTERVAL);
        interval.tick().await; // skip the immediate first tick
        loop {
            interval.tick().await;
            match run::sweep_expired_awaits(&pool, plane.clone(), plugins.clone()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!("await timeout sweep acted on {n} claim(s)"),
                Err(e) => tracing::error!("await timeout sweep error: {e}"),
            }
        }
    });
}
