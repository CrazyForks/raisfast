//! QuickJS engine wrapper (per-request mode)
//!
//! Based on rquickjs `AsyncRuntime` / `AsyncContext`,
//! supporting JavaScript plugins running in the tokio async environment.
//!
//! Uses scheme D (per-request): a fresh QuickJS context is created for each call and destroyed after use.
//! JS context creation cost is ~1ms, with zero contention, unlimited concurrency, and perfect isolation.
//!
//! ESM mode: plugins use `import/export` syntax;
//! the framework collects exported functions from the module namespace and registers them to the Plugin object.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use rquickjs::loader::Resolver;
use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Function, Module, Object, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

struct JsPluginEntry {
    code: String,
    permissions: Permissions,
    plugin_dir: PathBuf,
    sdk_source: &'static str,
}

// ── ESM Module Loader ────────────────────────────────────────────

struct PluginResolver;

impl Resolver for PluginResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        _base: &str,
        name: &str,
    ) -> rquickjs::Result<String> {
        Ok(name.to_string())
    }
}

struct PluginLoader {
    plugin_dir: PathBuf,
    sdk_source: &'static str,
}

impl PluginLoader {
    fn new(plugin_dir: PathBuf, sdk_source: &'static str) -> Self {
        Self {
            plugin_dir,
            sdk_source,
        }
    }
}

impl rquickjs::loader::Loader for PluginLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
        let source = match name {
            "sdk" => self.sdk_source.to_string(),
            n if n.starts_with("./") || n.starts_with("../") => {
                let path = self.plugin_dir.join(n);
                let canonical = path.canonicalize().map_err(|e| {
                    rquickjs::Error::new_loading_message(name, &format!("path error: {e}"))
                })?;
                let plugin_canonical = self
                    .plugin_dir
                    .canonicalize()
                    .unwrap_or_else(|_| self.plugin_dir.clone());
                if !canonical.starts_with(&plugin_canonical) {
                    return Err(rquickjs::Error::new_loading_message(
                        name,
                        "path traversal denied",
                    ));
                }
                std::fs::read_to_string(&canonical).map_err(|e| {
                    rquickjs::Error::new_loading_message(name, &format!("read error: {e}"))
                })?
            }
            _ => {
                return Err(rquickjs::Error::new_loading_message(name, "unknown module"));
            }
        };
        Module::declare(ctx.clone(), name, source)
    }
}

// ── JS Engine ────────────────────────────────────────────────────

/// QuickJS plugin engine (per-request mode)
///
/// Stores source code and metadata for all JS plugins; creates a fresh context for each call.
/// Timeout is implemented via interrupt_handler within the synchronous `ctx.with()`,
/// with `tokio::time::timeout` in the outer `plugins.rs` as a fallback.
pub struct JsEngine {
    plugins: DashMap<String, JsPluginEntry>,
    default_memory_limit_bytes: usize,
    timeout_ms: u64,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
}

impl JsEngine {
    pub async fn new(
        config: &AppConfig,
        pool: Option<Pool>,
        event_bus: Option<crate::eventbus::EventBus>,
    ) -> anyhow::Result<Self> {
        let default_memory_limit_bytes = (config.plugin_max_memory_mb as usize) * 1024 * 1024;

        Ok(Self {
            plugins: DashMap::new(),
            default_memory_limit_bytes,
            timeout_ms: config.plugin_default_timeout_ms,
            config: Arc::new(config.clone()),
            pool,
            event_bus,
        })
    }

    async fn create_instance(
        &self,
        entry: &JsPluginEntry,
        plugin_id: &str,
    ) -> anyhow::Result<(AsyncRuntime, AsyncContext)> {
        let memory_limit = entry
            .permissions
            .max_memory_mb
            .map_or(self.default_memory_limit_bytes, |mb| {
                mb as usize * 1024 * 1024
            });

        let runtime = AsyncRuntime::new()?;
        runtime.set_memory_limit(memory_limit).await;
        runtime.set_max_stack_size(512 * 1024).await;

        runtime
            .set_loader(
                PluginResolver,
                PluginLoader::new(entry.plugin_dir.clone(), entry.sdk_source),
            )
            .await;

        let ctx = AsyncContext::full(&runtime).await?;
        let config = self.config.clone();
        let plugin_id_owned = plugin_id.to_string();
        let perms = entry.permissions.clone();
        ctx.with(|ctx| {
            super::js_host::register_host_functions(
                ctx.clone(),
                config,
                plugin_id_owned,
                perms,
                self.pool.clone(),
                self.event_bus.clone(),
            )?;

            let module = Module::declare(ctx.clone(), "index.js", entry.code.clone())?;
            let (evaled, _promise) = module.eval()?;
            _promise.finish::<()>()?;

            let ns = evaled.namespace()?;
            let global = ctx.globals();
            let plugin_obj = Object::new(ctx.clone())?;

            for key_result in ns.keys::<String>() {
                let key = key_result?;
                let Ok(func) = ns.get::<_, Function>(&key) else {
                    continue;
                };
                plugin_obj.set(&key, func)?;
            }

            global.set("Plugin", plugin_obj)?;

            Ok::<_, rquickjs::Error>(())
        })
        .await?;

        Ok((runtime, ctx))
    }

    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
        plugin_dir: &Path,
        sdk_source: &'static str,
    ) -> anyhow::Result<()> {
        let entry = JsPluginEntry {
            code: code.to_string(),
            permissions: permissions.clone(),
            plugin_dir: plugin_dir.to_path_buf(),
            sdk_source,
        };

        let (runtime, ctx) = self.create_instance(&entry, id).await?;
        runtime.run_gc().await;
        drop(ctx);
        drop(runtime);

        self.plugins.insert(id.to_string(), entry);
        Ok(())
    }

    #[cfg(test)]
    pub async fn load_plugin_default(&self, id: &str, code: &str) -> anyhow::Result<()> {
        self.load_plugin(
            id,
            code,
            Permissions::default(),
            Path::new("."),
            crate::plugins::sdk_v1::JS_SDK_V1,
        )
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

        let (runtime, ctx) = self.create_instance(&entry, plugin_id).await?;
        let input_json = serde_json::to_string(input)?;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let result: anyhow::Result<Option<T>> = ctx
            .with(|ctx| {
                let global = ctx.globals();
                let plugin_obj: Object = match global.get("Plugin") {
                    Ok(obj) => obj,
                    Err(_) => return Ok(None),
                };
                let func: Function = match plugin_obj.get(func_name_owned.as_str()) {
                    Ok(f) => f,
                    Err(_) => return Ok(None),
                };
                let result_value: Value = func.call((input_json,))?;
                let result_str = ctx
                    .json_stringify(&result_value)
                    .map_err(|e| anyhow::anyhow!("json stringify error: {e}"))?
                    .ok_or_else(|| anyhow::anyhow!("json stringify returned undefined"))?;
                let output: T = serde_json::from_str(&result_str.to_string()?)?;
                Ok(Some(output))
            })
            .await;

        drop(ctx);
        runtime.run_gc().await;
        drop(runtime);
        result
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

        let (runtime, ctx) = self.create_instance(&entry, plugin_id).await?;
        let data_json = serde_json::to_string(data)?;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let result: anyhow::Result<()> = ctx
            .with(|ctx| {
                let global = ctx.globals();
                let plugin_obj: Object = match global.get("Plugin") {
                    Ok(obj) => obj,
                    Err(_) => return Ok(()),
                };
                let func: Function = match plugin_obj.get(func_name_owned.as_str()) {
                    Ok(f) => f,
                    Err(_) => return Ok(()),
                };
                let _: () = func.call((data_json,))?;
                Ok(())
            })
            .await;

        drop(ctx);
        runtime.run_gc().await;
        drop(runtime);
        result
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

        let (runtime, ctx) = self.create_instance(&entry, plugin_id).await?;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let input_owned = input.to_string();
        let result: anyhow::Result<Option<String>> = ctx
            .with(|ctx| {
                let global = ctx.globals();
                let plugin_obj: Object = match global.get("Plugin") {
                    Ok(obj) => obj,
                    Err(_) => return Ok(None),
                };
                let func: Function = match plugin_obj.get(func_name_owned.as_str()) {
                    Ok(f) => f,
                    Err(_) => return Ok(None),
                };
                let result_value: Value = func.call((input_owned,))?;
                let js_string = rquickjs::String::from_value(result_value)?;
                Ok(Some(js_string.to_string()?))
            })
            .await;

        drop(ctx);
        runtime.run_gc().await;
        drop(runtime);
        result
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
    async fn js_engine_create() {
        let engine = JsEngine::new(&test_config(), None, None).await;
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn js_engine_load_and_call_filter() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.title = input.title.toUpperCase();
    return input;
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
    async fn js_engine_call_filter_missing_plugin() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn js_engine_call_filter_missing_function() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"export const noop = 1;"#;
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
    async fn js_engine_call_action() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
import { logInfo } from 'sdk';

export function on_post_created(dataJson) {
    var data = JSON.parse(dataJson);
    logInfo("post created: " + data.id);
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
    async fn js_engine_call_string_filter() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function filter_html(html) {
    return html.replace("<head>", '<head><meta property="og:type" content="article">');
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
    async fn js_engine_unload_plugin() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"export const noop = 1;"#;
        engine
            .load_plugin_default("test-unload", code)
            .await
            .unwrap();
        assert_eq!(engine.plugin_count().await, 1);

        engine.unload_plugin("test-unload").await;
        assert_eq!(engine.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn js_engine_multiple_plugins() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        for i in 0..3 {
            let code = format!(
                r#"export function on_post_creating(j) {{ var d = JSON.parse(j); d.idx = {i}; return d; }}"#
            );
            engine
                .load_plugin_default(&format!("plugin-{i}"), &code)
                .await
                .unwrap();
        }

        assert_eq!(engine.plugin_count().await, 3);
    }

    #[tokio::test]
    async fn js_engine_host_log_available() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
import { logInfo } from 'sdk';

export function on_post_created(dataJson) {
    logInfo("test message");
}
"#;
        engine.load_plugin_default("test-host", code).await.unwrap();

        let result = engine
            .call_action("test-host", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn js_engine_host_get_config_returns_value() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
import { configGet } from 'sdk';

export function on_post_created(dataJson) {
    var env = configGet("app.env");
    if (env !== "test") {
        throw new Error("expected test, got: " + env);
    }
    var unknown = configGet("nonexistent.key");
    if (unknown != null) {
        throw new Error("expected null for unknown key");
    }
}
"#;
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };
        engine
            .load_plugin(
                "test-cfg",
                code,
                perms,
                Path::new("."),
                crate::plugins::sdk_v1::JS_SDK_V1,
            )
            .await
            .unwrap();

        let result = engine
            .call_action("test-cfg", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn js_engine_syntax_error_fails_load() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();
        let result = engine
            .load_plugin_default("test-bad-syntax", "var !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_engine_timeout_interrupts_long_execution() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = JsEngine::new(&Arc::new(config), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    var start = Date.now();
    while (Date.now() - start < 10000) {}
    return inputJson;
}
"#;
        engine
            .load_plugin_default("test-timeout", code)
            .await
            .unwrap();

        let result: anyhow::Result<Option<serde_json::Value>> = engine
            .call_filter("test-timeout", "on_post_creating", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_engine_filter_chain_multiple_plugins() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code_a = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.tags = ["a"];
    return input;
}
"#;
        let code_b = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.tags.push("b");
    return input;
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

    #[tokio::test]
    async fn js_engine_action_exception_does_not_crash() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_created(dataJson) {
    throw new Error("intentional error");
}
"#;
        engine
            .load_plugin_default("test-throw", code)
            .await
            .unwrap();

        let result = engine
            .call_action(
                "test-throw",
                "on_post_created",
                &serde_json::json!({"id": "1"}),
            )
            .await;
        assert!(result.is_err());
    }

    async fn eval_js_str(code: &str) -> Result<String, rquickjs::Error> {
        let rt = AsyncRuntime::new()?;
        let ctx = AsyncContext::full(&rt).await?;
        let result: String = ctx
            .with(|ctx| {
                let v: rquickjs::Value = ctx.eval(code)?;
                let s = v
                    .as_string()
                    .map(|s| s.to_string().unwrap_or_default())
                    .unwrap_or_else(|| format!("{v:?}"));
                Ok::<String, rquickjs::Error>(s)
            })
            .await?;
        Ok(result)
    }

    #[tokio::test]
    async fn qjs_let_const() {
        let r = eval_js_str("let x = 1; const y = 2; String(x + y)").await;
        assert!(r.is_ok(), "let/const should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "3");
    }

    #[tokio::test]
    async fn qjs_arrow_function() {
        let r = eval_js_str("var add = (a, b) => a + b; String(add(1, 2))").await;
        assert!(r.is_ok(), "arrow function should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "3");
    }

    #[tokio::test]
    async fn qjs_optional_chaining() {
        let r = eval_js_str("var obj = {a:{b:1}}; String(obj?.a?.b ?? 'no')").await;
        assert!(r.is_ok(), "optional chaining should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "1");
    }

    #[tokio::test]
    async fn qjs_nullish_coalescing() {
        let r = eval_js_str("null ?? 'default'").await;
        assert!(r.is_ok(), "nullish coalescing should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "default");
    }

    #[tokio::test]
    async fn qjs_template_literals() {
        let r = eval_js_str("var name = 'world'; `hello ${name}`").await;
        assert!(r.is_ok(), "template literals should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn qjs_for_of() {
        let r = eval_js_str("var s = ''; for (var x of [1,2,3]) { s += x; } s").await;
        assert!(r.is_ok(), "for...of should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "123");
    }

    #[tokio::test]
    async fn qjs_destructuring() {
        let r = eval_js_str("var {a, b} = {a:1, b:2}; String(a + b)").await;
        assert!(r.is_ok(), "destructuring should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "3");
    }

    #[tokio::test]
    async fn qjs_default_params() {
        let r =
            eval_js_str("function greet(name = 'world') { return 'hello ' + name; } greet()").await;
        assert!(r.is_ok(), "default params should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn qjs_spread() {
        let r = eval_js_str("var a = [1,2]; var b = [...a, 3]; String(b.length)").await;
        assert!(r.is_ok(), "spread should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "3");
    }

    #[tokio::test]
    async fn qjs_object_shorthand() {
        let r = eval_js_str("var x = 1; var obj = {x}; String(obj.x)").await;
        assert!(r.is_ok(), "object shorthand should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "1");
    }

    #[tokio::test]
    async fn qjs_class_syntax() {
        let r = eval_js_str(
            "class Foo { constructor(v) { this.v = v; } get_val() { return this.v; } } String(new Foo(42).get_val())"
        ).await;
        assert!(r.is_ok(), "class syntax should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "42");
    }

    #[tokio::test]
    async fn qjs_promise_async_await() {
        let r = eval_js_str("async function f() { return 42; } typeof f()").await;
        assert!(r.is_ok(), "async/await should work: {:?}", r.err());
    }

    #[tokio::test]
    async fn qjs_exponentiation() {
        let r = eval_js_str("String(2 ** 10)").await;
        assert!(r.is_ok(), "exponentiation should work: {:?}", r.err());
        assert_eq!(r.unwrap(), "1024");
    }

    #[tokio::test]
    async fn js_per_request_state_isolation() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
var counter = 0;
export function on_post_creating(inputJson) {
    counter++;
    var input = JSON.parse(inputJson);
    input.counter = counter;
    return input;
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
            "per-request: counter should reset to 1 on each call (isolated context)"
        );
    }

    #[tokio::test]
    async fn js_concurrent_calls_succeed() {
        let engine = Arc::new(JsEngine::new(&test_config(), None, None).await.unwrap());

        let code = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.processed = true;
    return input;
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
    async fn js_call_after_unload_returns_none() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();
        engine
            .load_plugin_default(
                "test-gone",
                "export function on_post_creating(j) { return j; }",
            )
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
    async fn js_engine_filter_returns_undefined() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    return undefined;
}
"#;
        engine
            .load_plugin_default("test-undefined", code)
            .await
            .unwrap();

        let result = engine
            .call_filter::<serde_json::Value>(
                "test-undefined",
                "on_post_creating",
                &serde_json::json!({"title": "hello"}),
            )
            .await;
        assert!(
            result.is_err(),
            "returning undefined should fail (json stringify undefined)"
        );
    }

    #[tokio::test]
    async fn js_engine_filter_exception_does_not_crash() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    throw new Error("filter error");
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
    async fn js_engine_string_filter_exception_does_not_crash() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function render_markdown(content) {
    throw new Error("string filter error");
}
"#;
        engine
            .load_plugin_default("test-strfilter-throw", code)
            .await
            .unwrap();

        let result = engine
            .call_string_filter("test-strfilter-throw", "render_markdown", "# hello")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_engine_string_filter_returns_empty_string() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function filter_html(html) {
    return "";
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
    async fn js_engine_filter_modifies_multiple_fields() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.title = input.title.toUpperCase();
    input.slug = input.title.toLowerCase().replace(/\s+/g, '-');
    input.processed = true;
    delete input.removable;
    return input;
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
            "removable field should be deleted"
        );
    }

    #[tokio::test]
    async fn js_engine_memory_limit_enforced() {
        let mut config = (*test_config()).clone();
        config.plugin_max_memory_mb = 1;
        let engine = JsEngine::new(&Arc::new(config), None, None).await.unwrap();

        let code = r#"
var arr = [];
for (var i = 0; i < 1000000; i++) {
    arr.push("x".repeat(100));
}
export const noop = 1;
"#;
        let result = engine.load_plugin_default("test-memlimit", code).await;
        assert!(result.is_err(), "memory limit should reject allocation");
    }

    #[tokio::test]
    async fn js_engine_reload_same_plugin() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code_v1 = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.version = 1;
    return input;
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
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.version = 2;
    return input;
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
    async fn js_engine_filter_pass_through_when_no_return() {
        let engine = JsEngine::new(&test_config(), None, None).await.unwrap();

        let code = r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.side_effect = true;
}
"#;
        engine
            .load_plugin_default("test-no-return", code)
            .await
            .unwrap();

        let result: anyhow::Result<Option<serde_json::Value>> = engine
            .call_filter(
                "test-no-return",
                "on_post_creating",
                &serde_json::json!({"title": "x"}),
            )
            .await;
        assert!(
            result.is_err(),
            "function without explicit return should fail (returns undefined)"
        );
    }

    #[tokio::test]
    async fn js_engine_action_timeout_interrupts() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = JsEngine::new(&Arc::new(config), None, None).await.unwrap();

        let code = r#"
export function on_post_created(dataJson) {
    var start = Date.now();
    while (Date.now() - start < 10000) {}
}
"#;
        engine
            .load_plugin_default("test-action-timeout", code)
            .await
            .unwrap();

        let result = engine
            .call_action(
                "test-action-timeout",
                "on_post_created",
                &serde_json::json!({"id": "1"}),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_engine_string_filter_timeout_interrupts() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = JsEngine::new(&Arc::new(config), None, None).await.unwrap();

        let code = r#"
export function render_markdown(content) {
    var start = Date.now();
    while (Date.now() - start < 10000) {}
    return content;
}
"#;
        engine
            .load_plugin_default("test-strfilter-timeout", code)
            .await
            .unwrap();

        let result = engine
            .call_string_filter("test-strfilter-timeout", "render_markdown", "# hello")
            .await;
        assert!(result.is_err());
    }
}
