//! KB models — one file per table, mirroring `crate::models` layout
//! (kb-technical-design §2).
//!
//! Static tables (decision D1), CRUD via the workspace macros, batch
//! precedent: the `ai_*` table family. SQLite/PG/MySQL portability per
//! AGENTS.md (timestamps bound as `DateTime<Utc>`, JSON as `Value`,
//! booleans as `bool`, ids as snowflakes).

pub mod chunk;
pub mod document;
pub mod faq;
pub mod kb_run;
pub mod knowledge_base;
pub mod query_log;
pub mod wiki_page;
pub mod wiki_source;
