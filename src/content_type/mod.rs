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

pub mod handler;
pub mod migration;
pub mod repository;
pub mod schema;

use std::collections::HashMap;
use std::path::Path;

use schema::ContentTypeSchema;

use crate::errors::app_error::AppError;

/// 内容类型注册表
///
/// 管理所有已注册的 content type schema，提供按名称/表名查询能力。
#[derive(Debug, Clone, Default)]
pub struct ContentTypeRegistry {
    types: HashMap<String, ContentTypeSchema>,
}

impl ContentTypeRegistry {
    /// 创建空注册表
    #[must_use]
    pub fn new() -> Self {
        Self {
            types: HashMap::new(),
        }
    }

    /// 从目录加载所有 TOML 定义
    ///
    /// 扫描 `dir` 下所有 `*.toml` 文件，解析为 `ContentTypeSchema` 并注册。
    pub fn load_from_dir(dir: &Path) -> Result<Self, AppError> {
        let mut registry = Self::new();
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

        tracing::info!("loaded {} content type(s)", registry.types.len());
        Ok(registry)
    }

    /// 注册单个 content type
    pub fn register(&mut self, schema: ContentTypeSchema) {
        self.types.insert(schema.singular.clone(), schema);
    }

    /// 按单数名称查询（如 "post"）
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ContentTypeSchema> {
        self.types.get(name)
    }

    /// 按表名查询
    #[must_use]
    pub fn get_by_table(&self, table: &str) -> Option<&ContentTypeSchema> {
        self.types.values().find(|ct| ct.table == table)
    }

    /// 获取所有已注册 content type
    #[must_use]
    pub fn all(&self) -> Vec<&ContentTypeSchema> {
        self.types.values().collect()
    }

    /// 已注册数量
    #[must_use]
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// 是否为空
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
}
