//! Extension 清单 (extension.toml) 解析

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::app_error::{AppError, AppResult};

/// extension.toml 顶层结构
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionManifest {
    pub extension: ExtensionInfo,
}

/// Extension 基本信息
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionInfo {
    /// 全局唯一标识（kebab-case）
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 语义化版本
    pub version: String,
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 作者
    pub author: Option<String>,
    /// 开源协议
    pub license: Option<String>,
    /// 主页 URL
    pub homepage: Option<String>,
    /// 依赖的其他 Extension（id → semver range）
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    /// Content Type 目录相对路径（None = 无 CT）
    pub content_types: Option<String>,
    /// Plugin manifest 相对路径（None = 无 Plugin）
    pub plugin: Option<String>,
}

impl ExtensionManifest {
    /// 从文件解析 extension.toml
    pub fn parse_from_file(path: &Path) -> AppResult<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "cannot read extension manifest {:?}: {e}",
                path
            ))
        })?;
        Self::parse_from_str(&content)
    }

    /// 从字符串解析 extension.toml
    pub fn parse_from_str(content: &str) -> AppResult<Self> {
        toml::from_str(content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("failed to parse extension.toml: {e}")))
    }

    /// Content Type 目录的绝对路径
    pub fn content_types_dir(&self, extension_root: &Path) -> Option<std::path::PathBuf> {
        self.extension
            .content_types
            .as_ref()
            .map(|relative| extension_root.join(relative))
    }

    /// Plugin manifest 的绝对路径
    pub fn plugin_manifest_path(&self, extension_root: &Path) -> Option<std::path::PathBuf> {
        self.extension
            .plugin
            .as_ref()
            .map(|relative| extension_root.join(relative))
    }

    /// 是否包含 Content Type
    pub fn has_content_types(&self) -> bool {
        self.extension.content_types.is_some()
    }

    /// 是否包含 Plugin
    pub fn has_plugin(&self) -> bool {
        self.extension.plugin.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[extension]
id = "blog-core"
name = "Blog Core"
version = "1.0.0"
"#;
        let manifest = ExtensionManifest::parse_from_str(toml).unwrap();
        assert_eq!(manifest.extension.id, "blog-core");
        assert_eq!(manifest.extension.name, "Blog Core");
        assert_eq!(manifest.extension.version, "1.0.0");
        assert!(!manifest.has_content_types());
        assert!(!manifest.has_plugin());
        assert!(manifest.extension.dependencies.is_empty());
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[extension]
id = "ecommerce"
name = "E-Commerce"
version = "2.1.0"
description = "E-commerce extension"
author = "Team"
license = "MIT"
homepage = "https://example.com"
content_types = "content_types/"
plugin = "plugin/manifest.toml"

[extension.dependencies]
blog-core = ">=1.0.0"
"#;
        let manifest = ExtensionManifest::parse_from_str(toml).unwrap();
        assert_eq!(manifest.extension.id, "ecommerce");
        assert!(manifest.has_content_types());
        assert!(manifest.has_plugin());
        assert_eq!(
            manifest.extension.dependencies.get("blog-core"),
            Some(&">=1.0.0".to_string())
        );

        let root = Path::new("/extensions/ecommerce");
        assert_eq!(
            manifest.content_types_dir(root),
            Some(root.join("content_types/"))
        );
        assert_eq!(
            manifest.plugin_manifest_path(root),
            Some(root.join("plugin/manifest.toml"))
        );
    }

    #[test]
    fn parse_ct_only() {
        let toml = r#"
[extension]
id = "blog-core"
name = "Blog Core"
version = "1.0.0"
content_types = "ct/"
"#;
        let manifest = ExtensionManifest::parse_from_str(toml).unwrap();
        assert!(manifest.has_content_types());
        assert!(!manifest.has_plugin());
    }

    #[test]
    fn parse_plugin_only() {
        let toml = r#"
[extension]
id = "seo"
name = "SEO"
version = "0.1.0"
plugin = "plugin/manifest.toml"
"#;
        let manifest = ExtensionManifest::parse_from_str(toml).unwrap();
        assert!(!manifest.has_content_types());
        assert!(manifest.has_plugin());
    }

    #[test]
    fn parse_invalid_toml_fails() {
        let toml = "not valid toml {{{";
        assert!(ExtensionManifest::parse_from_str(toml).is_err());
    }

    #[test]
    fn parse_missing_id_fails() {
        let toml = r#"
[extension]
name = "No ID"
version = "1.0.0"
"#;
        assert!(ExtensionManifest::parse_from_str(toml).is_err());
    }
}
