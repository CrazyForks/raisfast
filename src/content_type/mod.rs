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
//! - `WordPress` Custom Post Type
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
pub mod schema;
pub mod validation;

use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use schema::ContentTypeSchema;

use crate::errors::app_error::AppError;

/// 内容类型注册表
///
/// 管理所有已注册的 content type schema，提供按名称/表名查询能力。
/// 内部使用 `RwLock` 支持运行时热更新。
#[derive(Debug, Default)]
pub struct ContentTypeRegistry {
    inner: RwLock<RegistryInner>,
}

#[derive(Debug, Clone, Default)]
struct RegistryInner {
    types: HashMap<String, ContentTypeSchema>,
    by_table: HashMap<String, String>,
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
    pub fn load_from_dir(dir: &Path) -> Result<Self, AppError> {
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
                registry.register(schema);
            }
        }

        let count = registry.len();
        tracing::info!("loaded {} content type(s)", count);
        Ok(registry)
    }

    /// 注册单个 content type（线程安全）。
    ///
    /// 如果 CT 表名与系统保护表冲突则跳过并打印警告。
    pub fn register(&self, schema: ContentTypeSchema) {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        let protected = inner.protected_tables.clone();
        drop(inner);

        if crate::plugins::permissions::PermissionChecker::is_protected_table(
            &schema.table,
            &protected,
        ) {
            tracing::warn!(
                "skipping content type '{}': table '{}' is a protected system table",
                schema.name,
                schema.table
            );
            return;
        }
        let mut inner = self
            .inner
            .write()
            .expect("ContentTypeRegistry lock poisoned");
        inner
            .by_table
            .insert(schema.table.clone(), schema.singular.clone());
        inner.types.insert(schema.singular.clone(), schema);
    }

    /// 设置系统保护表列表
    pub fn set_protected_tables(&self, tables: Vec<String>) {
        let mut inner = self
            .inner
            .write()
            .expect("ContentTypeRegistry lock poisoned");
        inner.protected_tables = tables;
    }

    /// 按 singular name 查询
    #[must_use]
    pub fn get(&self, name: &str) -> Option<ContentTypeSchema> {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        inner.types.get(name).cloned()
    }

    /// 按表名查询
    #[must_use]
    pub fn get_by_table(&self, table: &str) -> Option<ContentTypeSchema> {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        inner
            .by_table
            .get(table)
            .and_then(|singular| inner.types.get(singular).cloned())
    }

    /// 获取所有已注册 content type
    #[must_use]
    pub fn all(&self) -> Vec<ContentTypeSchema> {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        inner.types.values().cloned().collect()
    }

    /// 已注册数量
    #[must_use]
    pub fn len(&self) -> usize {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        inner.types.len()
    }

    /// 是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 按 plural name 查询
    #[must_use]
    pub fn get_by_plural(&self, plural: &str) -> Option<ContentTypeSchema> {
        let inner = self
            .inner
            .read()
            .expect("ContentTypeRegistry lock poisoned");
        inner.types.values().find(|ct| ct.plural == plural).cloned()
    }

    /// 注销单个 content type（线程安全）
    pub fn unregister(&self, singular: &str) -> Option<ContentTypeSchema> {
        let mut inner = self
            .inner
            .write()
            .expect("ContentTypeRegistry lock poisoned");
        if let Some(schema) = inner.types.remove(singular) {
            inner.by_table.remove(&schema.table);
            Some(schema)
        } else {
            None
        }
    }
}
