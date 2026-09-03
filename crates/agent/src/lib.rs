#![forbid(unsafe_code)]
//! RaisFast agent core (MVP).
//!
//! Minimal but real: a native function-calling turn loop plus an
//! OpenAI-compatible provider and a small tool registry. No DB, memory,
//! streaming, or session persistence yet — see `dev-docs/agent/` for the
//! full design and milestone plan.
//!
//! Adapted from references in `third/` (see `dev-docs/agent/reference-analysis.md`):
//! wire-shape conventions borrowed from claw-code `api/src/providers/openai_compat.rs` (MIT).

pub mod loop_;
pub mod memory;
pub mod messages;
pub mod provider;
pub mod skill_doc;
pub mod tool;

pub use loop_::{TurnConfig, TurnEngine, TurnError, TurnEvent, TurnOutcome};
pub use memory::{
    InMemoryMemory, Memory, MemoryEntry, MemoryError, register_memory_tools, render_memory_context,
};
pub use messages::{ChatMessage, ChatRole, TokenUsage, ToolCall};
pub use provider::{ChatRequest, ChatResponse, ModelProvider, ProviderError, StreamEvent, openai};
pub use skill_doc::{SkillDocError, SkillDocument, SkillFrontmatter};
pub use tokio_util::sync::CancellationToken;
pub use tool::{Tool, ToolRegistry, ToolSpec};
