//! WASM 插件系统
//!
//! 基于 wasmtime 运行时，支持热加载的插件架构。
//! 插件通过 Hook 点与宿主交互，运行在沙箱中。

mod engine;
mod host;
mod manifest;

use axum::response::IntoResponse;
pub use manifest::{HookConfig, HookPoint, Permissions, PluginManifest};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use engine::WasmInstance;
use notify::Watcher;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;

use crate::config::app::AppConfig;
use crate::errors::app_error::{AppError, AppResult};

/// 已加载的插件实例
struct LoadedPlugin {
    manifest: PluginManifest,
    instance: RwLock<WasmInstance>,
}

/// 插件系统核心管理器
///
/// 负责插件的加载、卸载、Hook 调度。
/// 通过 `Arc<PluginManager>` 共享在 AppState 中。
pub struct PluginManager {
    engine: wasmtime::Engine,
    plugins: RwLock<HashMap<String, LoadedPlugin>>,
    config: Arc<AppConfig>,
    watcher: RwLock<Option<notify::RecommendedWatcher>>,
}

impl PluginManager {
    /// 创建新的 PluginManager 并加载插件目录
    pub async fn new(config: Arc<AppConfig>) -> Self {
        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true);
        engine_config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        let engine =
            wasmtime::Engine::new(&engine_config).expect("failed to create wasmtime engine");

        let manager = Self {
            engine,
            plugins: RwLock::new(HashMap::new()),
            config,
            watcher: RwLock::new(None),
        };

        if manager.config.plugin_dir.is_some() {
            manager.load_all().await;
            if manager.config.plugin_hot_reload {
                manager.start_watcher().await;
            }
        }

        manager
    }

    /// 扫描插件目录，加载所有有效插件
    pub async fn load_all(&self) {
        let plugin_dir = match &self.config.plugin_dir {
            Some(d) => d,
            None => return,
        };

        let plugin_dir = Path::new(plugin_dir);
        if !plugin_dir.exists() {
            tracing::info!(
                "plugin directory does not exist, skipping: {}",
                plugin_dir.display()
            );
            return;
        }

        let entries = match std::fs::read_dir(plugin_dir) {
            Ok(e) => e,
            Err(err) => {
                tracing::error!("failed to read plugin directory: {err}");
                return;
            }
        };

        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }

            let manifest_path = entry.path().join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }

            match self.load_plugin_from_dir(&manifest_path).await {
                Ok(id) => tracing::info!("loaded plugin: {id}"),
                Err(err) => {
                    tracing::error!(
                        "failed to load plugin from {}: {err}",
                        entry.path().display()
                    );
                }
            }
        }
    }

    /// 从 plugin.toml 路径加载插件，wasm 文件名从清单中读取
    async fn load_plugin_from_dir(&self, manifest_path: &Path) -> AppResult<String> {
        let dir = manifest_path.parent().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("manifest has no parent directory"))
        })?;

        let manifest_content = std::fs::read_to_string(manifest_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read manifest: {e}")))?;
        let manifest: PluginManifest = toml::from_str(&manifest_content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("parse manifest: {e}")))?;

        let wasm_path = dir.join(&manifest.plugin.wasm);
        if !wasm_path.exists() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "wasm file not found: {}",
                wasm_path.display()
            )));
        }

        self.load_plugin(manifest, &wasm_path).await
    }

    /// 加载单个插件（manifest 已解析）
    async fn load_plugin(&self, manifest: PluginManifest, wasm_path: &Path) -> AppResult<String> {
        let id = manifest.plugin.id.clone();

        if self.config.plugin_disabled.contains(&id) {
            tracing::info!("plugin {id} is disabled, skipping");
            return Ok(id);
        }

        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read wasm: {e}")))?;

        let timeout_ms = manifest
            .permissions
            .timeout_ms
            .unwrap_or(self.config.plugin_default_timeout_ms);

        let instance = WasmInstance::new(
            &self.engine,
            &wasm_bytes,
            manifest.plugin.id.clone(),
            timeout_ms,
            &manifest.permissions,
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("instantiate wasm: {e}")))?;

        let mut plugins = self.plugins.write().await;
        plugins.insert(
            id.clone(),
            LoadedPlugin {
                manifest,
                instance: RwLock::new(instance),
            },
        );

        Ok(id)
    }

    /// 卸载指定插件
    pub async fn unload_plugin(&self, id: &str) {
        let mut plugins = self.plugins.write().await;
        if plugins.remove(id).is_some() {
            tracing::info!("unloaded plugin: {id}");
        }
    }

    /// 重新加载指定插件（热更新）
    pub async fn reload_plugin(&self, plugin_dir: &Path) {
        let manifest_path = plugin_dir.join("plugin.toml");
        if !manifest_path.exists() {
            return;
        }

        let manifest_content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let manifest: PluginManifest = match toml::from_str(&manifest_content) {
            Ok(m) => m,
            Err(_) => return,
        };

        let id = manifest.plugin.id.clone();
        self.unload_plugin(&id).await;

        match self.load_plugin_from_dir(&manifest_path).await {
            Ok(_) => tracing::info!("reloaded plugin: {id}"),
            Err(e) => tracing::error!("failed to reload plugin {id}: {e}"),
        }
    }

    /// 启动文件监听器
    async fn start_watcher(&self) {
        let plugin_dir = match &self.config.plugin_dir {
            Some(d) => d.clone(),
            None => return,
        };

        let path = PathBuf::from(&plugin_dir);
        if !path.exists() {
            return;
        }

        tracing::info!("starting plugin hot-reload watcher on {plugin_dir}");

        let debounced: Arc<std::sync::Mutex<Option<std::time::Instant>>> =
            Arc::new(std::sync::Mutex::new(None));

        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                let event = match res {
                    Ok(e) => e,
                    Err(_) => return,
                };

                let is_wasm = event
                    .paths
                    .iter()
                    .any(|p| p.extension().is_some_and(|ext| ext == "wasm"));

                if !is_wasm {
                    return;
                }

                let mut last = debounced.lock().unwrap();
                let now = std::time::Instant::now();
                if let Some(t) = *last
                    && now.duration_since(t).as_millis() < 1000
                {
                    return;
                }
                *last = Some(now);

                tracing::info!("plugin file change detected, reloading...");
            },
            notify::Config::default().with_poll_interval(std::time::Duration::from_secs(2)),
        )
        .expect("failed to create file watcher");

        watcher
            .watch(&path, notify::RecursiveMode::Recursive)
            .expect("failed to start watching plugin directory");

        let mut w = self.watcher.write().await;
        *w = Some(watcher);
    }

    /// 调度 Filter 类型 Hook（链式调用，每个插件接收上一个的输出）
    ///
    /// 如果没有任何插件注册此 Hook，直接返回输入值（零开销）。
    /// 如果插件返回 None 或出错，跳过该插件继续链式传递。
    pub async fn dispatch_filter<T: Clone + Serialize + DeserializeOwned>(
        &self,
        hook: HookPoint,
        input: T,
    ) -> AppResult<T> {
        let plugins = self.plugins.read().await;
        if plugins.is_empty() {
            return Ok(input);
        }

        let func_name = hook.wasm_func_name();
        let mut current = input;

        let mut sorted: Vec<_> = plugins.values().collect();
        sorted.sort_by_key(|p| {
            p.manifest
                .hooks
                .get(func_name)
                .and_then(|h| h.priority)
                .unwrap_or(100)
        });

        for plugin in sorted {
            let hook_config = match plugin.manifest.hooks.get(func_name) {
                Some(h) => h,
                None => continue,
            };
            let _ = hook_config;

            let mut instance = plugin.instance.write().await;
            match instance.call_json_filter(func_name, &current) {
                Ok(Some(result)) => current = result,
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!(
                        "plugin {} hook {} failed: {e}",
                        plugin.manifest.plugin.id,
                        func_name,
                    );
                }
            }
        }

        Ok(current)
    }

    /// 调度 Action 类型 Hook（顺序执行，忽略返回值）
    pub async fn dispatch_action<T: Serialize>(&self, hook: HookPoint, data: &T) {
        let plugins = self.plugins.read().await;
        if plugins.is_empty() {
            return;
        }

        let func_name = hook.wasm_func_name();

        let mut sorted: Vec<_> = plugins.values().collect();
        sorted.sort_by_key(|p| {
            p.manifest
                .hooks
                .get(func_name)
                .and_then(|h| h.priority)
                .unwrap_or(100)
        });

        for plugin in sorted {
            if !plugin.manifest.hooks.contains_key(func_name) {
                continue;
            }

            let mut instance = plugin.instance.write().await;
            if let Err(e) = instance.call_json_action(func_name, data) {
                tracing::warn!(
                    "plugin {} action {} failed: {e}",
                    plugin.manifest.plugin.id,
                    func_name,
                );
            }
        }
    }

    /// 调度 render_markdown Hook（第一个返回 Some 的插件胜出）
    pub async fn dispatch_render_override(&self, content: &str) -> Option<String> {
        let plugins = self.plugins.read().await;
        if plugins.is_empty() {
            return None;
        }

        let func_name = "render_markdown";

        let mut sorted: Vec<_> = plugins.values().collect();
        sorted.sort_by_key(|p| {
            p.manifest
                .hooks
                .get(func_name)
                .and_then(|h| h.priority)
                .unwrap_or(100)
        });

        for plugin in sorted {
            if !plugin.manifest.hooks.contains_key(func_name) {
                continue;
            }

            let mut instance = plugin.instance.write().await;
            match instance.call_string_filter(func_name, content) {
                Ok(Some(result)) => return Some(result),
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(
                        "plugin {} render_markdown failed: {e}",
                        plugin.manifest.plugin.id,
                    );
                }
            }
        }

        None
    }

    /// 获取已加载插件数量
    pub async fn plugin_count(&self) -> usize {
        self.plugins.read().await.len()
    }

    /// 获取所有已加载插件的元数据
    pub async fn list_plugins(&self) -> Vec<(String, String, String)> {
        let plugins = self.plugins.read().await;
        plugins
            .values()
            .map(|p| {
                (
                    p.manifest.plugin.id.clone(),
                    p.manifest.plugin.name.clone(),
                    p.manifest.plugin.version.clone(),
                )
            })
            .collect()
    }

    /// 调度 handle_route Hook（自定义路由）。
    ///
    /// 遍历注册了 `handle_route` 的插件，第一个返回非空结果的胜出。
    /// 返回 `Some(Response)` 表示插件处理了该请求。
    pub async fn dispatch_route(
        &self,
        path: &str,
        method: &str,
    ) -> Option<axum::response::Response> {
        let plugins = self.plugins.read().await;
        if plugins.is_empty() {
            return None;
        }

        let func_name = "handle_route";

        for plugin in plugins.values() {
            let hook_config = match plugin.manifest.hooks.get(func_name) {
                Some(h) => h,
                None => continue,
            };

            if let Some(pattern) = &hook_config.match_pattern
                && !path_matches_pattern(path, pattern)
            {
                continue;
            }

            let mut instance = plugin.instance.write().await;
            let input = serde_json::json!({
                "path": path,
                "method": method,
            });

            match instance.call_json_filter::<serde_json::Value>(func_name, &input) {
                Ok(Some(result)) => {
                    if let Some(body) = result.get("body").and_then(|b| b.as_str()) {
                        let status =
                            result.get("status").and_then(|s| s.as_u64()).unwrap_or(200) as u16;
                        let status_code = axum::http::StatusCode::from_u16(status)
                            .unwrap_or(axum::http::StatusCode::OK);
                        return Some(
                            (
                                status_code,
                                [(axum::http::header::CONTENT_TYPE, "application/json")],
                                body.to_string(),
                            )
                                .into_response(),
                        );
                    }
                }
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(
                        "plugin {} handle_route failed: {e}",
                        plugin.manifest.plugin.id,
                    );
                }
            }
        }

        None
    }
}

/// 简单的 glob 风格路径匹配。
///
/// 支持 `*` 通配符，如 `/api/v1/plugins/seo/*`。
fn path_matches_pattern(path: &str, pattern: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    if pattern_parts.len() != path_parts.len() && !pattern.contains('*') {
        return false;
    }

    let pi = pattern_parts.iter().peekable();
    let mut pathi = path_parts.iter();

    for pp in pi {
        if pp == &"*" {
            pathi.next();
            continue;
        }
        match pathi.next() {
            Some(sp) if sp == pp => continue,
            _ => return false,
        }
    }

    pathi.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;

    /// 返回一个用于测试的 AppConfig（无插件目录）
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
            plugin_max_memory_mb: 32,
            plugin_default_timeout_ms: 5000,
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

    /// 用 WAT (WebAssembly Text Format) 构建最小 WASM 模块。
    ///
    /// 导出：
    /// - `memory` — 1 页线性内存
    /// - `alloc(size) -> ptr` — 在内存末尾分配空间
    /// - `dealloc(ptr, size)` — 空操作
    /// - `on_post_creating(ptr, len) -> ptr` — echo filter（将输入写为长度前缀格式并返回指针）
    /// - `on_post_created(ptr, len)` — 空操作（action）
    /// - `render_markdown(ptr, len) -> ptr` — echo string filter
    /// - `handle_route(ptr, len) -> ptr` — 返回空（0）
    /// - `infinite_loop()` — 死循环（测试 fuel 耗尽）
    fn build_test_wasm() -> Vec<u8> {
        let wat = r#"
(module
  (memory (export "memory") 1)
  (global $next_ptr (mut i32) (i32.const 0))
  (func (export "alloc") (param $size i32) (result i32)
    (global.get $next_ptr)
    (global.set $next_ptr (i32.add (global.get $next_ptr) (local.get $size)))
  )
  (func (export "dealloc") (param $ptr i32) (param $size i32))

  ;; helper: write length-prefix + copy input to new allocation, return ptr
  ;; layout: [4 bytes LE length][input data]
  ;; uses $next_ptr bump allocator: allocate 4+len, write len at ptr, copy input at ptr+4
  (func $echo_lp (param $ptr i32) (param $len i32) (result i32)
    (local $out i32)
    (local $total i32)
    (local.set $total (i32.add (i32.const 4) (local.get $len)))
    (local.set $out (global.get $next_ptr))
    (global.set $next_ptr (i32.add (global.get $next_ptr) (local.get $total)))
    ;; write length as LE u32 at out[0..4]
    (i32.store (local.get $out) (local.get $len))
    ;; copy input[ptr..ptr+len] to out+4
    (memory.copy (i32.add (local.get $out) (i32.const 4)) (local.get $ptr) (local.get $len))
    (local.get $out)
  )

  (func (export "on_post_creating") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "on_post_created") (param $ptr i32) (param $len i32))
  (func (export "on_comment_created") (param $ptr i32) (param $len i32))
  (func (export "on_login") (param $ptr i32) (param $len i32))
  (func (export "on_post_deleted") (param $ptr i32) (param $len i32))
  (func (export "render_markdown") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "on_comment_creating") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "on_post_updating") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
  (func (export "handle_route") (param $ptr i32) (param $len i32) (result i32)
    (i32.const 0)
  )
  (func (export "filter_html") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
)
"#;
        wat.as_bytes().to_vec()
    }

    // ── path_matches_pattern tests ───────────────────────────────

    #[test]
    fn path_exact_match() {
        assert!(path_matches_pattern("/api/v1/posts", "/api/v1/posts"));
    }

    #[test]
    fn path_no_match() {
        assert!(!path_matches_pattern("/api/v1/posts", "/api/v1/users"));
    }

    #[test]
    fn path_wildcard_match() {
        assert!(path_matches_pattern(
            "/api/v1/plugins/seo/sitemap",
            "/api/v1/plugins/seo/*"
        ));
    }

    #[test]
    fn path_wildcard_no_match_different_length() {
        assert!(!path_matches_pattern(
            "/api/v1/plugins/seo/a/b",
            "/api/v1/plugins/seo/*"
        ));
    }

    #[test]
    fn path_empty_both() {
        assert!(path_matches_pattern("", ""));
    }

    #[test]
    fn path_different_depth_no_wildcard() {
        assert!(!path_matches_pattern("/api/v1", "/api/v1/posts"));
    }

    #[test]
    fn path_root_match() {
        assert!(path_matches_pattern("/", "/"));
    }

    #[test]
    fn path_trailing_segment_mismatch() {
        assert!(!path_matches_pattern(
            "/api/v1/posts/123",
            "/api/v1/posts/456"
        ));
    }

    // ── PluginManager basic tests ────────────────────────────────

    #[tokio::test]
    async fn manager_no_plugins_when_no_dir() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        assert_eq!(mgr.plugin_count().await, 0);
        assert!(mgr.list_plugins().await.is_empty());
    }

    #[tokio::test]
    async fn manager_no_plugins_when_dir_not_exists() {
        let mut config = (*test_config()).clone();
        config.plugin_dir = Some("/tmp/rust-blog-plugin-test-nonexistent".into());
        let mgr = PluginManager::new(Arc::new(config)).await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn manager_unload_nonexistent_plugin() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        mgr.unload_plugin("does-not-exist").await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn dispatch_filter_passthrough_with_no_plugins() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn dispatch_action_with_no_plugins_does_nothing() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        mgr.dispatch_action(HookPoint::PostCreated, &serde_json::json!({"id": "123"}))
            .await;
    }

    #[tokio::test]
    async fn dispatch_render_override_returns_none_with_no_plugins() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        assert!(mgr.dispatch_render_override("# Hello").await.is_none());
    }

    #[tokio::test]
    async fn dispatch_route_returns_none_with_no_plugins() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        assert!(mgr.dispatch_route("/api/v1/test", "GET").await.is_none());
    }

    // ── WasmInstance tests (in engine.rs) ────────────────────────

    // ── PluginManager load from directory ─────────────────────────

    #[tokio::test]
    async fn manager_load_plugin_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.plugin"
name = "Test"
version = "0.1.0"

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        let plugins = mgr.list_plugins().await;
        assert_eq!(plugins[0].0, "com.test.plugin");
    }

    #[tokio::test]
    async fn manager_skip_disabled_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("disabled-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.disabled"
name = "Disabled"
version = "0.1.0"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        config.plugin_disabled = vec!["com.test.disabled".into()];
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn manager_skip_directory_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("incomplete-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();
        // no plugin.toml

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn manager_skip_directory_without_wasm() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("no-wasm-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"x\"\nname=\"X\"\nversion=\"1\"",
        )
        .unwrap();
        // no plugin.wasm

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn manager_unload_existing_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("unload-test");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.unload\"\nname=\"U\"\nversion=\"1\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        mgr.unload_plugin("com.test.unload").await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn manager_reload_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("reload-test");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.reload\"\nname=\"R\"\nversion=\"1\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;
        assert_eq!(mgr.plugin_count().await, 1);

        // reload same plugin
        mgr.reload_plugin(&plugin_dir).await;
        assert_eq!(mgr.plugin_count().await, 1);
    }

    #[tokio::test]
    async fn manager_load_multiple_plugins() {
        let dir = tempfile::tempdir().unwrap();

        for name in &["plugin-a", "plugin-b", "plugin-c"] {
            let pd = dir.path().join(name);
            std::fs::create_dir_all(&pd).unwrap();
            let manifest = format!("[plugin]\nid=\"{name}\"\nname=\"{name}\"\nversion=\"1.0.0\"");
            std::fs::write(pd.join("plugin.toml"), manifest).unwrap();
            std::fs::write(pd.join("plugin.wasm"), build_test_wasm()).unwrap();
        }

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 3);
    }
}
