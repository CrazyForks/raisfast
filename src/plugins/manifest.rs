//! 插件清单 (plugin.toml) 解析

use serde::Deserialize;
use std::collections::HashMap;

/// 插件清单顶层结构
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default, deserialize_with = "deserialize_hooks")]
    pub hooks: HashMap<String, HookConfig>,
}

fn deserialize_hooks<'de, D>(de: D) -> Result<HashMap<String, HookConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, HookConfig> = HashMap::deserialize(de)?;
    Ok(raw
        .into_iter()
        .map(|(k, v)| (k.replace('-', "_"), v))
        .collect())
}

/// 插件基本信息
#[derive(Debug, Clone, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub author: Option<String>,
    pub license: Option<String>,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_wasm")]
    pub wasm: String,
    #[serde(default = "default_entry")]
    pub entry: String,
}

fn default_runtime() -> String {
    "wasm".into()
}

fn default_language() -> String {
    "rust".into()
}

fn default_wasm() -> String {
    "plugin.wasm".into()
}

fn default_entry() -> String {
    "index.js".into()
}

/// 插件权限声明
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub http: Vec<String>,
    #[serde(default)]
    pub config: Vec<String>,
    pub max_memory_mb: Option<u32>,
    pub timeout_ms: Option<u64>,
}

/// Hook 注册配置
#[derive(Debug, Clone, Deserialize)]
pub struct HookConfig {
    pub priority: Option<i32>,
    #[serde(rename = "match")]
    pub match_pattern: Option<String>,
}

/// Hook 点枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    PostCreating,
    PostCreated,
    PostUpdating,
    PostUpdated,
    PostDeleted,
    CommentCreating,
    CommentCreated,
    RenderMarkdown,
    FilterHtml,
    HandleRoute,
    OnLogin,
}

impl HookPoint {
    /// 返回对应的 WASM 导出函数名
    pub fn wasm_func_name(self) -> &'static str {
        match self {
            HookPoint::PostCreating => "on_post_creating",
            HookPoint::PostCreated => "on_post_created",
            HookPoint::PostUpdating => "on_post_updating",
            HookPoint::PostUpdated => "on_post_updated",
            HookPoint::PostDeleted => "on_post_deleted",
            HookPoint::CommentCreating => "on_comment_creating",
            HookPoint::CommentCreated => "on_comment_created",
            HookPoint::RenderMarkdown => "render_markdown",
            HookPoint::FilterHtml => "filter_html",
            HookPoint::HandleRoute => "handle_route",
            HookPoint::OnLogin => "on_login",
        }
    }

    /// 返回所有 Hook 点，用于遍历测试
    pub fn all() -> &'static [HookPoint] {
        &[
            HookPoint::PostCreating,
            HookPoint::PostCreated,
            HookPoint::PostUpdating,
            HookPoint::PostUpdated,
            HookPoint::PostDeleted,
            HookPoint::CommentCreating,
            HookPoint::CommentCreated,
            HookPoint::RenderMarkdown,
            HookPoint::FilterHtml,
            HookPoint::HandleRoute,
            HookPoint::OnLogin,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[plugin]
id = "com.example.test"
name = "Test Plugin"
version = "1.0.0"
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.plugin.id, "com.example.test");
        assert_eq!(m.plugin.name, "Test Plugin");
        assert_eq!(m.plugin.version, "1.0.0");
        assert_eq!(m.plugin.description, "");
        assert!(m.plugin.author.is_none());
        assert_eq!(m.plugin.runtime, "wasm");
        assert_eq!(m.plugin.language, "rust");
        assert!(m.permissions.http.is_empty());
        assert!(m.permissions.config.is_empty());
        assert!(m.permissions.max_memory_mb.is_none());
        assert!(m.permissions.timeout_ms.is_none());
        assert!(m.hooks.is_empty());
        assert_eq!(m.plugin.wasm, "plugin.wasm");
        assert_eq!(m.plugin.entry, "index.js");
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[plugin]
id = "com.example.seo"
name = "SEO Optimizer"
version = "2.1.0"
description = "Auto-generate meta descriptions"
author = "Example Corp"
license = "MIT"
runtime = "wasi"
language = "assemblyscript"
wasm = "seo_optimizer.wasm"

[permissions]
http = ["cdn.example.com/*", "api.example.com/v1/*"]
config = ["seo.*"]
max_memory_mb = 64
timeout_ms = 3000

[hooks.on-post-creating]
priority = 10

[hooks.render-markdown]
priority = 20

[hooks.handle-route]
match = "/api/v1/plugins/seo/*"
priority = 5
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();

        assert_eq!(m.plugin.id, "com.example.seo");
        assert_eq!(m.plugin.author, Some("Example Corp".into()));
        assert_eq!(m.plugin.license, Some("MIT".into()));
        assert_eq!(m.plugin.runtime, "wasi");
        assert_eq!(m.plugin.language, "assemblyscript");
        assert_eq!(m.plugin.wasm, "seo_optimizer.wasm");

        assert_eq!(
            m.permissions.http,
            vec!["cdn.example.com/*", "api.example.com/v1/*"]
        );
        assert_eq!(m.permissions.config, vec!["seo.*"]);
        assert_eq!(m.permissions.max_memory_mb, Some(64));
        assert_eq!(m.permissions.timeout_ms, Some(3000));

        assert_eq!(m.hooks.len(), 3);
        let hpc = m.hooks.get("on_post_creating").unwrap();
        assert_eq!(hpc.priority, Some(10));
        assert!(hpc.match_pattern.is_none());

        let hrm = m.hooks.get("render_markdown").unwrap();
        assert_eq!(hrm.priority, Some(20));

        let hhr = m.hooks.get("handle_route").unwrap();
        assert_eq!(hhr.priority, Some(5));
        assert_eq!(hhr.match_pattern.as_deref(), Some("/api/v1/plugins/seo/*"));
    }

    #[test]
    fn parse_manifest_missing_required_field() {
        let toml = r#"
[plugin]
name = "Missing ID"
version = "1.0.0"
"#;
        assert!(toml::from_str::<PluginManifest>(toml).is_err());
    }

    #[test]
    fn hookpoint_wasm_func_name_roundtrip() {
        for hp in HookPoint::all() {
            let name = hp.wasm_func_name();
            assert!(!name.is_empty(), "HookPoint::{hp:?} has empty func name");
            assert!(
                name.contains('_')
                    || name == "render_markdown"
                    || name == "filter_html"
                    || name == "handle_route"
            );
        }
    }

    #[test]
    fn hookpoint_all_has_11_variants() {
        assert_eq!(HookPoint::all().len(), 11);
    }

    #[test]
    fn permissions_default_is_empty() {
        let p = Permissions::default();
        assert!(p.http.is_empty());
        assert!(p.config.is_empty());
        assert!(p.max_memory_mb.is_none());
        assert!(p.timeout_ms.is_none());
    }
}
