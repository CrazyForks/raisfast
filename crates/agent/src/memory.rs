//! Long-term memory contract (engine-host agnostic).
//!
//! The engine only knows `store/recall/forget`. Scoping (tenant/agent/session)
//! is applied by the wrapping layer (production = core's `ScopedMemory` over
//! `ai_memories`; this module ships an in-memory impl for the MVP/demo/tests).
//! Full design: `dev-docs/agent/contracts.md §4`.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub key: String,
    pub content: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("store: {0}")]
    Store(String),
    #[error("recall: {0}")]
    Recall(String),
    #[error("forget: {0}")]
    Forget(String),
}

#[async_trait]
pub trait Memory: Send + Sync {
    fn name(&self) -> &str;

    async fn store(&self, key: &str, content: &str) -> Result<(), MemoryError>;

    /// `query = None` returns most-recent entries; `Some(q)` does keyword recall.
    async fn recall(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;

    async fn forget(&self, key: &str) -> Result<bool, MemoryError>;
}

/// Simple keyword in-memory store (single scope). Good enough to prove the
/// agent-driven memory loop before the sqlx-backed store lands.
#[derive(Default)]
pub struct InMemoryMemory {
    entries: Mutex<Vec<MemoryEntry>>,
}

impl InMemoryMemory {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Memory for InMemoryMemory {
    fn name(&self) -> &str {
        "in_memory"
    }

    async fn store(&self, key: &str, content: &str) -> Result<(), MemoryError> {
        let mut entries = self.entries.lock().unwrap();
        if let Some(entry) = entries.iter_mut().find(|e| e.key == key) {
            entry.content = content.to_string();
        } else {
            entries.push(MemoryEntry {
                key: key.to_string(),
                content: content.to_string(),
            });
        }
        Ok(())
    }

    async fn recall(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<MemoryEntry> = match query {
            None => entries.iter().rev().take(limit).cloned().collect(),
            Some(q) if q.trim().is_empty() => entries.iter().rev().take(limit).cloned().collect(),
            Some(q) => {
                let q = q.to_lowercase();
                entries
                    .iter()
                    .filter(|e| {
                        let key = e.key.to_lowercase();
                        let content = e.content.to_lowercase();
                        ngram_hits(&q, &key) || ngram_hits(&q, &content)
                    })
                    .rev()
                    .take(limit)
                    .cloned()
                    .collect()
            }
        };
        out.reverse();
        Ok(out)
    }

    async fn forget(&self, key: &str) -> Result<bool, MemoryError> {
        let mut entries = self.entries.lock().unwrap();
        let before = entries.len();
        entries.retain(|e| e.key != key);
        Ok(entries.len() != before)
    }
}

/// Rough keyword match that works for CJK text without spaces: true when any
/// 2..=16 char contiguous window of the query appears in `hay`. For pure-ASCII
/// queries this degrades gracefully to near-substring matching.
fn ngram_hits(query: &str, hay: &str) -> bool {
    let chars: Vec<char> = query.chars().collect();
    let n = chars.len();
    if n == 0 {
        return false;
    }
    let max_win = n.min(16);
    for win in 2..=max_win {
        for start in 0..=(n - win) {
            let piece: String = chars[start..start + win].iter().collect();
            if hay.contains(&piece) {
                return true;
            }
        }
    }
    // single-token query (e.g. a pure ASCII word) falls back to whole query.
    if n == 1 { hay.contains(query) } else { false }
}

/// Render recalled entries into the `[Memory context]` block that is prepended
/// to the latest user message (loop-engine §9, prompt-engineering §4.1).
pub fn render_memory_context(entries: &[MemoryEntry]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let mut lines = vec!["[Memory context]".to_string()];
    for e in entries {
        lines.push(format!("- {}: {}", e.key, e.content));
    }
    lines.push("[/Memory context]".to_string());
    Some(lines.join("\n"))
}

/// Build the memory tools bound to a shared [`Memory`] handle and register
/// them into the registry (core will swap in its scoped sqlx-backed handle).
pub fn register_memory_tools(registry: &mut crate::tool::ToolRegistry, memory: Arc<dyn Memory>) {
    registry.register(crate::memory::tools::MemoryStoreTool::new(memory.clone()));
    registry.register(crate::memory::tools::MemoryRecallTool::new(memory.clone()));
    registry.register(crate::memory::tools::MemoryForgetTool::new(memory));
}

pub mod tools {
    //! Agent-facing memory tools: the model decides when to persist facts.
    use super::{Memory, MemoryError};
    use crate::tool::{Tool, ToolExecution};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Arc;

    pub struct MemoryStoreTool {
        memory: Arc<dyn Memory>,
    }

    impl MemoryStoreTool {
        pub fn new(memory: Arc<dyn Memory>) -> Self {
            Self { memory }
        }
    }

    #[async_trait]
    impl Tool for MemoryStoreTool {
        fn name(&self) -> &str {
            "memory_store"
        }
        fn description(&self) -> &str {
            "Store a durable fact about the user or task (key-value) so it is available in \
             future turns, e.g. the user's preferred language or an agreed policy."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "short snake_case key" },
                    "content": { "type": "string", "description": "the fact to remember" }
                },
                "required": ["key", "content"]
            })
        }
        async fn execute(&self, args: Value) -> ToolExecution {
            let key = args
                .get("key")
                .and_then(Value::as_str)
                .ok_or("key required")?;
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or("content required")?;
            self.memory.store(key, content).await.map_err(mem_err)?;
            Ok(format!("已记住 {key}"))
        }
    }

    pub struct MemoryRecallTool {
        memory: Arc<dyn Memory>,
    }

    impl MemoryRecallTool {
        pub fn new(memory: Arc<dyn Memory>) -> Self {
            Self { memory }
        }
    }

    #[async_trait]
    impl Tool for MemoryRecallTool {
        fn name(&self) -> &str {
            "memory_recall"
        }
        fn description(&self) -> &str {
            "Recall stored facts relevant to a query. Facts are also injected automatically \
             before each turn; use this for explicit retrieval."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "optional keyword" },
                    "limit": { "type": "integer", "description": "max results (default 5)" }
                }
            })
        }
        async fn execute(&self, args: Value) -> ToolExecution {
            let query = args.get("query").and_then(Value::as_str);
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
            let entries = self.memory.recall(query, limit).await.map_err(mem_err)?;
            if entries.is_empty() {
                return Ok("(无相关记忆)".to_string());
            }
            let mut out = String::new();
            for e in &entries {
                out.push_str(&format!("- {}: {}\n", e.key, e.content));
            }
            Ok(out.trim_end().to_string())
        }
    }

    pub struct MemoryForgetTool {
        memory: Arc<dyn Memory>,
    }

    impl MemoryForgetTool {
        pub fn new(memory: Arc<dyn Memory>) -> Self {
            Self { memory }
        }
    }

    #[async_trait]
    impl Tool for MemoryForgetTool {
        fn name(&self) -> &str {
            "memory_forget"
        }
        fn description(&self) -> &str {
            "Forget a stored fact by key."
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            })
        }
        async fn execute(&self, args: Value) -> ToolExecution {
            let key = args
                .get("key")
                .and_then(Value::as_str)
                .ok_or("key required")?;
            let removed = self.memory.forget(key).await.map_err(mem_err)?;
            Ok(if removed {
                format!("已忘记 {key}")
            } else {
                format!("没有找到 {key}")
            })
        }
    }

    fn mem_err(e: MemoryError) -> String {
        e.to_string()
    }
}
