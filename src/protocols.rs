//! Protocol 层 — AOP 之上的薄声明层
//!
//! Protocol = 一组 Aspect 的命名别名 + 可选配置 + 列声明。
//! 一个 Protocol 可以组合多个 Aspect（1:N）。
//!
//! ## 设计
//!
//! ```text
//! ContentTypeSchema.implements = ["auditable"]
//!                          │
//!                          ▼
//! ProtocolRegistry.get("auditable")
//!   ├─ name: "auditable"
//!   ├─ description: "审计追踪"
//!   ├─ aspects: [OwnableAspect, TimestampableAspect]
//!   ├─ columns: [created_by, updated_by, created_at, updated_at]
//!   └─ built_in: false
//! ```
//!
//! ## 内置 Protocol
//!
//! - `ownable` — 注入 created_by / updated_by
//! - `timestampable` — 注入 created_at / updated_at
//!
//! ## 自定义 Protocol（未来）
//!
//! 用户可通过 manifest.toml 定义 Protocol，组合已有 Aspect。

pub mod cacheable;
pub mod ownable;
pub mod soft_deletable;
pub mod timestampable;
pub mod versionable;

use std::collections::HashMap;
use std::sync::Arc;

use crate::aspects::{Aspect, ColumnDef};

// ─── Protocol Trait ───

pub trait Protocol: Send + Sync + 'static {
    fn name(&self) -> &str;

    fn description(&self) -> &str {
        ""
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>>;

    fn columns(&self) -> Vec<ColumnDef> {
        self.aspects().iter().flat_map(|a| a.columns()).collect()
    }

    fn built_in(&self) -> bool {
        false
    }
}

// ─── ProtocolRegistry ───

pub struct ProtocolRegistry {
    protocols: HashMap<String, Arc<dyn Protocol>>,
}

impl ProtocolRegistry {
    pub fn new() -> Self {
        Self {
            protocols: HashMap::new(),
        }
    }

    pub fn register(&mut self, protocol: impl Protocol) {
        let name = protocol.name().to_string();
        self.protocols.insert(name, Arc::new(protocol));
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Protocol>> {
        self.protocols.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.protocols.keys().map(|s| s.as_str()).collect()
    }

    pub fn columns_for(&self, names: &[String]) -> Vec<ColumnDef> {
        let mut cols = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            if let Some(protocol) = self.protocols.get(name.as_str()) {
                for col in protocol.columns() {
                    if seen.insert(col.name.clone()) {
                        cols.push(col);
                    }
                }
            }
        }
        cols
    }

    pub fn aspects_for(&self, names: &[String]) -> Vec<Arc<dyn Aspect>> {
        let mut aspects = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for name in names {
            if let Some(protocol) = self.protocols.get(name.as_str()) {
                for aspect in protocol.aspects() {
                    if seen.insert(aspect.name().to_string()) {
                        aspects.push(aspect);
                    }
                }
            }
        }
        aspects
    }

    pub fn register_aspects_into(&self, engine: &crate::aspects::engine::AspectEngine) {
        let mut seen = std::collections::HashSet::new();
        for protocol in self.protocols.values() {
            for aspect in protocol.aspects() {
                if seen.insert(aspect.name().to_string()) {
                    engine.register_from_arc(aspect);
                }
            }
        }
    }
}

impl Default for ProtocolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ProtocolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.names();
        f.debug_struct("ProtocolRegistry")
            .field("protocols", &names)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_get() {
        let mut reg = ProtocolRegistry::new();
        reg.register(ownable::OwnableProtocol);
        assert!(reg.get("ownable").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn names_returns_all() {
        let mut reg = ProtocolRegistry::new();
        reg.register(ownable::OwnableProtocol);
        reg.register(timestampable::TimestampableProtocol);
        let mut names = reg.names();
        names.sort();
        assert_eq!(names, vec!["ownable", "timestampable"]);
    }

    #[test]
    fn columns_for_deduplicates() {
        let mut reg = ProtocolRegistry::new();
        reg.register(ownable::OwnableProtocol);
        reg.register(timestampable::TimestampableProtocol);
        let cols = reg.columns_for(&["ownable".into(), "timestampable".into()]);
        let col_names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"created_by"));
        assert!(col_names.contains(&"updated_by"));
        assert!(col_names.contains(&"created_at"));
        assert!(col_names.contains(&"updated_at"));
    }

    #[test]
    fn aspects_for_deduplicates() {
        let mut reg = ProtocolRegistry::new();
        reg.register(ownable::OwnableProtocol);
        reg.register(timestampable::TimestampableProtocol);
        let aspects = reg.aspects_for(&["ownable".into(), "timestampable".into()]);
        assert_eq!(aspects.len(), 2);
    }
}
