//! Protocol layer — Declarative protocol definitions
//!
//! Protocol = Aspect composition + declarative effects (ProtocolDeclaration).
//!
//! ## Design
//!
//! ```text
//! Protocol implementation
//!   ├─ aspects()        → Aspect list (AOP data injection)
//!   ├─ declaration()    → ProtocolDeclaration (pure data, compile-time safe)
//!   │     ├─ query_filters     — automatically appended WHERE conditions
//!   │     ├─ delete_strategy   — Soft / Hard
//!   │     ├─ snapshot_before_update — snapshot old record before update
//!   │     └─ revision_routes   — provide version history API
//!   └─ on_after_delete() → async hook (the only non-pure-data method)
//! ```
//!
//! ## Extending Protocols
//!
//! **Scenario A (common): new protocol only uses existing capabilities** → just add 1 file
//!
//! **Scenario B (rare): introduces a brand-new system integration point** → extend the
//! ProtocolDeclaration struct, and the compiler will error on all sites that need adaptation.

pub mod expirable;
pub mod lockable;
pub mod metaable;
pub mod nestable;
pub mod ownable;
pub mod soft_deletable;
pub mod sortable;
pub mod statusable;
pub mod tenantable;
pub mod timestampable;
pub mod versionable;

use crate::types::snowflake_id::SnowflakeId;
use std::collections::HashMap;
use std::sync::Arc;

use crate::aspects::{Aspect, ColumnDef};

// ─── DeleteStrategy ───

/// Delete strategy
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeleteStrategy {
    /// Physical DELETE (default)
    #[default]
    Hard,
    /// UPDATE SET column = now WHERE ...
    Soft { column: String },
}

// ─── SortDir ───

/// Sort direction
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

// ─── ProtocolDeclaration ───

/// Status mode
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusMode {
    #[default]
    String,
    Numeric,
}

/// All declarative effects a protocol has on the system
///
/// Pure data struct. New capabilities are added as fields.
/// Consumers read fields directly; the compiler guarantees type safety.
/// When extending this struct, the compiler will flag all sites needing adaptation.
#[derive(Debug, Clone, Default)]
pub struct ProtocolDeclaration {
    /// Query filters automatically appended as WHERE conditions: (column, SQL_condition)
    pub query_filters: Vec<(String, String)>,
    /// Delete strategy
    pub delete_strategy: DeleteStrategy,
    /// Whether to snapshot the current record before update (for version history)
    pub snapshot_before_update: bool,
    /// Whether to provide version history API routes (/revisions)
    pub revision_routes: bool,
    /// Optimistic lock column name (UPDATE WHERE column = ? + SET column = column + 1)
    pub lock_column: Option<String>,
    /// Default sort for list queries (column, direction)
    pub default_sort: Option<(String, SortDir)>,
    /// statusable: allowed status values
    pub status_values: Option<Vec<String>>,
    /// statusable: numeric mapping (label → number)
    pub status_map: Option<Vec<(String, i64)>>,
    /// statusable: default status value (label in string mode, label in numeric mode)
    pub status_default: Option<String>,
    /// statusable: storage mode
    pub status_mode: StatusMode,
}

impl ProtocolDeclaration {
    /// Merge another protocol declaration (Soft takes priority over Hard, bool uses OR, lock/sort: last wins)
    pub fn merge(&mut self, other: &ProtocolDeclaration) {
        self.query_filters
            .extend(other.query_filters.iter().cloned());
        if matches!(other.delete_strategy, DeleteStrategy::Soft { .. }) {
            self.delete_strategy = other.delete_strategy.clone();
        }
        if other.snapshot_before_update {
            self.snapshot_before_update = true;
        }
        if other.revision_routes {
            self.revision_routes = true;
        }
        if other.lock_column.is_some() {
            if self.lock_column.is_some() {
                tracing::warn!(
                    "conflict: lock_column already set, overwriting with {:?}",
                    other.lock_column
                );
            }
            self.lock_column = other.lock_column.clone();
        }
        if other.default_sort.is_some() {
            if self.default_sort.is_some() {
                tracing::warn!(
                    "conflict: default_sort already set, overwriting with {:?}",
                    other.default_sort
                );
            }
            self.default_sort = other.default_sort.clone();
        }
        if other.status_values.is_some() {
            self.status_values = other.status_values.clone();
        }
        if other.status_map.is_some() {
            self.status_map = other.status_map.clone();
        }
        if other.status_default.is_some() {
            self.status_default = other.status_default.clone();
        }
        if matches!(other.status_mode, StatusMode::Numeric) {
            self.status_mode = StatusMode::Numeric;
        }
    }

    /// Aggregate declarations from multiple protocols
    pub fn aggregated(names: &[String], registry: &ProtocolRegistry) -> Self {
        let mut agg = Self::default();
        for name in names {
            if let Some(protocol) = registry.get(name) {
                agg.merge(&protocol.declaration());
            }
        }
        agg.query_filters.sort_by(|a, b| a.0.cmp(&b.0));
        agg
    }

    pub fn is_soft_delete(&self) -> bool {
        matches!(self.delete_strategy, DeleteStrategy::Soft { .. })
    }

    pub fn is_lockable(&self) -> bool {
        self.lock_column.is_some()
    }

    pub fn is_sortable(&self) -> bool {
        self.default_sort.is_some()
    }
}

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

    fn behaviors(&self) -> Vec<&'static str> {
        vec![]
    }

    fn built_in(&self) -> bool {
        false
    }

    /// Protocol declarative effects (pure data)
    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration::default()
    }

    /// Apply user configuration to the declaration (e.g., sortable's field/direction)
    ///
    /// Default no-op. Protocols that need configuration override this method.
    fn apply_config(
        &self,
        _config: &HashMap<String, String>,
        _decl: &mut ProtocolDeclaration,
        _all_columns: &[&str],
    ) {
    }

    /// Register additional API routes required by the protocol
    ///
    /// Default no-op. Protocols that need routes override this method.
    fn register_routes(
        &self,
        _router: axum::Router<crate::AppState>,
        _plural: &str,
        _admin_prefix: &str,
    ) -> axum::Router<crate::AppState> {
        _router
    }

    /// Async callback after record deletion (e.g., cleanup related tables)
    ///
    /// This is the only hook that cannot be purely data-declared, because it involves
    /// async IO operations. Most protocols use the default no-op implementation.
    fn on_after_delete(
        &self,
        _pool: &crate::db::pool::Pool,
        _content_type_singular: &str,
        _record_id: SnowflakeId,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), anyhow::Error>> + Send + '_>>
    {
        Box::pin(async { Ok(()) })
    }
}

// ─── ProtocolEntry (inventory) ───

pub struct ProtocolEntry {
    pub factory: fn() -> Arc<dyn Protocol>,
}

inventory::collect!(ProtocolEntry);

/// Macro for self-registration within protocol files
#[macro_export]
macro_rules! register_protocol {
    ($protocol_type:ty, $instance:expr) => {
        ::inventory::submit! {
            $crate::protocols::ProtocolEntry {
                factory: || std::sync::Arc::new($instance),
            }
        }
    };
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

    pub fn register_from_inventory(&mut self) {
        for entry in inventory::iter::<ProtocolEntry> {
            let name = (entry.factory)().name().to_string();
            self.protocols.insert(name, (entry.factory)());
        }
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Protocol>> {
        self.protocols.get(name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.protocols.keys().map(|s| s.as_str()).collect()
    }

    pub fn columns_for(&self, names: &[String]) -> Vec<ColumnDef> {
        let mut cols = Vec::new();
        let mut seen: HashMap<String, (String, ColumnDef)> = HashMap::new();
        for name in names {
            if let Some(protocol) = self.protocols.get(name.as_str()) {
                for col in protocol.columns() {
                    if let Some((prev_proto, prev_col)) = seen.get(&col.name) {
                        if prev_col.sql_type != col.sql_type || prev_col.default != col.default {
                            tracing::warn!(
                                "column '{}' declared by '{}' ({:?}) and '{}' ({:?}): first wins",
                                col.name,
                                prev_proto,
                                prev_col.sql_type,
                                name,
                                col.sql_type,
                            );
                        }
                        continue;
                    }
                    seen.insert(col.name.clone(), (name.clone(), col.clone()));
                    cols.push(col);
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

    /// Aggregate declarations from multiple protocols
    pub fn declaration_for(&self, names: &[String]) -> ProtocolDeclaration {
        ProtocolDeclaration::aggregated(names, self)
    }

    /// Apply user configuration to the aggregated declaration
    pub fn apply_config_for(
        &self,
        impl_refs: &[crate::content_type::schema::ProtocolRef],
        decl: &mut ProtocolDeclaration,
        all_columns: &[&str],
    ) {
        for pref in impl_refs {
            if let Some(protocol) = self.protocols.get(pref.name()) {
                protocol.apply_config(pref.config(), decl, all_columns);
            }
        }
    }

    /// Register additional routes for all protocols
    pub fn register_routes_for(
        &self,
        names: &[String],
        router: axum::Router<crate::AppState>,
        plural: &str,
        admin_prefix: &str,
    ) -> axum::Router<crate::AppState> {
        let mut router = router;
        for name in names {
            if let Some(protocol) = self.protocols.get(name.as_str()) {
                router = protocol.register_routes(router, plural, admin_prefix);
            }
        }
        router
    }

    /// Post-delete callback
    pub async fn dispatch_after_delete(
        &self,
        names: &[String],
        pool: &crate::db::pool::Pool,
        content_type_singular: &str,
        record_id: SnowflakeId,
    ) -> Result<(), anyhow::Error> {
        for name in names {
            if let Some(protocol) = self.protocols.get(name.as_str()) {
                protocol
                    .on_after_delete(pool, content_type_singular, record_id)
                    .await?;
            }
        }
        Ok(())
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

    #[test]
    fn declaration_aggregation() {
        let mut reg = ProtocolRegistry::new();
        reg.register(soft_deletable::SoftDeletableProtocol);
        reg.register(versionable::VersionableProtocol);

        let sd = reg.declaration_for(&["soft_deletable".into()]);
        assert!(sd.is_soft_delete());
        assert_eq!(sd.query_filters.len(), 1);
        assert_eq!(sd.query_filters[0].0, "deleted_at");
        assert_eq!(sd.query_filters[0].1, "IS NULL");

        let ver = reg.declaration_for(&["versionable".into()]);
        assert!(!ver.is_soft_delete());
        assert!(ver.snapshot_before_update);
        assert!(ver.revision_routes);

        let both = reg.declaration_for(&["soft_deletable".into(), "versionable".into()]);
        assert!(both.is_soft_delete());
        assert!(both.snapshot_before_update);
        assert!(both.revision_routes);
        assert_eq!(both.query_filters.len(), 1);
    }

    /// Guard test: ensures merge() covers all ProtocolDeclaration fields.
    /// When adding new fields, this test will fail if merge() is not updated.
    #[test]
    fn merge_covers_all_declaration_fields() {
        let full = ProtocolDeclaration {
            query_filters: vec![("col_a".into(), "IS NULL".into())],
            delete_strategy: DeleteStrategy::Soft {
                column: "archived_at".into(),
            },
            snapshot_before_update: true,
            revision_routes: true,
            lock_column: Some("lock_version".into()),
            default_sort: Some(("priority".into(), SortDir::Desc)),
            status_values: Some(vec!["draft".into(), "published".into()]),
            status_map: Some(vec![("draft".into(), 1), ("published".into(), 10)]),
            status_default: Some("draft".into()),
            status_mode: StatusMode::Numeric,
        };

        let mut empty = ProtocolDeclaration::default();
        empty.merge(&full);

        assert_eq!(empty.query_filters.len(), 1);
        assert_eq!(empty.query_filters[0].0, "col_a");
        assert!(matches!(empty.delete_strategy, DeleteStrategy::Soft { .. }));
        assert!(empty.snapshot_before_update);
        assert!(empty.revision_routes);
        assert_eq!(empty.lock_column.as_deref(), Some("lock_version"));
        assert_eq!(
            empty.default_sort.as_ref().map(|(c, d)| (c.as_str(), *d)),
            Some(("priority", SortDir::Desc))
        );
        assert_eq!(
            empty.status_values,
            Some(vec!["draft".into(), "published".into()])
        );
        assert_eq!(
            empty.status_map,
            Some(vec![("draft".into(), 1), ("published".into(), 10)])
        );
        assert_eq!(empty.status_default, Some("draft".into()));
        assert_eq!(empty.status_mode, StatusMode::Numeric);
    }
}
