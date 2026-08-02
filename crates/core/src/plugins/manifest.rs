//! Plugin manifest (plugin.toml) parsing

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(feature = "export-types")]
use ts_rs::TS;

/// Plugin manifest top-level structure
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
    /// Content type files declared by the plugin (added in Phase 10)
    #[serde(default)]
    pub content_types: Vec<ContentTypeRef>,
    /// Custom routes registered by the plugin (added in Phase 10)
    #[serde(default)]
    pub routes: Vec<RouteDef>,
    /// Admin pages registered by the plugin (added in Phase 10)
    #[serde(default)]
    pub admin_pages: Vec<AdminPageDef>,
}

/// Content type reference
#[derive(Debug, Clone, Deserialize)]
pub struct ContentTypeRef {
    pub file: String,
}

/// Route definition
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

/// Route parameter definition
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

/// Route output definition
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RouteOutput {
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub fields: Vec<RouteOutputField>,
}

/// Output field definition
#[derive(Debug, Clone, Deserialize)]
pub struct RouteOutputField {
    pub name: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// Admin page definition
#[derive(Debug, Clone, Deserialize)]
pub struct AdminPageDef {
    pub path: String,
    pub label: String,
    pub icon: Option<String>,
    pub component: Option<String>,
}

/// Cron scheduled task entry declared by the plugin
#[derive(Debug, Clone, Deserialize)]
pub struct CronEntry {
    /// Human-readable label
    pub label: String,
    /// Custom `job_type` string (any value, not required to match built-in enums)
    pub job_type: String,
    /// JSON payload (optional)
    #[serde(default)]
    pub payload: Option<String>,
    /// Seven-segment cron expression (including seconds)
    pub cron_expr: String,
    /// Whether enabled (default true)
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

/// Plugin basic information
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
    /// SDK version (default "v1")
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

/// Plugin permission declaration
#[cfg_attr(feature = "export-types", derive(TS))]
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

/// Hook registration configuration
#[derive(Debug, Clone, Deserialize)]
pub struct HookConfig {
    pub priority: Option<i32>,
    #[serde(rename = "match")]
    pub match_pattern: Option<String>,
    /// Only receive events for the specified content_type (empty or omitted = receive all)
    #[serde(default)]
    pub content_types: Vec<String>,
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
description = "Get product list"

[[routes.input]]
name = "page"
in = "query"
type = "integer"
description = "Page number"

[[routes.input]]
name = "page_size"
in = "query"
type = "integer"
description = "Items per page"
default = 20

[[routes.input]]
name = "category_id"
in = "query"
type = "string"
description = "Filter by category"

[routes.output]
description = "Paginated product list"

[[routes.output.fields]]
name = "data"
type = "array"
description = "Product list"

[[routes.output.fields]]
name = "total"
type = "integer"
description = "Total count"

[[routes]]
method = "POST"
path = "/api/v1/plugins/ecommerce/cart"
handler = "addToCart"
description = "Add to cart"

[[routes.input]]
name = "product_id"
in = "body"
type = "string"
required = true
description = "Product ID"

[[routes.input]]
name = "quantity"
in = "body"
type = "integer"
required = true
description = "Quantity"
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

    #[test]
    fn parse_manifest_hook_with_content_types() {
        let toml = r#"
[plugin]
id = "com.example.articles"
name = "Article Plugin"
version = "1.0.0"

[hooks.on-content-creating]
priority = 50
content_types = ["articles", "blog-posts"]

[hooks.on-content-created]
priority = 30

[hooks.render-markdown]
priority = 10
content_types = ["articles"]
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();

        let creating = m.hooks.get("on_content_creating").unwrap();
        assert_eq!(creating.priority, Some(50));
        assert_eq!(creating.content_types, vec!["articles", "blog-posts"]);

        let created = m.hooks.get("on_content_created").unwrap();
        assert_eq!(created.priority, Some(30));
        assert!(
            created.content_types.is_empty(),
            "no content_types = receives all"
        );

        let render = m.hooks.get("render_markdown").unwrap();
        assert_eq!(render.content_types, vec!["articles"]);
    }

    #[test]
    fn parse_manifest_hook_content_types_default_empty() {
        let toml = r#"
[plugin]
id = "com.example.test"
name = "Test"
version = "1.0.0"

[hooks.on-post-creating]
priority = 10
"#;
        let m: PluginManifest = toml::from_str(toml).unwrap();
        let hook = m.hooks.get("on_post_creating").unwrap();
        assert!(hook.content_types.is_empty());
    }
}
