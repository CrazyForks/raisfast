//! Domain tool adapters: wrap `crates/core` services as `raisfast-agent` tools.
//!
//! One file per business domain (`posts`, later `ecommerce`, `content_type`,
//! `wallet`, …), each exposing a `register(&mut ToolRegistry, &AppState,
//! &AuthUser)`. The registry is built per turn with the session's actor
//! snapshot (AuthUser) so tools hit the service layer with correct ownership.
//! Tools are thin shells — no business logic, no auth checks (see
//! `architecture.md §3`, `prompt-engineering.md §5`).

pub mod files;
pub mod posts;
pub mod script;
pub mod shell;
pub mod skills;
pub mod system;

use raisfast_agent::ToolRegistry;

use crate::AppState;
use crate::middleware::auth::AuthUser;

/// Build the domain tool registry for one turn from the agent's actor.
/// Every available domain tool is registered here; the per-agent allowlist
/// (`ai_agents.tools`) is applied later by `AgentService`.
pub fn build_domain_tools(state: &AppState, auth: &AuthUser) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    posts::register(&mut registry, state, auth);
    system::register(&mut registry, state, auth);
    script::register(&mut registry, &state.plugins);
    files::register(&mut registry, state, auth);
    // `run_shell` is default closed: only registered when an operator enabled
    // `[ai].allow_shell` (RAISFAST_AI_ALLOW_SHELL=true), then gated per agent
    // by the `tools` allowlist like every other domain tool.
    if state.config.ai.allow_shell {
        shell::register(&mut registry, auth);
    }
    registry
}
