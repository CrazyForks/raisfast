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
                let schema = ContentTypeSchema::parse_from_file(&path)?;
                tracing::info!(
                    "loaded content type: {} (table={})",
                    schema.name,
                    schema.table
                );
                registry.register(schema, rule_config, reserved_segments)?;
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

        let mut schema = schema;
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
