//! MCP Prompts — user-triggered slash commands that assemble pre-built
//! conversation messages with server-side context.
//!
//! Prompts are **user-controlled** (the user picks them from a menu), unlike
//! tools (AI-controlled) and resources (application-controlled). Each prompt
//! returns one or more [`PromptMessage`]s that the client injects into the
//! conversation.
//!
//! # Adding a new prompt domain
//!
//! 1. Create `src/mcp/prompts/{domain}.rs`
//! 2. Define a struct that `impl PromptProvider`
//! 3. Add a `register(...)` function
//! 4. Call it from [`register_all`]

use super::registry::PromptRegistry;

pub mod blog;
pub mod cms;

/// Register all built-in prompt providers.
pub fn register_all(reg: &mut PromptRegistry) {
    reg.register(blog::BlogPrompts);
    reg.register(cms::CmsPrompts);
}
