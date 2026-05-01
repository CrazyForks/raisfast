//! QuickJS 引擎封装
//!
//! 基于 rquickjs 的 `AsyncRuntime` / `AsyncContext`，
//! 支持 JavaScript 插件在 tokio 异步环境中运行。
//! 每个插件拥有独立的 AsyncRuntime + AsyncContext（完全隔离的内存空间）。
//!
//! ESM 模式：插件使用 `import/export` 语法，
//! 框架从 module namespace 收集 export 函数注册到 Plugin 对象。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use rquickjs::loader::Resolver;
use rquickjs::{AsyncContext, AsyncRuntime, Ctx, Function, Module, Object, Value};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

struct PluginSlot {
    runtime: AsyncRuntime,
    context: AsyncContext,
}

struct JsInstancePool {
    instances: Vec<Mutex<PluginSlot>>,
    next: AtomicUsize,
}

impl JsInstancePool {
    fn new(instances: Vec<PluginSlot>) -> Self {
        Self {
            instances: instances.into_iter().map(Mutex::new).collect(),
            next: AtomicUsize::new(0),
        }
    }

    async fn acquire(&self) -> tokio::sync::MutexGuard<'_, PluginSlot> {
        let len = self.instances.len();
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % len;
        self.instances[idx].lock().await
    }
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

pub struct JsEngine {
    pools: Mutex<HashMap<String, Arc<JsInstancePool>>>,
    permissions_map: Mutex<HashMap<String, Permissions>>,
    default_memory_limit_bytes: usize,
    timeout_ms: u64,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    pool_size: usize,
    event_bus: Option<crate::eventbus::EventBus>,
}

impl JsEngine {
    pub async fn new(
        config: &AppConfig,
        pool: Option<Pool>,
        event_bus: Option<crate::eventbus::EventBus>,
    ) -> anyhow::Result<Self> {
        let default_memory_limit_bytes = (config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let pool_size = config.plugin_js_pool_size.max(1) as usize;

        Ok(Self {
            pools: Mutex::new(HashMap::new()),
            permissions_map: Mutex::new(HashMap::new()),
            default_memory_limit_bytes,
            timeout_ms: config.plugin_default_timeout_ms,
            config: Arc::new(config.clone()),
            pool,
            pool_size,
            event_bus,
        })
    }

    async fn create_instance(
        &self,
        code: &str,
        plugin_id: &str,
        permissions: &Permissions,
        plugin_dir: &Path,
        sdk_source: &'static str,
    ) -> anyhow::Result<PluginSlot> {
        let memory_limit = permissions
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
                PluginLoader::new(plugin_dir.to_path_buf(), sdk_source),
            )
            .await;

        let ctx = AsyncContext::full(&runtime).await?;
        let config = self.config.clone();
        let plugin_id_owned = plugin_id.to_string();
        let perms = permissions.clone();
        ctx.with(|ctx| {
            super::js_host::register_host_functions(
                ctx.clone(),
                config,
                plugin_id_owned,
                perms,
                self.pool.clone(),
                self.event_bus.clone(),
            )?;

            let module = Module::declare(ctx.clone(), "index.js", code)?;
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

        Ok(PluginSlot {
            runtime,
            context: ctx,
        })
    }

    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
        plugin_dir: &Path,
        sdk_source: &'static str,
    ) -> anyhow::Result<()> {
        let mut instances = Vec::with_capacity(self.pool_size);
        for _ in 0..self.pool_size {
            instances.push(
                self.create_instance(code, id, &permissions, plugin_dir, sdk_source)
                    .await?,
            );
        }

        self.permissions_map
            .lock()
            .await
            .insert(id.to_string(), permissions);
        self.pools
            .lock()
            .await
            .insert(id.to_string(), Arc::new(JsInstancePool::new(instances)));
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
        self.pools.lock().await.remove(id);
    }

    pub async fn call_filter<T: Serialize + DeserializeOwned + Send>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let pool = {
            let pools = self.pools.lock().await;
            match pools.get(plugin_id) {
                Some(p) => Arc::clone(p),
                None => return Ok(None),
            }
        };

        let slot = pool.acquire().await;
        let input_json = serde_json::to_string(input)?;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        slot.runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let result: anyhow::Result<Option<T>> = slot
            .context
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

        slot.runtime.set_interrupt_handler(None).await;
        result
    }

    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let pool = {
            let pools = self.pools.lock().await;
            match pools.get(plugin_id) {
                Some(p) => Arc::clone(p),
                None => return Ok(()),
            }
        };

        let slot = pool.acquire().await;
        let data_json = serde_json::to_string(data)?;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        slot.runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let result: anyhow::Result<()> = slot
            .context
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

        slot.runtime.set_interrupt_handler(None).await;
        result
    }

    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let pool = {
            let pools = self.pools.lock().await;
            match pools.get(plugin_id) {
                Some(p) => Arc::clone(p),
                None => return Ok(None),
            }
        };

        let slot = pool.acquire().await;
        let timeout = self.timeout_ms;
        let start = Instant::now();
        slot.runtime
            .set_interrupt_handler(Some(Box::new(move || {
                start.elapsed().as_millis() > u128::from(timeout)
            })))
            .await;

        let func_name_owned = func_name.to_string();
        let input_owned = input.to_string();
        let result: anyhow::Result<Option<String>> = slot
            .context
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

        slot.runtime.set_interrupt_handler(None).await;
        result
    }

    #[allow(dead_code)]
    pub async fn plugin_count(&self) -> usize {
        self.pools.lock().await.len()
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
}
