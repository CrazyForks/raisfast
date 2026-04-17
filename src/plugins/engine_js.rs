//! `QuickJS` 引擎封装
//!
//! 基于 rquickjs 的 `AsyncRuntime` / `AsyncContext`，
//! 支持 JavaScript 插件在 tokio 异步环境中运行。
//! 每个插件拥有独立的 AsyncRuntime + AsyncContext（完全隔离的内存空间）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use rquickjs::{AsyncContext, AsyncRuntime, Function, Object};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;

/// 单个 JS 插件的隔离运行环境
struct PluginSlot {
    runtime: AsyncRuntime,
    context: AsyncContext,
}

/// JS 插件引擎
///
/// 管理所有 JS 插件，每个插件拥有独立的 `AsyncRuntime`（独立内存限制）。
pub struct JsEngine {
    slots: Mutex<HashMap<String, PluginSlot>>,
    permissions_map: Mutex<HashMap<String, Permissions>>,
    default_memory_limit_bytes: usize,
    timeout_ms: u64,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
}

impl JsEngine {
    /// 创建新的 JS 引擎
    pub async fn new(config: &AppConfig, pool: Option<Pool>) -> anyhow::Result<Self> {
        let default_memory_limit_bytes = (config.plugin_max_memory_mb as usize) * 1024 * 1024;

        Ok(Self {
            slots: Mutex::new(HashMap::new()),
            permissions_map: Mutex::new(HashMap::new()),
            default_memory_limit_bytes,
            timeout_ms: config.plugin_default_timeout_ms,
            config: Arc::new(config.clone()),
            pool,
        })
    }

    /// 加载 JS 插件代码到独立上下文（每个插件独立 AsyncRuntime）
    pub async fn load_plugin(
        &self,
        id: &str,
        code: &str,
        permissions: Permissions,
    ) -> anyhow::Result<()> {
        let memory_limit = permissions
            .max_memory_mb
            .map_or(self.default_memory_limit_bytes, |mb| {
                mb as usize * 1024 * 1024
            });

        let runtime = AsyncRuntime::new()?;
        runtime.set_memory_limit(memory_limit).await;
        runtime.set_max_stack_size(512 * 1024).await;

        let ctx = AsyncContext::full(&runtime).await?;
        let config = self.config.clone();
        let plugin_id = id.to_string();
        let perms = permissions.clone();
        ctx.with(|ctx| {
            super::js_host::register_host_functions(
                ctx.clone(),
                config,
                plugin_id,
                perms,
                self.pool.clone(),
            )?;
            ctx.eval::<(), _>(code)?;
            Ok::<_, rquickjs::Error>(())
        })
        .await?;

        self.permissions_map
            .lock()
            .await
            .insert(id.to_string(), permissions);
        self.slots.lock().await.insert(
            id.to_string(),
            PluginSlot {
                runtime,
                context: ctx,
            },
        );
        Ok(())
    }

    /// 加载 JS 插件（兼容旧的无权限接口）
    #[cfg(test)]
    pub async fn load_plugin_default(&self, id: &str, code: &str) -> anyhow::Result<()> {
        self.load_plugin(id, code, Permissions::default()).await
    }

    /// 卸载插件（移除上下文和运行时）
    pub async fn unload_plugin(&self, id: &str) {
        self.slots.lock().await.remove(id);
    }

    /// 调用 Filter Hook（JSON 字符串进出）
    pub async fn call_filter<T: Serialize + DeserializeOwned + Send>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let mut slots = self.slots.lock().await;
        let slot = match slots.get_mut(plugin_id) {
            Some(s) => s,
            None => return Ok(None),
        };

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
                let result_str: String = func.call((input_json,))?;
                let output: T = serde_json::from_str(&result_str)?;
                Ok(Some(output))
            })
            .await;

        if let Some(slot) = slots.get(plugin_id) {
            slot.runtime.set_interrupt_handler(None).await;
        }
        result
    }

    /// 调用 Action Hook（无返回值）
    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let mut slots = self.slots.lock().await;
        let slot = match slots.get_mut(plugin_id) {
            Some(s) => s,
            None => return Ok(()),
        };

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

        if let Some(slot) = slots.get(plugin_id) {
            slot.runtime.set_interrupt_handler(None).await;
        }
        result
    }

    /// 调用 String Filter Hook（如 `render_markdown、filter_html`）
    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let mut slots = self.slots.lock().await;
        let slot = match slots.get_mut(plugin_id) {
            Some(s) => s,
            None => return Ok(None),
        };

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
                let result_str: String = func.call((input_owned,))?;
                Ok(Some(result_str))
            })
            .await;

        if let Some(slot) = slots.get(plugin_id) {
            slot.runtime.set_interrupt_handler(None).await;
        }
        result
    }

    /// 获取已加载 JS 插件数量
    #[allow(dead_code)]
    pub async fn plugin_count(&self) -> usize {
        self.slots.lock().await.len()
    }
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
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 8,
            plugin_default_timeout_ms: 2000,
            plugin_disabled: vec![],
            plugin_vfs_root: "./plugins-data".into(),
            plugin_vfs_max_file_size: 1048576,
            plugin_vfs_max_total_size: 10485760,
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
            rate_limit_api_token_max: 120,
            rate_limit_api_token_window: 60,
            worker_enabled: false,
            worker_concurrency: 1,
            worker_poll_interval_ms: 500,
            worker_default_max_attempts: 3,
            worker_cron_tick_ms: 60000,
            cron_seed_enabled: false,
            cron_schedules: vec![],
            cron_log_retention_days: 30,
            search_engine: "none".into(),
            search_index_dir: "./data/search_index".into(),
            content_type_dir: "./content_types".into(),
            timezone: "UTC".into(),
            extension_dir: "./extensions".into(),
            protected_tables: vec![],
        })
    }

    #[tokio::test]
    async fn js_engine_create() {
        let engine = JsEngine::new(&test_config(), None).await;
        assert!(engine.is_ok());
    }

    #[tokio::test]
    async fn js_engine_load_and_call_filter() {
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.title = input.title.toUpperCase();
        return JSON.stringify(input);
    }
};
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();
        let result: Option<serde_json::Value> = engine
            .call_filter("nonexistent", "on_post_creating", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn js_engine_call_filter_missing_function() {
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"var Plugin = {};"#;
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_created: function(dataJson) {
        var data = JSON.parse(dataJson);
        Host.log("info", "post created: " + data.id);
    }
};
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    filter_html: function(html) {
        return html.replace("<head>", '<head><meta property="og:type" content="article">');
    }
};
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"var Plugin = {};"#;
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        for i in 0..3 {
            let code = format!(
                r#"var Plugin = {{ on_post_creating: function(j) {{ var d = JSON.parse(j); d.idx = {i}; return JSON.stringify(d); }} }};"#
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_created: function(dataJson) {
        Host.log("info", "test message");
    }
};
"#;
        engine.load_plugin_default("test-host", code).await.unwrap();

        let result = engine
            .call_action("test-host", "on_post_created", &serde_json::json!({}))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn js_engine_host_get_config_returns_value() {
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_created: function(dataJson) {
        var env = Host.getConfig("app.env");
        if (env !== "test") {
            throw new Error("expected test, got: " + env);
        }
        var unknown = Host.getConfig("nonexistent.key");
        if (unknown != null) {
            throw new Error("expected null for unknown key");
        }
    }
};
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
    async fn js_engine_syntax_error_fails_load() {
        let engine = JsEngine::new(&test_config(), None).await.unwrap();
        let result = engine
            .load_plugin_default("test-bad-syntax", "var !!!invalid!!!")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn js_engine_timeout_interrupts_long_execution() {
        let mut config = (*test_config()).clone();
        config.plugin_default_timeout_ms = 100;
        let engine = JsEngine::new(&Arc::new(config), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var start = Date.now();
        while (Date.now() - start < 10000) {}
        return inputJson;
    }
};
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code_a = r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.tags = ["a"];
        return JSON.stringify(input);
    }
};
"#;
        let code_b = r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.tags.push("b");
        return JSON.stringify(input);
    }
};
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
        let engine = JsEngine::new(&test_config(), None).await.unwrap();

        let code = r#"
var Plugin = {
    on_post_created: function(dataJson) {
        throw new Error("intentional error");
    }
};
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
}
