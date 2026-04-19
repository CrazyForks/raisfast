//! Lua 引擎封装
//!
//! 基于 mlua 的 Lua 5.4 运行时，支持 Send+Sync（send feature），
//! 原生 serde 集成（Rust struct ↔ Lua table 零序列化映射）。
//! 每个插件拥有独立的 Lua 状态（隔离的全局作用域）。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, VmState};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

const DEFAULT_TIMEOUT_INSTRUCTIONS: i64 = 5_000_000;

/// Lua 实例池
///
/// 为单个 Lua 插件维护多个独立 VM 实例，支持并发执行。
struct LuaInstancePool {
    instances: Vec<Mutex<Lua>>,
    next: AtomicUsize,
}

impl LuaInstancePool {
    fn new(instances: Vec<Lua>) -> Self {
        Self {
            instances: instances.into_iter().map(Mutex::new).collect(),
            next: AtomicUsize::new(0),
        }
    }

    async fn acquire(&self) -> tokio::sync::MutexGuard<'_, Lua> {
        let len = self.instances.len();
        let idx = self.next.fetch_add(1, Ordering::Relaxed) % len;
        self.instances[idx].lock().await
    }
}

/// Lua 插件引擎
///
/// 管理所有 Lua 插件的独立 Lua 状态。
/// 每个插件拥有独立的实例池，不同插件可并发执行。
pub struct LuaEngine {
    pools: Mutex<HashMap<String, Arc<LuaInstancePool>>>,
    permissions_map: Mutex<HashMap<String, Permissions>>,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    pool_size: usize,
}

impl LuaEngine {
    pub fn new(config: &AppConfig, pool: Option<Pool>) -> anyhow::Result<Self> {
        let pool_size = config.plugin_lua_pool_size.max(1) as usize;
        Ok(Self {
            pools: Mutex::new(HashMap::new()),
            permissions_map: Mutex::new(HashMap::new()),
            config: Arc::new(config.clone()),
            pool,
            pool_size,
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

    fn create_instance(
        &self,
        code: &str,
        plugin_id: &str,
        permissions: &Permissions,
        memory_limit: usize,
    ) -> anyhow::Result<Lua> {
        let lua = Self::create_sandboxed_lua(memory_limit)?;
        super::lua_host::register_host_functions(
            &lua,
            self.config.clone(),
            plugin_id.to_string(),
            permissions.clone(),
            self.pool.clone(),
        )?;
        lua.load(code).exec()?;
        Ok(lua)
    }

    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
    ) -> anyhow::Result<()> {
        let memory_limit = (self.config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let mut instances = Vec::with_capacity(self.pool_size);
        for _ in 0..self.pool_size {
            instances.push(self.create_instance(code, id, &permissions, memory_limit)?);
        }

        self.permissions_map
            .lock()
            .await
            .insert(id.to_string(), permissions);
        self.pools
            .lock()
            .await
            .insert(id.to_string(), Arc::new(LuaInstancePool::new(instances)));
        Ok(())
    }

    #[cfg(test)]
    pub async fn load_plugin_default(&self, id: &str, code: &str) -> anyhow::Result<()> {
        self.load_plugin(id, code, Permissions::default()).await
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

        let lua = pool.acquire().await;
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
        let pool = {
            let pools = self.pools.lock().await;
            match pools.get(plugin_id) {
                Some(p) => Arc::clone(p),
                None => return Ok(()),
            }
        };

        let lua = pool.acquire().await;
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
        let pool = {
            let pools = self.pools.lock().await;
            match pools.get(plugin_id) {
                Some(p) => Arc::clone(p),
                None => return Ok(None),
            }
        };

        let lua = pool.acquire().await;
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
        self.pools.lock().await.len()
    }
}

/// 带超时执行 Lua 代码（指令计数 hook）
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
        let engine = LuaEngine::new(&test_config(), None);
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_load_and_call_filter() {
        let engine = LuaEngine::new(&test_config(), None).unwrap();

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
        let engine = LuaEngine::new(&test_config(), None).unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lua_engine_call_filter_missing_function() {
        let engine = LuaEngine::new(&test_config(), None).unwrap();
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
        let engine = LuaEngine::new(&test_config(), None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        Host.log("info", "post created: " .. tostring(data.id))
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
        let engine = LuaEngine::new(&test_config(), None).unwrap();

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
        let engine = LuaEngine::new(&test_config(), None).unwrap();
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
        let engine = LuaEngine::new(&test_config(), None).unwrap();

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
        let engine = LuaEngine::new(&test_config(), None).unwrap();
        let result = engine
            .load_plugin_default("test-bad", "function !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_engine_timeout_interrupts_long_execution() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = LuaEngine::new(&Arc::new(config), None).unwrap();

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
        let engine = LuaEngine::new(&test_config(), None).unwrap();

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
        let engine = LuaEngine::new(&test_config(), None).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        local env = Host.getConfig("app.env")
        if env ~= "test" then
            error("expected test, got: " .. tostring(env))
        end
        local unknown = Host.getConfig("nonexistent.key")
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
        engine.load_plugin("test-cfg", code, perms).await.unwrap();

        let result = engine
            .call_action("test-cfg", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_no_io_os_libs() {
        let engine = LuaEngine::new(&test_config(), None).unwrap();

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
        let engine = LuaEngine::new(&Arc::new(config), None).unwrap();

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
}
