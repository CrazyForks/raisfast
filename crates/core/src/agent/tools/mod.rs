//! Domain tool adapters: wrap `crates/core` services as `raisfast-agent` tools.
//!
//! One file per business domain (`posts`, later `ecommerce`, `content_type`,
//! `wallet`, …), each exposing a `register(&mut ToolRegistry, &AppState,
//! &AuthUser)`. The registry is built per turn with the session's actor
//! snapshot (AuthUser) so tools hit the service layer with correct ownership.
//! Tools are thin shells — no business logic, no auth checks (see
//! `architecture.md §3`, `prompt-engineering.md §5`).

pub mod posts;
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
    registry
}
