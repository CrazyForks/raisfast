//! WASM 插件系统
//!
//! 基于 wasmtime 运行时，支持热加载的插件架构。
//! 插件通过 Hook 点与宿主交互，运行在沙箱中。

mod engine;
mod host;
mod manifest;

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
            let wasm_path = entry.path().join("plugin.wasm");

            if !manifest_path.exists() || !wasm_path.exists() {
                continue;
            }

            match self.load_plugin(&manifest_path, &wasm_path).await {
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

    /// 加载单个插件
    async fn load_plugin(&self, manifest_path: &Path, wasm_path: &Path) -> AppResult<String> {
        let manifest_content = std::fs::read_to_string(manifest_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read manifest: {e}")))?;
        let manifest: PluginManifest = toml::from_str(&manifest_content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("parse manifest: {e}")))?;

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
        let wasm_path = plugin_dir.join("plugin.wasm");

        if !manifest_path.exists() || !wasm_path.exists() {
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

        match self.load_plugin(&manifest_path, &wasm_path).await {
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
}
