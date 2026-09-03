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
}

impl ScopedMemory {
    pub fn new(pool: crate::db::Pool, tenant: Option<String>, agent: SnowflakeId) -> Arc<Self> {
        Arc::new(Self {
            pool,
            tenant,
            agent,
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
            key,
            content,
            "core",
            0.5,
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
        ai_memory::recall_memories(&self.pool, self.agent, self.tenant(), query, limit)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|m| MemoryEntry {
                        key: m.key,
                        content: m.content,
                    })
                    .collect()
            })
            .map_err(|e| MemoryError::Recall(e.to_string()))
    }

    async fn forget(&self, key: &str) -> Result<bool, MemoryError> {
        ai_memory::forget_memory(&self.pool, self.agent, key, self.tenant())
            .await
            .map_err(|e| MemoryError::Forget(e.to_string()))
    }
}
