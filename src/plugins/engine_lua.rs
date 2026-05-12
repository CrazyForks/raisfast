//! Lua engine wrapper (per-request mode)
//!
//! Based on mlua's Lua 5.4 runtime with Send+Sync support (send feature),
//! native serde integration (Rust struct ↔ Lua table zero-serialization mapping).
//!
//! Uses scheme D (per-request): a fresh Lua VM is created for each call and destroyed after use.
//! Lua context creation cost is ~0.1ms, with zero contention, unlimited concurrency, and perfect isolation.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicI64;
use std::sync::atomic::Ordering;

use dashmap::DashMap;
use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, VmState};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

const DEFAULT_TIMEOUT_INSTRUCTIONS: i64 = 5_000_000;

struct LuaPluginEntry {
    code: String,
    permissions: Permissions,
    plugin_dir: PathBuf,
    sdk_source: &'static str,
}

/// Lua plugin engine (per-request mode)
///
/// Stores source code and metadata for all Lua plugins; creates a fresh context for each call.
pub struct LuaEngine {
    plugins: DashMap<String, LuaPluginEntry>,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
}

impl LuaEngine {
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

    fn create_sandboxed_lua(memory_limit_bytes: usize) -> anyhow::Result<Lua> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(memory_limit_bytes)?;
        Ok(lua)
    }

    fn create_instance(&self, entry: &LuaPluginEntry, plugin_id: &str) -> anyhow::Result<Lua> {
        let memory_limit = (self.config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let lua = Self::create_sandboxed_lua(memory_limit)?;
        super::lua_host::register_host_functions(
            &lua,
            self.config.clone(),
            plugin_id.to_string(),
            entry.permissions.clone(),
            self.pool.clone(),
            self.event_bus.clone(),
        )?;
        Self::register_require(&lua, &entry.plugin_dir, entry.sdk_source)?;
        lua.load(&entry.code).set_name("init.lua").exec()?;
        Ok(lua)
    }

    fn register_require(
        lua: &Lua,
        plugin_dir: &Path,
        sdk_source: &'static str,
    ) -> anyhow::Result<()> {
        let globals = lua.globals();

        let dir = plugin_dir.to_path_buf();
        let sdk = sdk_source.to_string();

        let require_fn =
            lua.create_function(move |lua, name: String| -> mlua::Result<mlua::Table> {
                match name.as_str() {
                    "sdk" => {
                        lua.load(&sdk).set_name("sdk").exec()?;
                        let module: mlua::Table = lua.globals().get("_sdk_module")?;
                        Ok(module)
                    }
                    n if n.starts_with("./") || n.starts_with("../") => {
                        let path = dir.join(&name);
                        let canonical = path
                            .canonicalize()
                            .map_err(|e| mlua::Error::runtime(format!("path error: {e}")))?;
                        let plugin_canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
                        if !canonical.starts_with(&plugin_canonical) {
                            return Err(mlua::Error::runtime("path traversal denied"));
                        }
                        let source = std::fs::read_to_string(&canonical)
                            .map_err(|e| mlua::Error::runtime(format!("read error: {e}")))?;
                        lua.load(&source).set_name(&name).exec()?;
                        let module: mlua::Table = lua
                            .globals()
                            .get("_module")
                            .or_else(|_| lua.create_table())?;
                        Ok(module)
                    }
                    _ => Err(mlua::Error::runtime(format!("module not found: {name}"))),
                }
            })?;

        globals.set("require", require_fn)?;
        Ok(())
    }

    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
        plugin_dir: &Path,
        sdk_source: &'static str,
    ) -> anyhow::Result<()> {
        let memory_limit = (self.config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let lua = Self::create_sandboxed_lua(memory_limit)?;
        super::lua_host::register_host_functions(
            &lua,
            self.config.clone(),
            id.to_string(),
            permissions.clone(),
            self.pool.clone(),
            self.event_bus.clone(),
        )?;
        Self::register_require(&lua, plugin_dir, sdk_source)?;
        lua.load(code).set_name("init.lua").exec()?;
        drop(lua);

        self.plugins.insert(
            id.to_string(),
            LuaPluginEntry {
                code: code.to_string(),
                permissions,
                plugin_dir: plugin_dir.to_path_buf(),
                sdk_source,
            },
        );
        Ok(())
    }

    #[cfg(test)]
    pub async fn load_plugin_default(&self, id: &str, code: &str) -> anyhow::Result<()> {
        self.load_plugin(
            id,
            code,
            Permissions::default(),
            Path::new("."),
            crate::plugins::sdk_v1::LUA_SDK_V1,
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

        let lua = self.create_instance(&entry, plugin_id)?;
        exec_with_timeout(&lua, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(None),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

            let input_value = lua.to_value(input)?;
            let result_value = func.call::<mlua::Value>(input_value)?;
            let output: T = lua.from_value(result_value)?;
            Ok(Some(output))
        })
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

        let lua = self.create_instance(&entry, plugin_id)?;
        exec_with_timeout(&lua, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(()),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(()),
            };

            let data_value = lua.to_value(data)?;
            func.call::<()>(data_value)?;
            Ok(())
        })
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

        let lua = self.create_instance(&entry, plugin_id)?;
        exec_with_timeout(&lua, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(None),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

            let result: String = func.call(input)?;
            Ok(Some(result))
        })
    }

    #[allow(dead_code)]
    pub async fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

/// Execute Lua code with timeout (instruction count hook)
fn exec_with_timeout<R>(lua: &Lua, f: impl FnOnce() -> anyhow::Result<R>) -> anyhow::Result<R> {
    let remaining = Arc::new(AtomicI64::new(DEFAULT_TIMEOUT_INSTRUCTIONS));
    let remaining_clone = remaining.clone();

    lua.set_hook(
        HookTriggers::new().every_nth_instruction(1000),
        move |_lua, _debug| {
            if remaining_clone.fetch_sub(1000, Ordering::Relaxed) <= 1000 {
                Err(LuaError::runtime("execution timeout"))
            } else {
                Ok(VmState::Continue)
            }
        },
    )?;

    let result = f();
    lua.remove_hook();
    result
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
    async fn lua_engine_create() {
        let engine = LuaEngine::new(&test_config(), None, None);
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_load_and_call_filter() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        input.title = input.title:upper()
        return input
    end
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
    async fn lua_engine_call_filter_missing_plugin() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lua_engine_call_filter_missing_function() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();
        engine
            .load_plugin_default("test-nofunc", "Plugin = {}")
            .await
            .unwrap();

        let result: Option<serde_json::Value> = engine
            .call_filter("test-nofunc", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lua_engine_call_action() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        RaisFastHost.log("info", "post created: " .. tostring(data.id))
    end
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
    async fn lua_engine_call_string_filter() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    filter_html = function(html)
        return html:gsub("<head>", '<head><meta property="og:type" content="article">')
    end
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
    async fn lua_engine_unload_plugin() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();
        engine
            .load_plugin_default("test-unload", "Plugin = {}")
            .await
            .unwrap();
        assert_eq!(engine.plugin_count().await, 1);

        engine.unload_plugin("test-unload").await;
        assert_eq!(engine.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn lua_engine_multiple_plugins() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        for i in 0..3 {
            let code = format!(
                r#"Plugin = {{ on_post_creating = function(input) input.idx = {i}; return input end }}"#
            );
            engine
                .load_plugin_default(&format!("plugin-{i}"), &code)
                .await
                .unwrap();
        }

        assert_eq!(engine.plugin_count().await, 3);
    }

    #[tokio::test]
    async fn lua_engine_syntax_error_fails_load() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();
        let result = engine
            .load_plugin_default("test-bad", "function !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_engine_timeout_interrupts_long_execution() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = LuaEngine::new(&Arc::new(config), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        local i = 0
        while i < 100000000 do i = i + 1 end
        return input
    end
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
    async fn lua_engine_action_exception_does_not_crash() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        error("intentional error")
    end
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

    #[tokio::test]
    async fn lua_engine_host_get_config_returns_value() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        local env = RaisFastHost.getConfig("app.env")
        if env ~= "test" then
            error("expected test, got: " .. tostring(env))
        end
        local unknown = RaisFastHost.getConfig("nonexistent.key")
        if unknown ~= nil then
            error("expected nil for unknown key")
        end
    end
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
                crate::plugins::sdk_v1::LUA_SDK_V1,
            )
            .await
            .unwrap();

        let result = engine
            .call_action("test-cfg", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_no_io_os_libs() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {}
if io ~= nil then error("io should not be available") end
if os ~= nil then error("os should not be available") end
if debug ~= nil then error("debug should not be available") end
"#;
        let result = engine.load_plugin_default("test-sandbox", code).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_memory_limit_enforced() {
        let mut config = (*test_config()).clone();
        config.plugin_max_memory_mb = 1;
        let engine = LuaEngine::new(&Arc::new(config), None, None).unwrap();

        let code = r#"
local t = {}
for i = 1, 1000000 do
    t[i] = string.rep("x", 100)
end
Plugin = {}
"#;
        let result = engine.load_plugin_default("test-memlimit", code).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_per_request_state_isolation() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
counter = 0
Plugin = {
    on_post_creating = function(input)
        counter = counter + 1
        input.counter = counter
        return input
    end
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
            "per-request: counter should reset to 1 on each call (isolated VM)"
        );
    }

    #[tokio::test]
    async fn lua_concurrent_calls_succeed() {
        let engine = Arc::new(LuaEngine::new(&test_config(), None, None).unwrap());

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        input.processed = true
        return input
    end
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
    async fn lua_call_after_unload_returns_none() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();
        engine
            .load_plugin_default(
                "test-gone",
                "Plugin = { on_post_creating = function(i) return i end }",
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
    async fn lua_engine_filter_returns_nil() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        return nil
    end
}
"#;
        engine
            .load_plugin_default("test-nil-return", code)
            .await
            .unwrap();

        let result: Option<serde_json::Value> = engine
            .call_filter(
                "test-nil-return",
                "on_post_creating",
                &serde_json::json!({"title": "hello"}),
            )
            .await
            .unwrap();
        match result {
            None => {}
            Some(v) if v.is_null() => {}
            other => panic!("expected None or Null, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn lua_engine_filter_exception_does_not_crash() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        error("filter error")
    end
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
    async fn lua_engine_string_filter_exception_does_not_crash() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    filter_html = function(content)
        error("string filter error")
    end
}
"#;
        engine
            .load_plugin_default("test-strfilter-throw", code)
            .await
            .unwrap();

        let result = engine
            .call_string_filter("test-strfilter-throw", "filter_html", "<html></html>")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_engine_string_filter_returns_empty_string() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    filter_html = function(html)
        return ""
    end
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
    async fn lua_engine_filter_modifies_multiple_fields() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        input.title = string.upper(input.title)
        input.slug = string.lower(input.title):gsub("%s+", "-")
        input.processed = true
        input.removable = nil
        return input
    end
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
            "removable field should be nil"
        );
    }

    #[tokio::test]
    async fn lua_engine_reload_same_plugin() {
        let engine = LuaEngine::new(&test_config(), None, None).unwrap();

        let code_v1 = r#"
Plugin = {
    on_post_creating = function(input)
        input.version = 1
        return input
    end
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
Plugin = {
    on_post_creating = function(input)
        input.version = 2
        return input
    end
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
    async fn lua_engine_action_timeout_interrupts() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = LuaEngine::new(&Arc::new(config), None, None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        local i = 0
        while i < 100000000 do i = i + 1 end
    end
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
    async fn lua_engine_string_filter_timeout_interrupts() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = LuaEngine::new(&Arc::new(config), None, None).unwrap();

        let code = r#"
Plugin = {
    render_markdown = function(content)
        local i = 0
        while i < 100000000 do i = i + 1 end
        return content
    end
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
