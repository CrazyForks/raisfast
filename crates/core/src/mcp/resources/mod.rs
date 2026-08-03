//! MCP Resources — URI-addressable, read-only data that AI clients can browse.
//!
//! Resources are organised by domain into submodules. Each domain implements
//! [`ResourceProvider`] and is registered via [`register_all`].

use super::registry::ResourceRegistry;

pub mod content_types;
pub mod routes;

/// Register all built-in resource providers.
pub fn register_all(reg: &mut ResourceRegistry) {
    reg.register(content_types::ContentTypeProvider);
    reg.register(routes::RouteProvider);
}
