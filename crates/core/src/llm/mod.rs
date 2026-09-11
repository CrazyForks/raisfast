//! LLM foundation: upstream channel/key-pool routing core, model directory and
//! admin API. Design doc: `dev-docs/llm/design.md`; flow diagrams:
//! `dev-docs/llm/flow.md`.
//!
//! Layout follows the vertical-module convention of `agent/` and `kb/`
//! (design §13): this directory owns its handler/service/models, exposing only
//! three contact points to the rest of the crate — `lib.rs` (`pub mod` +
//! `AppState::llm_router`), `server.rs` (route merge behind
//! `builtins.llm_gateway`) and `migrations/` (four `llm_*` tables).

pub mod cache;
pub mod crypto;
pub mod execute;
pub mod handler;
pub mod models;
pub mod queue;
pub mod registry;
pub mod relay;
pub mod service;

#[cfg(test)]
mod tests_db;
