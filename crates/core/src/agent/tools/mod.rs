//! Domain tool adapters: wrap `crates/core` services as `raisfast-agent` tools.
//!
//! One file per business domain (`posts`, later `ecommerce`, `content_type`,
//! `wallet`, …), each exposing a `register(&mut ToolRegistry, &AppState,
//! &AuthUser)`. The registry is built per turn with the session's actor
//! snapshot (AuthUser) so tools hit the service layer with correct ownership.
//! Tools are thin shells — no business logic, no auth checks (see
//! `architecture.md §3`, `prompt-engineering.md §5`).

pub mod files;
pub mod kb;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod posts;
pub mod script;
pub mod shell;
pub mod skills;
pub mod system;

use raisfast_agent::ToolRegistry;

use crate::AppState;
use crate::middleware::auth::AuthUser;

use super::models::ai_agent::AiAgent;

/// Build the domain tool registry for one turn from the agent's actor.
/// Every available domain tool is registered here; the per-agent allowlist
/// (`ai_agents.tools`) is applied later by `AgentService`. `agent` carries
/// the turn's agent row (KB binding etc.); `None` on paths without one.
/// `session_id` is the turn's conversation (KB run attribution,
/// kb-observability-design §4.2); `None` outside a session turn.
pub async fn build_domain_tools(
    state: &AppState,
    auth: &AuthUser,
    agent: Option<&AiAgent>,
    session_id: Option<crate::types::snowflake_id::SnowflakeId>,
) -> ToolRegistry {
    let mut registry = build_static_tools(state, auth, agent, session_id).await;
    #[cfg(feature = "mcp")]
    {
        mcp::register_mcp_tools(&mut registry, &state.config.ai.mcp_servers).await;
    }
    registry
}

/// The non-MCP part of the domain registry: pure construction (Arc clones,
/// path joins — zero IO), so the admin tool catalog rebuilds it fresh on
/// every request and code/env-level tool changes surface after the
/// mandatory compile/restart without any cache to invalidate. MCP tools
/// (live connections) are added by [`build_domain_tools`] for the turn
/// path, and via [`mcp::cached_catalog_specs`] for the catalog path.
pub async fn build_static_tools(
    state: &AppState,
    auth: &AuthUser,
    agent: Option<&AiAgent>,
    session_id: Option<crate::types::snowflake_id::SnowflakeId>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    posts::register(&mut registry, state, auth);
    system::register(&mut registry, state, auth);
    script::register(&mut registry, &state.plugins);
    files::register(&mut registry, state, auth);
    if let Some(agent) = agent {
        kb::register(&mut registry, state, auth, agent, session_id).await;
    }
    // `run_shell` is default closed: only registered when an operator enabled
    // `[ai].allow_shell` (RAISFAST_AI_ALLOW_SHELL=true), then gated per agent
    // by the `tools` allowlist like every other domain tool.
    if state.config.ai.allow_shell {
        shell::register(&mut registry, auth);
    }
    registry
}
