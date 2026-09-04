//! sqlx-backed `raisfast_agent::Memory` implementation, scoped to exactly one
//! (tenant, agent): every read/write is pinned to that scope (decorator pattern
//! from `dev-docs/agent/contracts.md §4` / `architecture.md §3`).
//!
//! The agent's own `memory_store/recall/forget` tools receive an instance of
//! this handle, so the model can never touch another tenant/agent's rows.

use async_trait::async_trait;
use std::sync::Arc;

use raisfast_agent::{Memory, MemoryEntry, MemoryError};

use crate::agent::models::ai_memory;
use crate::types::snowflake_id::SnowflakeId;

/// Memory scoped to (tenant, agent). Clone cheaply for multiple tools.
pub struct ScopedMemory {
    pool: crate::db::Pool,
    tenant: Option<String>,
    agent: SnowflakeId,
    user: Option<SnowflakeId>,
}

impl ScopedMemory {
    pub fn new(
        pool: crate::db::Pool,
        tenant: Option<String>,
        agent: SnowflakeId,
        user: Option<SnowflakeId>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            tenant,
            agent,
            user,
        })
    }

    fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
}

#[async_trait]
impl Memory for ScopedMemory {
    fn name(&self) -> &str {
        "scoped_sqlx"
    }

    async fn store(&self, key: &str, content: &str) -> Result<(), MemoryError> {
        ai_memory::store_memory(
            &self.pool,
            self.tenant(),
            self.agent,
            self.user,
            key,
            content,
            "core",
            importance_for("core", content),
            false,
        )
        .await
        .map(|_| ())
        .map_err(|e| MemoryError::Store(e.to_string()))
    }

    async fn recall(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let limit = limit.clamp(1, 50) as i64;
        ai_memory::recall_memories(
            &self.pool,
            self.agent,
            self.user,
            self.tenant(),
            query,
            limit,
        )
        .await
        .map(|rows| {
            rows.into_iter()
                // Host-managed tiers (daily logs) never surface as model
                // memory; only durable Core facts are recallable/injected.
                .filter(|m| m.category == "core")
                .map(|m| MemoryEntry {
                    key: m.key,
                    content: m.content,
                })
                .collect()
        })
        .map_err(|e| MemoryError::Recall(e.to_string()))
    }

    async fn forget(&self, key: &str) -> Result<bool, MemoryError> {
        ai_memory::forget_memory(&self.pool, self.agent, self.user, key, self.tenant())
            .await
            .map_err(|e| MemoryError::Forget(e.to_string()))
    }
}

/// Heuristic importance scorer (port of zeroclaw `importance.rs`):
/// `category` base score + keyword boost (capped +0.2), total clamped to 1.0.
/// Mirrors zeroclaw exactly: base Core 0.7 / Daily 0.3 / Conversation 0.2 /
/// Custom 0.4; high-signal keywords each +0.1 up to +0.2.
pub fn importance_for(category: &str, content: &str) -> f64 {
    let base = match category {
        "core" => 0.7,
        "daily" => 0.3,
        "conversation" => 0.2,
        _ => 0.4,
    };
    const HIGH_SIGNAL_KEYWORDS: &[&str] = &[
        "decision",
        "always",
        "never",
        "important",
        "critical",
        "must",
        "requirement",
        "policy",
        "rule",
        "principle",
    ];
    let lowered = content.to_ascii_lowercase();
    let matches = HIGH_SIGNAL_KEYWORDS
        .iter()
        .filter(|kw| lowered.contains(**kw))
        .count();
    let boost = (matches as f64 * 0.1).min(0.2);
    (base + boost).min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_base_and_keyword_boost_capped() {
        assert!((importance_for("core", "plain fact") - 0.7).abs() < f64::EPSILON);
        let scored = importance_for(
            "core",
            "This is a critical decision that must always be followed",
        );
        assert!(scored > 0.85, "score: {scored}");
        let saturated = importance_for(
            "core",
            "important critical decision rule policy must always never requirement principle",
        );
        assert!(saturated <= 1.0);
    }

    #[test]
    fn category_bases_follow_zeroclaw() {
        assert!((importance_for("daily", "x") - 0.3).abs() < f64::EPSILON);
        assert!((importance_for("conversation", "x") - 0.2).abs() < f64::EPSILON);
        assert!((importance_for("custom", "x") - 0.4).abs() < f64::EPSILON);
    }
}
