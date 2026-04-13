//! Lua 引擎封装
//!
//! 基于 mlua 的 Lua 5.4 运行时，支持 Send+Sync（send feature），
//! 原生 serde 集成（Rust struct ↔ Lua table 零序列化映射）。
//! 每个插件拥有独立的 Lua 状态（隔离的全局作用域）。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use mlua::{Error as LuaError, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, VmState};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::config::app::AppConfig;

/// 默认超时指令数（约 2-5 秒执行时间）
const DEFAULT_TIMEOUT_INSTRUCTIONS: i64 = 5_000_000;

/// Lua 插件引擎
///
/// 管理所有 Lua 插件的独立 Lua 状态。
pub struct LuaEngine {
    states: Mutex<HashMap<String, Lua>>,
    config: Arc<AppConfig>,
}

impl LuaEngine {
    /// 创建新的 Lua 引擎
    pub fn new(config: &AppConfig) -> anyhow::Result<Self> {
        Ok(Self {
            states: Mutex::new(HashMap::new()),
            config: Arc::new(config.clone()),
        })
    }

    /// 创建受限 Lua 状态（沙箱）
    fn create_sandboxed_lua(memory_limit_bytes: usize) -> anyhow::Result<Lua> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(memory_limit_bytes)?;
        Ok(lua)
    }

    /// 加载 Lua 插件代码
    pub async fn load_plugin(&self, id: &str, code: &str) -> anyhow::Result<()> {
        let memory_limit = (self.config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let lua = Self::create_sandboxed_lua(memory_limit)?;
        let config = self.config.clone();

        super::lua_host::register_host_functions(&lua, config)?;
        lua.load(code).exec()?;

        self.states.lock().await.insert(id.to_string(), lua);
        Ok(())
    }

    /// 卸载插件（移除 Lua 状态）
    pub async fn unload_plugin(&self, id: &str) {
        self.states.lock().await.remove(id);
    }

    /// 调用 Filter Hook（原生 serde table 映射）
    pub async fn call_filter<T: Serialize + DeserializeOwned + Send>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(None),
        };

        exec_with_timeout(lua, || {
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

    /// 调用 Action Hook（无返回值）
    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        exec_with_timeout(lua, || {
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

    /// 调用 String Filter Hook
    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(None),
        };

        exec_with_timeout(lua, || {
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

    /// 获取已加载 Lua 插件数量
    #[allow(dead_code)]
    pub async fn plugin_count(&self) -> usize {
        self.states.lock().await.len()
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
        Arc::new(AppConfig {
            host: "127.0.0.1".into(),
            port: 0,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "/tmp/test-uploads".into(),
            max_upload_size: 5242880,
            static_dir: "./static".into(),
            base_url: "http://localhost:9000".into(),
            cors_origins: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 8,
            plugin_default_timeout_ms: 2000,
            plugin_disabled: vec![],
            log_dir: "./logs".into(),
            log_max_files: 7,
            rate_limit_global_max: 60,
            rate_limit_global_window: 60,
            rate_limit_register_max: 5,
            rate_limit_register_window: 3600,
            rate_limit_login_max: 10,
            rate_limit_login_window: 60,
            rate_limit_comment_max: 3,
            rate_limit_comment_window: 60,
        })
    }

    #[tokio::test]
    async fn lua_engine_create() {
        let engine = LuaEngine::new(&test_config());
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_load_and_call_filter() {
        let engine = LuaEngine::new(&test_config()).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        input.title = input.title:upper()
        return input
    end
}
"#;
        engine.load_plugin("test-filter", code).await.unwrap();

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
        let engine = LuaEngine::new(&test_config()).unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn lua_engine_call_filter_missing_function() {
        let engine = LuaEngine::new(&test_config()).unwrap();
        engine
            .load_plugin("test-nofunc", "Plugin = {}")
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
        let engine = LuaEngine::new(&test_config()).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        Host.log("info", "post created: " .. tostring(data.id))
    end
}
"#;
        engine.load_plugin("test-action", code).await.unwrap();

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
        let engine = LuaEngine::new(&test_config()).unwrap();

        let code = r#"
Plugin = {
    filter_html = function(html)
        return html:gsub("<head>", '<head><meta property="og:type" content="article">')
    end
}
"#;
        engine.load_plugin("test-strfilter", code).await.unwrap();

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
        let engine = LuaEngine::new(&test_config()).unwrap();
        engine
            .load_plugin("test-unload", "Plugin = {}")
            .await
            .unwrap();
        assert_eq!(engine.plugin_count().await, 1);

        engine.unload_plugin("test-unload").await;
        assert_eq!(engine.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn lua_engine_multiple_plugins() {
        let engine = LuaEngine::new(&test_config()).unwrap();

        for i in 0..3 {
            let code = format!(
                r#"Plugin = {{ on_post_creating = function(input) input.idx = {i}; return input end }}"#
            );
            engine
                .load_plugin(&format!("plugin-{i}"), &code)
                .await
                .unwrap();
        }

        assert_eq!(engine.plugin_count().await, 3);
    }

    #[tokio::test]
    async fn lua_engine_syntax_error_fails_load() {
        let engine = LuaEngine::new(&test_config()).unwrap();
        let result = engine
            .load_plugin("test-bad", "function !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_engine_timeout_interrupts_long_execution() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = LuaEngine::new(&Arc::new(config)).unwrap();

        let code = r#"
Plugin = {
    on_post_creating = function(input)
        local i = 0
        while i < 100000000 do i = i + 1 end
        return input
    end
}
"#;
        engine.load_plugin("test-timeout", code).await.unwrap();

        let result: anyhow::Result<Option<serde_json::Value>> = engine
            .call_filter("test-timeout", "on_post_creating", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn lua_engine_action_exception_does_not_crash() {
        let engine = LuaEngine::new(&test_config()).unwrap();

        let code = r#"
Plugin = {
    on_post_created = function(data)
        error("intentional error")
    end
}
"#;
        engine.load_plugin("test-throw", code).await.unwrap();

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
        let engine = LuaEngine::new(&test_config()).unwrap();

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
        engine.load_plugin("test-cfg", code).await.unwrap();

        let result = engine
            .call_action("test-cfg", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_no_io_os_libs() {
        let engine = LuaEngine::new(&test_config()).unwrap();

        let code = r#"
Plugin = {}
if io ~= nil then error("io should not be available") end
if os ~= nil then error("os should not be available") end
if debug ~= nil then error("debug should not be available") end
"#;
        let result = engine.load_plugin("test-sandbox", code).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn lua_engine_memory_limit_enforced() {
        let mut config = (*test_config()).clone();
        config.plugin_max_memory_mb = 1;
        let engine = LuaEngine::new(&Arc::new(config)).unwrap();

        let code = r#"
local t = {}
for i = 1, 1000000 do
    t[i] = string.rep("x", 100)
end
Plugin = {}
"#;
        let result = engine.load_plugin("test-memlimit", code).await;
        assert!(result.is_err());
    }
}
