//! Rhai engine wrapper (per-request mode)
//!
//! Based on Rhai, a pure Rust scripting language with zero C dependencies, compiles to wasm32 without issues.
//! Native sandboxing (limited loop count, expression depth), cold start <1ms.
//!
//! Uses scheme D (per-request): a fresh Scope + Engine is created for each call and destroyed after use.
//! Rhai AST is pre-compiled and reused; Scope creation cost is minimal, with zero contention, unlimited concurrency, and perfect isolation.
//!
//! Plugin convention: scripts register a global `Plugin` map with hook function names as keys and closures as values.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dashmap::DashMap;
use rhai::{AST, Engine, Scope};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

struct RhaiPluginEntry {
    ast: AST,
    permissions: Permissions,
    #[allow(dead_code)]
    plugin_dir: PathBuf,
}

/// Rhai plugin engine (per-request mode)
///
/// Stores pre-compiled ASTs for all Rhai plugins; creates a fresh Engine + Scope for each call.
/// Timeout is implemented via `Engine::on_progress`.
pub struct RhaiEngine {
    plugins: DashMap<String, RhaiPluginEntry>,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
}

impl RhaiEngine {
    pub fn new(
        config: &AppConfig,
        pool: Option<Pool>,
        event_bus: Option<crate::eventbus::EventBus>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            plugins: DashMap::new(),
            config: Arc::new(config.clone()),
            pool,
            event_bus,
        })
    }

    fn build_engine(timeout_ms: u64) -> Engine {
        let mut engine = Engine::new();

        engine.set_allow_looping(true);
        engine.set_max_expr_depths(64, 32);
        engine.set_max_modules(0);
        engine.set_max_string_size(1024 * 1024);
        engine.set_max_array_size(10_000);
        engine.set_max_map_size(10_000);

        let max_ops = if timeout_ms > 0 { 1_000_000 } else { u64::MAX };
        engine.on_progress(move |ops| {
            if ops > max_ops {
                return Some("execution timeout".into());
            }
            None
        });

        engine
    }

    fn create_base_engine(&self) -> Engine {
        let timeout_ms = self.config.plugin_default_timeout_ms;
        Self::build_engine(timeout_ms)
    }

    fn create_instance(&self, entry: &RhaiPluginEntry, plugin_id: &str) -> Engine {
        let mut engine = self.create_base_engine();
        super::rhai_host::register_host_functions(
            &mut engine,
            self.config.clone(),
            plugin_id.to_string(),
            entry.permissions.clone(),
            self.pool.clone(),
            self.event_bus.clone(),
        );
        engine
    }

    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
        plugin_dir: &Path,
        _sdk_source: &'static str,
    ) -> anyhow::Result<()> {
        let mut engine = self.create_base_engine();
        super::rhai_host::register_host_functions(
            &mut engine,
            self.config.clone(),
            id.to_string(),
            permissions.clone(),
            self.pool.clone(),
            self.event_bus.clone(),
        );

        let ast = engine
            .compile(code)
            .map_err(|e| anyhow::anyhow!("rhai compile error: {e}"))?;

        let mut scope = Scope::new();
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| anyhow::anyhow!("rhai init error: {e}"))?;

        self.plugins.insert(
            id.to_string(),
            RhaiPluginEntry {
                ast,
                permissions,
                plugin_dir: plugin_dir.to_path_buf(),
            },
        );
        Ok(())
    }

    #[cfg(test)]
    pub async fn load_plugin_default(&self, id: &str, code: &str) -> anyhow::Result<()> {
        self.load_plugin(id, code, Permissions::default(), Path::new("."), "")
            .await
    }

    pub async fn unload_plugin(&self, id: &str) {
        self.plugins.remove(id);
    }

    pub async fn call_filter<T: Serialize + DeserializeOwned + Send>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let Some(entry) = self.plugins.get(plugin_id) else {
            return Ok(None);
        };

        let engine = self.create_instance(&entry, plugin_id);
        let input_dynamic = rhai::serde::to_dynamic(input)?;

        let mut scope = Scope::new();
        let result: rhai::Dynamic =
            match engine.call_fn(&mut scope, &entry.ast, func_name, (input_dynamic,)) {
                Ok(r) => r,
                Err(e) => match *e {
                    rhai::EvalAltResult::ErrorFunctionNotFound(_, _) => return Ok(None),
                    _ => return Err(anyhow::anyhow!("rhai call_filter error: {e}")),
                },
            };

        if result.is::<()>() {
            return Ok(None);
        }

        let output: T = rhai::serde::from_dynamic(&result)?;
        Ok(Some(output))
    }

    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let Some(entry) = self.plugins.get(plugin_id) else {
            return Ok(());
        };

        let engine = self.create_instance(&entry, plugin_id);
        let data_dynamic = rhai::serde::to_dynamic(data)?;

        let mut scope = Scope::new();
        match engine.call_fn::<()>(&mut scope, &entry.ast, func_name, (data_dynamic,)) {
            Ok(_) => Ok(()),
            Err(e) => match *e {
                rhai::EvalAltResult::ErrorFunctionNotFound(_, _) => Ok(()),
                _ => Err(anyhow::anyhow!("rhai call_action error: {e}")),
            },
        }
    }

    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let Some(entry) = self.plugins.get(plugin_id) else {
            return Ok(None);
        };

        let engine = self.create_instance(&entry, plugin_id);

        let mut scope = Scope::new();
        let result: String =
            match engine.call_fn(&mut scope, &entry.ast, func_name, (input.to_string(),)) {
                Ok(r) => r,
                Err(e) => match *e {
                    rhai::EvalAltResult::ErrorFunctionNotFound(_, _) => return Ok(None),
                    _ => return Err(anyhow::anyhow!("rhai call_string_filter error: {e}")),
                },
            };

        Ok(Some(result))
    }

    #[allow(dead_code)]
    pub async fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;
    use std::sync::Arc;

    fn test_config() -> Arc<AppConfig> {
        let mut config = AppConfig::test_defaults();
        config.plugin_max_memory_mb = 8;
        config.plugin_default_timeout_ms = 2000;
        Arc::new(config)
    }

    #[tokio::test]
    async fn rhai_engine_create() {
        let engine = RhaiEngine::new(&test_config(), None, None);
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn rhai_engine_load_and_call_filter() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn on_post_creating(input) {
    input.title = to_upper(input.title);
    input
}
"#;
        engine
            .load_plugin_default("test-filter", code)
            .await
            .unwrap();

        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: Option<serde_json::Value> = engine
            .call_filter("test-filter", "on_post_creating", &input)
            .await
            .unwrap();

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result["title"], "HELLO");
        assert_eq!(result["content"], "world");
    }

    #[tokio::test]
    async fn rhai_engine_call_filter_missing_plugin() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn rhai_engine_call_filter_missing_function() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = "fn noop() { 42 }";
        engine
            .load_plugin_default("test-nofunc", code)
            .await
            .unwrap();

        let result: Option<serde_json::Value> = engine
            .call_filter("test-nofunc", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn rhai_engine_call_action() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn on_post_created(data) {
    log("info", "post created");
}
"#;
        engine
            .load_plugin_default("test-action", code)
            .await
            .unwrap();

        let result = engine
            .call_action(
                "test-action",
                "on_post_created",
                &serde_json::json!({"id": "123"}),
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn rhai_engine_call_string_filter() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn filter_html(html) {
    replace(html, "<head>", `<head><meta property="og:type" content="article">`)
}
"#;
        engine
            .load_plugin_default("test-strfilter", code)
            .await
            .unwrap();

        let result = engine
            .call_string_filter(
                "test-strfilter",
                "filter_html",
                "<head><title>Test</title></head>",
            )
            .await
            .unwrap();

        assert!(result.is_some());
        assert!(result.unwrap().contains("og:type"));
    }

    #[tokio::test]
    async fn rhai_engine_unload_plugin() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = "fn noop() { 42 }";
        engine
            .load_plugin_default("test-unload", code)
            .await
            .unwrap();
        assert_eq!(engine.plugin_count().await, 1);

        engine.unload_plugin("test-unload").await;
        assert_eq!(engine.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn rhai_engine_multiple_plugins() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        for i in 0..3 {
            let code = format!(r#"fn on_post_creating(m) {{ m.idx = {i}; m }}"#);
            engine
                .load_plugin_default(&format!("plugin-{i}"), &code)
                .await
                .unwrap();
        }

        assert_eq!(engine.plugin_count().await, 3);
    }

    #[tokio::test]
    async fn rhai_engine_syntax_error_fails_load() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();
        let result = engine
            .load_plugin_default("test-bad-syntax", "let !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rhai_per_request_state_isolation() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn on_post_creating(input) {
    input.counter = 1;
    input
}
"#;
        engine
            .load_plugin_default("test-isolation", code)
            .await
            .unwrap();

        let r1: Option<serde_json::Value> = engine
            .call_filter("test-isolation", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(r1.as_ref().unwrap()["counter"], 1);

        let r2: Option<serde_json::Value> = engine
            .call_filter("test-isolation", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(
            r2.as_ref().unwrap()["counter"],
            1,
            "per-request: each call should produce identical result (isolated)"
        );
    }

    #[tokio::test]
    async fn rhai_concurrent_calls_succeed() {
        let engine = Arc::new(RhaiEngine::new(&test_config(), None, None).unwrap());

        let code = r#"
fn on_post_creating(input) {
    input.processed = true;
    input
}
"#;
        engine
            .load_plugin_default("test-concurrent", code)
            .await
            .unwrap();

        let mut handles = Vec::new();
        for i in 0..10 {
            let eng = Arc::clone(&engine);
            handles.push(tokio::spawn(async move {
                let input = serde_json::json!({"idx": i});
                eng.call_filter::<serde_json::Value>("test-concurrent", "on_post_creating", &input)
                    .await
            }));
        }

        let mut success = 0;
        for h in handles {
            let r = h.await.unwrap().unwrap();
            if r.is_some() && r.as_ref().unwrap()["processed"] == true {
                success += 1;
            }
        }
        assert_eq!(success, 10, "all 10 concurrent calls should succeed");
    }

    #[tokio::test]
    async fn rhai_call_after_unload_returns_none() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();
        engine
            .load_plugin_default("test-gone", r#"fn on_post_creating(m) { m }"#)
            .await
            .unwrap();

        engine.unload_plugin("test-gone").await;

        let result: Option<serde_json::Value> = engine
            .call_filter("test-gone", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none(), "call after unload should return None");

        let result = engine
            .call_action("test-gone", "on_post_creating", &serde_json::json!({}))
            .await;
        assert!(
            result.is_ok(),
            "call_action after unload should return Ok(())"
        );

        let result = engine
            .call_string_filter("test-gone", "on_post_creating", "hello")
            .await
            .unwrap();
        assert!(
            result.is_none(),
            "call_string_filter after unload should return None"
        );
    }

    #[tokio::test]
    async fn rhai_engine_reload_same_plugin() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code_v1 = r#"
fn on_post_creating(input) {
    input.version = 1;
    input
}
"#;
        engine
            .load_plugin_default("test-reload", code_v1)
            .await
            .unwrap();

        let r1: Option<serde_json::Value> = engine
            .call_filter("test-reload", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(r1.as_ref().unwrap()["version"], 1);

        let code_v2 = r#"
fn on_post_creating(input) {
    input.version = 2;
    input
}
"#;
        engine
            .load_plugin_default("test-reload", code_v2)
            .await
            .unwrap();

        let r2: Option<serde_json::Value> = engine
            .call_filter("test-reload", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(r2.as_ref().unwrap()["version"], 2);
    }

    #[tokio::test]
    async fn rhai_engine_filter_modifies_multiple_fields() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn on_post_creating(input) {
    input.title = to_upper(input.title);
    input.slug = replace(to_lower(input.title), " ", "-");
    input.processed = true;
    remove(input, "removable");
    input
}
"#;
        engine
            .load_plugin_default("test-multi-field", code)
            .await
            .unwrap();

        let input = serde_json::json!({
            "title": "Hello World",
            "slug": "",
            "processed": false,
            "removable": "yes"
        });
        let result: Option<serde_json::Value> = engine
            .call_filter("test-multi-field", "on_post_creating", &input)
            .await
            .unwrap();

        let r = result.unwrap();
        assert_eq!(r["title"], "HELLO WORLD");
        assert_eq!(r["slug"], "hello-world");
        assert_eq!(r["processed"], true);
        assert!(
            r.get("removable").is_none(),
            "removable field should be removed"
        );
    }

    #[tokio::test]
    async fn rhai_engine_filter_exception_does_not_crash() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn on_post_creating(input) {
    throw "filter error";
}
"#;
        engine
            .load_plugin_default("test-filter-throw", code)
            .await
            .unwrap();

        let result: anyhow::Result<Option<serde_json::Value>> = engine
            .call_filter(
                "test-filter-throw",
                "on_post_creating",
                &serde_json::json!({}),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rhai_engine_string_filter_returns_empty_string() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
fn filter_html(html) {
    ""
}
"#;
        engine
            .load_plugin_default("test-empty-str", code)
            .await
            .unwrap();

        let result = engine
            .call_string_filter("test-empty-str", "filter_html", "<html></html>")
            .await
            .unwrap();
        assert_eq!(result.as_deref(), Some(""));
    }

    #[tokio::test]
    async fn rhai_engine_filter_chain_multiple_plugins() {
        let engine = RhaiEngine::new(&test_config(), None, None).unwrap();

        let code_a = r#"
fn on_post_creating(input) {
    input.tags = ["a"];
    input
}
"#;
        let code_b = r#"
fn on_post_creating(input) {
    input.tags = ["a", "b"];
    input
}
"#;
        engine.load_plugin_default("chain-a", code_a).await.unwrap();
        engine.load_plugin_default("chain-b", code_b).await.unwrap();

        let input = serde_json::json!({"title": "test"});
        let result_a: Option<serde_json::Value> = engine
            .call_filter("chain-a", "on_post_creating", &input)
            .await
            .unwrap();
        assert!(result_a.is_some());
        let result_a = result_a.unwrap();
        assert_eq!(result_a["tags"], serde_json::json!(["a"]));

        let result_b: Option<serde_json::Value> = engine
            .call_filter("chain-b", "on_post_creating", &result_a)
            .await
            .unwrap();
        assert!(result_b.is_some());
        assert_eq!(result_b.unwrap()["tags"], serde_json::json!(["a", "b"]));
    }
}
