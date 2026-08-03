//! MCP Tools — discrete operations exposed to AI clients.
//!
//! Tools are organised by domain into submodules. Each tool is a struct
//! implementing [`Tool`]. All tools are registered via [`register_all`].
//!
//! # Adding a new domain
//!
//! 1. Create `src/mcp/tools/{domain}.rs`
//! 2. Define tool structs that `impl Tool for ...`
//! 3. Add a `register(...)` function in that module
//! 4. Call it from [`register_all`] below

use super::registry::ToolRegistry;

pub mod content_types;
pub mod entries;
pub mod posts;

/// Register all built-in tools into the registry.
///
/// To add a new tool domain, add one line here.
pub fn register_all(reg: &mut ToolRegistry) {
    content_types::register(reg);
    entries::register(reg);
    posts::register(reg);
}
