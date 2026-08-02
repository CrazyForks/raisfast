//! `mcp serve` subcommand — runs the MCP server over stdio.
//!
//! Builds a full [`raisfast::AppState`] (so all services are available) and
//! hands control to [`raisfast::mcp::handler::serve_stdio`]. Intended for local
//! AI clients (Claude Desktop, Cursor) that spawn raisfast as a child process.

use raisfast::config::app::AppConfig;

/// `raisfast mcp serve` — run the MCP stdio server.
pub async fn serve(config: &AppConfig) -> anyhow::Result<()> {
    if !config.builtins.mcp {
        anyhow::bail!("MCP is disabled (BUILTIN_MCP=false). Enable it to use `mcp serve`.");
    }

    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let state = raisfast::build_app_state(config, shutdown_rx).await?;
    raisfast::mcp::handler::serve_stdio(state).await
}
