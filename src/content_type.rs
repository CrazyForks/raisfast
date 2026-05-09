//! 动态内容类型引擎
//!
//! 提供 Schema-Driven 的内容管理系统核心：
//! - 从 TOML 文件解析内容类型定义
//! - 自动生成数据库 Migration
//! - 泛型 CRUD Repository 和 API Handler
//! - 字段校验与关系解析
//!
//! # 设计参考
//!
//! - Strapi v5 Content Type Builder
//!
//! # 使用流程
//!
//! 1. 在 `content_types/` 目录创建 TOML 定义文件
//! 2. 启动时 `ContentTypeRegistry::load_from_dir()` 加载所有 schema
//! 3. `SchemaMigrator::migrate()` 自动建表/加列
//! 4. `register_content_routes()` 自动注册 CRUD API
//!
//! # 运行时热更新
//!
//! `ContentTypeRegistry` 内部使用 `RwLock`，支持运行时增删改 schema。
//! 新增的 content type 通过 catch-all 动态路由处理，无需重启服务。

pub mod handler;
pub mod migration;
pub mod repository;
pub mod resolver;
pub mod rule_engine;
pub mod schema;
pub mod validation;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arc_swap::ArcSwap;
use schema::ContentTypeSchema;

use crate::errors::app_error::AppError;

/// 内容类型注册表
///
/// 管理所有已注册的 content type schema，提供按名称/表名查询能力。
/// 内部使用 `ArcSwap` 实现无锁读、低开销写，支持运行时热更新。
/// 所有查询返回 `Arc<ContentTypeSchema>` 避免深拷贝。
#[derive(Debug, Default)]
pub struct ContentTypeRegistry {
    inner: ArcSwap<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    types: HashMap<String, Arc<ContentTypeSchema>>,
    by_table: HashMap<String, String>,
    by_plural: HashMap<String, String>,
    protected_tables: Vec<String>,
}

impl ContentTypeRegistry {
    /// 创建空注册表
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 从目录加载所有 TOML 定义
    ///
    /// 扫描 `dir` 下所有 `*.toml` 文件，解析为 `ContentTypeSchema` 并注册。
    pub fn load_from_dir(
        dir: &Path,
        rule_config: &crate::config::app::RuleEngineConfig,
        reserved_segments: &[&str],
        valid_protocols: &[&str],
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<Self, AppError> {
        let registry = Self::new();
        let entries = std::fs::read_dir(dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "cannot read content_types dir {dir:?}: {e}"
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let schema = match ContentTypeSchema::parse_from_file(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("skipping {file_name}: parse error: {e}");
                        continue;
                    }
                };
                tracing::info!(
                    "loaded content type: {} (table={})",
                    schema.name,
                    schema.table
                );
                if let Err(e) = registry.register(
                    schema,
                    rule_config,
                    reserved_segments,
                    valid_protocols,
                    protocol_registry,
                ) {
                    tracing::warn!("skipping {file_name}: register error: {e}");
                }
            }
        }

        let count = registry.len();
        tracing::info!("loaded {} content type(s)", count);
        Ok(registry)
    }

    /// 注册单个 content type（线程安全）。
    ///
    /// 检查 singular/plural/table 全局唯一性：
    /// - table 不能和系统保护表冲突
    /// - singular/plural/table 不能和其他已注册 content type 冲突
    /// - singular/plural 不能和内置路由段冲突（posts, categories, tags 等）
    pub fn register(
        &self,
        schema: ContentTypeSchema,
        rule_config: &crate::config::app::RuleEngineConfig,
        reserved_segments: &[&str],
        valid_protocols: &[&str],
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<(), AppError> {
        let mut conflicts = Vec::new();

        let protected = {
            let guard = self.inner.load();
            guard.protected_tables.clone()
        };

        if crate::plugins::permissions::PermissionChecker::is_protected_table(
            &schema.table,
            &protected,
        ) {
            conflicts.push(format!(
                "table '{}' is a protected system table",
                schema.table
            ));
        }

        if reserved_segments.contains(&schema.singular.as_str()) {
            conflicts.push(format!(
                "singular '{}' conflicts with a built-in route",
                schema.singular
            ));
        }
        if reserved_segments.contains(&schema.plural.as_str()) {
            conflicts.push(format!(
                "plural '{}' conflicts with a built-in route",
                schema.plural
            ));
        }

        {
            let guard = self.inner.load();

            if let Some(existing) = guard.types.get(&schema.singular)
                && (existing.table != schema.table || existing.plural != schema.plural)
            {
                conflicts.push(format!(
                    "singular '{}' already used by '{}'",
                    schema.singular, existing.name
                ));
            }
            if let Some(conflict_singular) = guard.by_plural.get(&schema.plural)
                && conflict_singular != &schema.singular
            {
                let name = guard
                    .types
                    .get(conflict_singular)
                    .map(|ct| ct.name.as_str())
                    .unwrap_or(conflict_singular);
                conflicts.push(format!(
                    "plural '{}' already used by '{}'",
                    schema.plural, name
                ));
            }
            if let Some(conflict_singular) = guard.by_table.get(&schema.table)
                && conflict_singular != &schema.singular
            {
                let name = guard
                    .types
                    .get(conflict_singular)
                    .map(|ct| ct.name.as_str())
                    .unwrap_or(conflict_singular);
                conflicts.push(format!(
                    "table '{}' already used by '{}'",
                    schema.table, name
                ));
            }
        }

        if !conflicts.is_empty() {
            tracing::warn!(
                "content type '{}' registration failed: {}",
                schema.name,
                conflicts.join("; ")
            );
            return Err(AppError::Internal(anyhow::anyhow!(
                "content type '{}' registration failed: {}",
                schema.name,
                conflicts.join("; ")
            )));
        }

        let unknown: Vec<&str> = schema
            .implements
            .iter()
            .map(|s| s.name())
            .filter(|name| !valid_protocols.contains(name))
            .collect();
        if !unknown.is_empty() {
            return Err(AppError::BadRequest(format!(
                "content type '{}' references unknown protocol(s): {}",
                schema.name,
                unknown.join(", ")
            )));
        }

        let mut schema = schema;
        schema.cache_protocol_columns(protocol_registry);
        schema.cache_select_columns();
        schema.cache_rules(rule_config);
        let plural = schema.plural.clone();
        let table = schema.table.clone();
        let singular = schema.singular.clone();
        let arc = Arc::new(schema);

        self.inner.rcu(|inner| {
            let mut new_inner = RegistryInner {
                types: inner.types.clone(),
                by_table: inner.by_table.clone(),
                by_plural: inner.by_plural.clone(),
                protected_tables: inner.protected_tables.clone(),
            };
            new_inner.by_table.insert(table.clone(), singular.clone());
            new_inner.by_plural.insert(plural.clone(), singular.clone());
            new_inner.types.insert(singular.clone(), arc.clone());
            new_inner
        });

        Ok(())
    }

    /// 设置系统保护表列表
    pub fn set_protected_tables(&self, tables: Vec<String>) {
        self.inner.rcu(|inner| RegistryInner {
            types: inner.types.clone(),
            by_table: inner.by_table.clone(),
            by_plural: inner.by_plural.clone(),
            protected_tables: tables.clone(),
        });
    }

    /// 按 singular name 查询
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard.types.get(name).cloned()
    }

    /// 按表名查询
    #[must_use]
    pub fn get_by_table(&self, table: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard
            .by_table
            .get(table)
            .and_then(|singular| guard.types.get(singular).cloned())
    }

    /// 获取所有已注册 content type
    #[must_use]
    pub fn all(&self) -> Vec<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard.types.values().cloned().collect()
    }

    /// 已注册数量
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.load().types.len()
    }

    /// 是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按 plural name 查询（O(1) HashMap 查找）
    #[must_use]
    pub fn get_by_plural(&self, plural: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard
            .by_plural
            .get(plural)
            .and_then(|singular| guard.types.get(singular).cloned())
    }

    /// 注销单个 content type（线程安全）
    pub fn unregister(&self, singular: &str) -> Option<Arc<ContentTypeSchema>> {
        let removed = {
            let guard = self.inner.load();
            guard.types.get(singular).cloned()
        };

        if let Some(schema) = &removed {
            let table = schema.table.clone();
            let plural = schema.plural.clone();
            let singular_owned = singular.to_string();
            self.inner.rcu(|inner| {
                let mut new_inner = RegistryInner {
                    types: inner.types.clone(),
                    by_table: inner.by_table.clone(),
                    by_plural: inner.by_plural.clone(),
                    protected_tables: inner.protected_tables.clone(),
                };
                new_inner.types.remove(&singular_owned);
                new_inner.by_table.remove(&table);
                new_inner.by_plural.remove(&plural);
                new_inner
            });
        }

        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::RuleEngineConfig;

    fn test_protocol_registry() -> crate::protocols::ProtocolRegistry {
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(crate::protocols::timestampable::TimestampableProtocol);
        reg.register(crate::protocols::soft_deletable::SoftDeletableProtocol);
        reg.register(crate::protocols::versionable::VersionableProtocol);
        reg
    }

    fn valid_protocols() -> Vec<&'static str> {
        vec!["ownable", "timestampable", "soft_deletable", "versionable"]
    }

    fn register_ct(
        reg: &ContentTypeRegistry,
        singular: &str,
        plural: &str,
        table: &str,
    ) -> Result<(), AppError> {
        let toml = format!(
            r#"
[content_type]
name = "{singular}"
singular = "{singular}"
plural = "{plural}"
table = "{table}"

[fields.title]
type = "text"
"#
        );
        let schema = schema::ContentTypeSchema::parse_from_str(&toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = ContentTypeRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_and_lookup() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("product").is_some());
        assert!(reg.get_by_table("products").is_some());
        assert!(reg.get_by_plural("products").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn register_conflict_table() {
        let reg = ContentTypeRegistry::new();
        reg.set_protected_tables(vec!["users".into()]);
        let toml = r#"
[content_type]
name = "User"
singular = "custom_user"
plural = "custom_users"
table = "users"

[fields.name]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_conflict_reserved_singular() {
        let reg = ContentTypeRegistry::new();
        let toml = r#"
[content_type]
name = "Auth"
singular = "auth"
plural = "auths"
table = "auth_stuff"

[fields.name]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &["auth"],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_unknown_protocol() {
        let reg = ContentTypeRegistry::new();
        let toml = r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"
implements = ["nonexistent_protocol"]

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn unregister_removes_ct() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        let removed = reg.unregister("product").unwrap();
        assert_eq!(removed.singular, "product");
        assert!(reg.is_empty());
        assert!(reg.get("product").is_none());
        assert!(reg.get_by_table("products").is_none());
        assert!(reg.get_by_plural("products").is_none());
    }

    #[test]
    fn unregister_nonexistent_returns_none() {
        let reg = ContentTypeRegistry::new();
        assert!(reg.unregister("ghost").is_none());
    }

    #[test]
    fn all_returns_all_registered() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "a", "as", "table_a").unwrap();
        register_ct(&reg, "b", "bs", "table_b").unwrap();
        let all = reg.all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn set_protected_tables_updates() {
        let reg = ContentTypeRegistry::new();
        reg.set_protected_tables(vec!["posts".into(), "users".into()]);
        let toml = r#"
[content_type]
name = "P"
singular = "p"
plural = "ps"
table = "posts"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_duplicate_singular_different_table() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        let toml = r#"
[content_type]
name = "Product2"
singular = "product"
plural = "product2s"
table = "product2s"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }
}
