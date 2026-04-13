//! 插件清单 (plugin.toml) 解析

use serde::Deserialize;
use std::collections::HashMap;

/// 插件清单顶层结构
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub hooks: HashMap<String, HookConfig>,
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
}

fn default_runtime() -> String {
    "wasm".into()
}

fn default_language() -> String {
    "rust".into()
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
}
