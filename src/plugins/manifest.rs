//! 插件清单 (plugin.toml) 解析

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 插件清单顶层结构
#[derive(Debug, Clone, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default, deserialize_with = "deserialize_hooks")]
    pub hooks: HashMap<String, HookConfig>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    #[serde(default)]
    pub cron: Vec<CronEntry>,
    /// 插件声明的内容类型文件（Phase 10 新增）
    #[serde(default)]
    pub content_types: Vec<ContentTypeRef>,
    /// 插件注册的自定义路由（Phase 10 新增）
    #[serde(default)]
    pub routes: Vec<RouteDef>,
    /// 插件注册的 Admin 页面（Phase 10 新增）
    #[serde(default)]
    pub admin_pages: Vec<AdminPageDef>,
}

/// 内容类型引用
#[derive(Debug, Clone, Deserialize)]
pub struct ContentTypeRef {
    pub file: String,
}

/// 路由定义
#[derive(Debug, Clone, Deserialize)]
pub struct RouteDef {
    pub method: String,
    pub path: String,
    pub handler: String,
    #[serde(default)]
    pub auth: crate::content_type::schema::ApiAccess,
    pub permission: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input: Vec<RouteParam>,
    #[serde(default)]
    pub output: RouteOutput,
}

/// 路由参数定义
#[derive(Debug, Clone, Deserialize)]
pub struct RouteParam {
    pub name: String,
    #[serde(default = "default_query")]
    pub r#in: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub default: Option<toml::Value>,
}

fn default_query() -> String {
    "query".into()
}

/// 路由输出定义
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RouteOutput {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub fields: Vec<RouteOutputField>,
}

/// 输出字段定义
#[derive(Debug, Clone, Deserialize)]
pub struct RouteOutputField {
    pub name: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Admin 页面定义
#[derive(Debug, Clone, Deserialize)]
pub struct AdminPageDef {
    pub path: String,
    pub label: String,
    pub icon: Option<String>,
    pub component: Option<String>,
}

/// 插件声明的 Cron 定时任务条目
#[derive(Debug, Clone, Deserialize)]
pub struct CronEntry {
    /// 可读标签
    pub label: String,
    /// 自定义 `job_type` 字符串（任意值，不要求匹配内置枚举）
    pub job_type: String,
    /// JSON payload（可选）
    #[serde(default)]
    pub payload: Option<String>,
    /// 七段式 Cron 表达式（含秒）
    pub cron_expr: String,
    /// 是否启用（默认 true）
    #[serde(default = "cron_default_true")]
    pub enabled: bool,
}

fn cron_default_true() -> bool {
    true
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
    /// SDK 版本（默认 "v1"）
    #[serde(default = "default_sdk_version")]
    pub sdk_version: String,
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

fn default_sdk_version() -> String {
    "v1".into()
}

/// 插件权限声明
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub http: Vec<String>,
    #[serde(default)]
    pub config: Vec<String>,
    #[serde(default)]
    pub database: Vec<String>,
    #[serde(default)]
    pub filesystem: Vec<String>,
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
    // ── 内容生命周期（旧，兼容） ──
    PostCreating,
    PostCreated,
    PostUpdating,
    PostUpdated,
    PostDeleted,
    CommentCreating,
    CommentCreated,

    // ── 通用 CMS 事件（Phase 10 新增） ──
    ContentCreating,
    ContentCreated,
    ContentUpdating,
    ContentUpdated,
    ContentDeleted,

    // ── 内容访问 ──
    ContentViewed,

    // ── 字段级 ──
    RenderMarkdown,
    FilterHtml,

    // ── 路由/认证 ──
    OnLogin,

    // ── 定时任务 ──
    CronTick,
}

impl HookPoint {
    /// 返回对应的 WASM 导出函数名
    #[must_use]
    pub fn wasm_func_name(self) -> &'static str {
        match self {
            HookPoint::PostCreating => "on_post_creating",
            HookPoint::PostCreated => "on_post_created",
            HookPoint::PostUpdating => "on_post_updating",
            HookPoint::PostUpdated => "on_post_updated",
            HookPoint::PostDeleted => "on_post_deleted",
            HookPoint::CommentCreating => "on_comment_creating",
            HookPoint::CommentCreated => "on_comment_created",
            HookPoint::ContentCreating => "on_content_creating",
            HookPoint::ContentCreated => "on_content_created",
            HookPoint::ContentUpdating => "on_content_updating",
            HookPoint::ContentUpdated => "on_content_updated",
            HookPoint::ContentDeleted => "on_content_deleted",
            HookPoint::ContentViewed => "on_content_viewed",
            HookPoint::RenderMarkdown => "render_markdown",
            HookPoint::FilterHtml => "filter_html",
            HookPoint::OnLogin => "on_login",
            HookPoint::CronTick => "on_cron_tick",
        }
    }

    /// 返回所有 Hook 点，用于遍历测试
    #[must_use]
    pub fn all() -> &'static [HookPoint] {
        &[
            HookPoint::PostCreating,
            HookPoint::PostCreated,
            HookPoint::PostUpdating,
            HookPoint::PostUpdated,
            HookPoint::PostDeleted,
            HookPoint::CommentCreating,
            HookPoint::CommentCreated,
            HookPoint::ContentCreating,
            HookPoint::ContentCreated,
            HookPoint::ContentUpdating,
            HookPoint::ContentUpdated,
            HookPoint::ContentDeleted,
            HookPoint::ContentViewed,
            HookPoint::RenderMarkdown,
            HookPoint::FilterHtml,
            HookPoint::OnLogin,
            HookPoint::CronTick,
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

[hooks.on-login]
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

        let hol = m.hooks.get("on_login").unwrap();
        assert_eq!(hol.priority, Some(5));
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
            assert!(name.contains('_') || name == "render_markdown" || name == "filter_html");
        }
    }

    #[test]
    fn hookpoint_all_has_17_variants() {
        assert_eq!(HookPoint::all().len(), 17);
    }

    #[test]
    fn permissions_default_is_empty() {
        let p = Permissions::default();
        assert!(p.http.is_empty());
        assert!(p.config.is_empty());
        assert!(p.database.is_empty());
        assert!(p.max_memory_mb.is_none());
        assert!(p.timeout_ms.is_none());
    }

    #[test]
    fn parse_manifest_with_database_permissions() {
        let toml = r#"
[plugin]
id = "com.example.analytics"
name = "Analytics"
version = "1.0.0"

[permissions]
http = ["api.analytics.com/*"]
config = ["seo.*"]
database = ["read:posts", "read:comments", "write:analytics"]
max_memory_mb = 64
timeout_ms = 3000
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(
            m.permissions.database,
            vec!["read:posts", "read:comments", "write:analytics"]
        );
        assert_eq!(m.permissions.http, vec!["api.analytics.com/*"]);
        assert_eq!(m.permissions.config, vec!["seo.*"]);
    }

    #[test]
    fn parse_manifest_database_defaults_empty() {
        let toml = r#"
[plugin]
id = "com.example.basic"
name = "Basic"
version = "1.0.0"
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert!(m.permissions.database.is_empty());
        assert!(m.permissions.http.is_empty());
        assert!(m.permissions.config.is_empty());
    }

    #[test]
    fn parse_manifest_with_filesystem_permissions() {
        let toml = r#"
[plugin]
id = "com.example.cache"
name = "Cache"
version = "1.0.0"

[permissions]
filesystem = ["read-write"]
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.permissions.filesystem, vec!["read-write"]);
    }

    #[test]
    fn parse_manifest_filesystem_defaults_empty() {
        let toml = r#"
[plugin]
id = "com.example.basic"
name = "Basic"
version = "1.0.0"
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert!(m.permissions.filesystem.is_empty());
    }

    #[test]
    fn parse_manifest_filesystem_wildcard() {
        let toml = r#"
[plugin]
id = "com.example.admin"
name = "Admin"
version = "1.0.0"

[permissions]
filesystem = ["*"]
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.permissions.filesystem, vec!["*"]);
    }

    #[test]
    fn parse_manifest_with_cron_entries() {
        let toml = r#"
[plugin]
id = "com.example.cleanup"
name = "Cleanup"
version = "1.0.0"

[[cron]]
label = "Cleanup Sessions"
job_type = "cleanup_sessions"
payload = '{"max_age_hours": 24}'
cron_expr = "0 0 */6 * * *"

[[cron]]
label = "Send Digest"
job_type = "send_digest"
cron_expr = "0 0 3 * * *"
enabled = false
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.cron.len(), 2);

        assert_eq!(m.cron[0].label, "Cleanup Sessions");
        assert_eq!(m.cron[0].job_type, "cleanup_sessions");
        assert_eq!(
            m.cron[0].payload,
            Some(r#"{"max_age_hours": 24}"#.to_string())
        );
        assert_eq!(m.cron[0].cron_expr, "0 0 */6 * * *");
        assert!(m.cron[0].enabled);

        assert_eq!(m.cron[1].label, "Send Digest");
        assert_eq!(m.cron[1].job_type, "send_digest");
        assert!(m.cron[1].payload.is_none());
        assert_eq!(m.cron[1].cron_expr, "0 0 3 * * *");
        assert!(!m.cron[1].enabled);
    }

    #[test]
    fn parse_manifest_cron_defaults_empty() {
        let toml = r#"
[plugin]
id = "com.example.basic"
name = "Basic"
version = "1.0.0"
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert!(m.cron.is_empty());
    }

    #[test]
    fn parse_manifest_with_route_params() {
        let toml = r#"
[plugin]
id = "com.example.ecommerce"
name = "E-Commerce"
version = "0.1.0"

[[routes]]
method = "GET"
path = "/api/v1/plugins/ecommerce/products"
handler = "listProducts"
description = "获取商品列表"

[[routes.input]]
name = "page"
in = "query"
type = "integer"
description = "页码"

[[routes.input]]
name = "page_size"
in = "query"
type = "integer"
description = "每页数量"
default = 20

[[routes.input]]
name = "category_id"
in = "query"
type = "string"
description = "按分类筛选"

[routes.output]
description = "商品分页列表"

[[routes.output.fields]]
name = "data"
type = "array"
description = "商品列表"

[[routes.output.fields]]
name = "total"
type = "integer"
description = "总数"

[[routes]]
method = "POST"
path = "/api/v1/plugins/ecommerce/cart"
handler = "addToCart"
description = "添加到购物车"

[[routes.input]]
name = "product_id"
in = "body"
type = "string"
required = true
description = "商品ID"

[[routes.input]]
name = "quantity"
in = "body"
type = "integer"
required = true
description = "数量"
default = 1
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(m.routes.len(), 2);

        let r0 = &m.routes[0];
        assert_eq!(r0.method, "GET");
        assert_eq!(r0.input.len(), 3);
        assert_eq!(r0.input[0].name, "page");
        assert_eq!(r0.input[0].r#in, "query");
        assert!(!r0.input[0].required);
        assert_eq!(r0.output.fields.len(), 2);

        let r1 = &m.routes[1];
        assert_eq!(r1.method, "POST");
        assert_eq!(r1.input.len(), 2);
        assert!(r1.input[0].required);
        assert_eq!(r1.input[1].default, Some(toml::Value::Integer(1)));
    }
}
