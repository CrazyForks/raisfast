//! Protocol 层 — 声明式协议定义
//!
//! Protocol = Aspect 组合 + 声明式效果（ProtocolDeclaration）。
//!
//! ## 设计
//!
//! ```text
//! Protocol 实现
//!   ├─ aspects()        → Aspect 列表（AOP 数据注入）
//!   ├─ declaration()    → ProtocolDeclaration（纯数据，编译期安全）
//!   │     ├─ query_filters     — 自动追加 WHERE 条件
//!   │     ├─ delete_strategy   — Soft / Hard
//!   │     ├─ snapshot_before_update — 更新前获取旧记录
//!   │     └─ revision_routes   — 提供版本历史 API
//!   └─ on_after_delete() → async hook（唯一非纯数据方法）
//! ```
//!
//! ## 扩展协议
//!
//! **场景 A（常见）：新协议只用到已有能力** → 只加 1 个文件
//!
//! **场景 B（罕见）：引入全新系统集成点** → 扩展 ProtocolDeclaration struct，
//! 编译器会报错提醒所有需要适配的位置。

pub mod expirable;
pub mod lockable;
pub mod nestable;
pub mod ownable;
pub mod soft_deletable;
pub mod sortable;
pub mod timestampable;
pub mod versionable;

use std::collections::HashMap;
use std::sync::Arc;

use crate::aspects::{Aspect, ColumnDef};

// ─── DeleteStrategy ───

/// 删除策略
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DeleteStrategy {
    /// 物理 DELETE（默认）
    #[default]
    Hard,
    /// UPDATE SET column = now WHERE ...
    Soft { column: String },
}

// ─── SortDir ───

/// 排序方向
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

// ─── ProtocolDeclaration ───

/// 协议对系统的所有声明式效果
///
/// 纯数据 struct，新能力加字段。
/// 消费方直接读字段，编译器保证类型安全。
/// 扩展此 struct 时，编译器会提醒所有需要适配的位置。
#[derive(Debug, Clone, Default)]
pub struct ProtocolDeclaration {
    /// 查询时自动追加的 WHERE 过滤条件: (column, SQL_condition)
    pub query_filters: Vec<(String, String)>,
    /// 删除策略
    pub delete_strategy: DeleteStrategy,
    /// 更新前是否获取当前记录快照（用于版本历史）
    pub snapshot_before_update: bool,
    /// 是否提供版本历史 API 路由（/revisions）
    pub revision_routes: bool,
    /// 乐观锁列名（UPDATE WHERE column = ? + SET column = column + 1）
    pub lock_column: Option<String>,
    /// 列表查询的默认排序 (column, direction)
    pub default_sort: Option<(String, SortDir)>,
}

impl ProtocolDeclaration {
    /// 合并另一个协议声明（Soft 优先于 Hard，bool 取 OR，lock/sort 后覆盖前）
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
    }

    /// 聚合多个协议的声明
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

    /// 协议声明式效果（纯数据）
    fn declaration(&self) -> ProtocolDeclaration {
        ProtocolDeclaration::default()
    }

    /// 将用户配置应用到声明（如 sortable 的 field/direction）
    ///
    /// 默认空实现。需要配置的协议覆写此方法。
    fn apply_config(
        &self,
        _config: &HashMap<String, String>,
        _decl: &mut ProtocolDeclaration,
        _all_columns: &[&str],
    ) {
    }

    /// 注册协议所需的额外 API 路由
    ///
    /// 默认空实现。需要路由的协议覆写此方法。
    fn register_routes(
        &self,
        _router: axum::Router<crate::AppState>,
        _plural: &str,
        _admin_prefix: &str,
    ) -> axum::Router<crate::AppState> {
        _router
    }

    /// 删除记录后的异步回调（如清理关联表）
    ///
    /// 这是唯一无法纯数据声明的 hook，因为涉及异步 IO 操作。
    /// 大多数协议使用默认空实现。
    fn on_after_delete(
        &self,
        _pool: &crate::db::pool::Pool,
        _content_type_singular: &str,
        _record_id: &str,
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

/// 在协议文件内自注册的宏
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

    /// 聚合多个协议的声明
    pub fn declaration_for(&self, names: &[String]) -> ProtocolDeclaration {
        ProtocolDeclaration::aggregated(names, self)
    }

    /// 对聚合后的声明应用用户配置
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

    /// 注册所有协议的额外路由
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

    /// 删除后回调
    pub async fn dispatch_after_delete(
        &self,
        names: &[String],
        pool: &crate::db::pool::Pool,
        content_type_singular: &str,
        record_id: &str,
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

    /// 防护测试：确保 merge() 覆盖 ProtocolDeclaration 的所有字段。
    /// 新增字段时如果忘记更新 merge()，此测试会失败。
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
    }
}
