//! Agent-core table models (`ai_agents`, `ai_sessions`, `ai_messages`,
//! `ai_memories`), kept inside the `agent` module for cohesion.
//!
//! One file per table, mirroring the top-level `models/` convention. Semantics:
//! - `ai_messages` is the append-only session log; `role` includes `meta` rows
//!   (`turn:meta`, `context:summary`, `context:reset`) skipped on replay.
//! - `usage` rides every assistant row (per LLM call); tool rows carry
//!   `tool_success/error/elapsed_ms/truncated`.
//! - `ai_memories.superseded_by IS NULL` is the live-row predicate.
//!
//! See `dev-docs/agent/db-schema.md`.

pub mod ai_agent;
pub mod ai_memory;
pub mod ai_message;
pub mod ai_session;
