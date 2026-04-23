//! 插件系统
//!
//! 支持三运行时：WASM (wasmtime)、JavaScript (QuickJS/rquickjs)、Lua (mlua)。
//! 通过 feature flag `plugin-wasm` / `plugin-js` / `plugin-lua` 控制编译。
//! 插件通过 Hook 点与宿主交互，运行在沙箱中。
//!
//! # 新增能力（v2）
//!
//! - **Host API 扩展**: `httpGet`, `httpPost`, `getPost`, `getData`, `setData`, `dbQuery`
//! - **权限执行**: manifest 声明的 permissions 在运行时强制校验
//! - **生命周期 Hook**: `on_load` / `on_unload` 回调
//! - **错误恢复**: 连续错误达到阈值自动禁用插件
//! - **`EventBus`**: 事件驱动架构，插件可订阅内部事件
//! - **性能指标**: 每个插件的 Hook 执行耗时、错误次数统计
//! - **依赖管理**: manifest 可声明插件依赖，加载时检测
//! - **管理 API**: 运行时启用/禁用/重载插件

#[cfg(feature = "plugin-wasm")]
mod bindings;
#[cfg(feature = "plugin-wasm")]
mod engine;
#[cfg(feature = "plugin-js")]
mod engine_js;
#[cfg(feature = "plugin-lua")]
mod engine_lua;
#[cfg(feature = "plugin-wasm")]
mod host;
pub mod host_common;
#[cfg(feature = "plugin-js")]
mod js_host;
#[cfg(feature = "plugin-lua")]
mod lua_host;

pub mod http_client;
mod manifest;
pub mod permissions;
pub mod vfs;

pub use manifest::{CronEntry, HookConfig, HookPoint, Permissions, PluginManifest, RouteDef};
pub use permissions::PermissionChecker;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::response::IntoResponse;
use notify::Watcher;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;

#[cfg(feature = "plugin-wasm")]
use engine::WasmInstancePool;
#[cfg(feature = "plugin-js")]
use engine_js::JsEngine;
#[cfg(feature = "plugin-lua")]
use engine_lua::LuaEngine;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};

/// 将 sqlx 任意查询结果行转换为 JSON 数组字符串
///
/// 仅支持当前编译的数据库后端。
#[cfg(feature = "db-sqlite")]
pub(crate) fn rows_to_json(rows: &[sqlx::sqlite::SqliteRow]) -> String {
    use sqlx::{Column, Row};
    if rows.is_empty() {
        return "[]".to_string();
    }
    let columns: Vec<String> = rows[0]
        .columns()
        .iter()
        .map(|c| c.name().to_string())
        .collect();
    let result: Vec<serde_json::Map<String, serde_json::Value>> = rows
        .iter()
        .map(|row| {
            let mut map = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let val: Option<String> = row.try_get::<Option<String>, _>(i).ok().flatten();
                match val {
                    Some(v) => {
                        if let Ok(n) = v.parse::<i64>() {
                            map.insert(col.clone(), serde_json::Value::Number(n.into()));
                        } else if let Ok(b) = v.parse::<bool>() {
                            map.insert(col.clone(), serde_json::Value::Bool(b));
                        } else {
                            map.insert(col.clone(), serde_json::Value::String(v));
                        }
                    }

                    None => {
                        map.insert(col.clone(), serde_json::Value::Null);
                    }
                }
            }
            map
        })
        .collect();
    serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(not(feature = "db-sqlite"))]
pub(crate) fn rows_to_json(_rows: &Vec<()>) -> String {
    "[]".to_string()
}

/// 连续错误达到此阈值后自动禁用插件
const AUTO_DISABLE_THRESHOLD: u32 = 5;

/// 已加载的插件实例（WASM、JS 或 Lua）
enum LoadedPluginInstance {
    #[cfg(feature = "plugin-wasm")]
    Wasm(Arc<WasmInstancePool>),
    #[cfg(feature = "plugin-js")]
    Js(String),
    #[cfg(feature = "plugin-lua")]
    Lua(String),
}

/// 插件健康状态与性能指标
#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginHealth {
    pub error_count: u32,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub auto_disabled: bool,
}

/// 插件 Hook 执行性能指标
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginMetrics {
    pub total_calls: u64,
    pub total_errors: u64,
    pub total_duration_us: u64,
}

/// 已加载的插件
struct LoadedPlugin {
    manifest: PluginManifest,
    instance: LoadedPluginInstance,
    health: RwLock<PluginHealth>,
    metrics: RwLock<HashMap<String, PluginMetrics>>,
}

/// 插件系统核心管理器
///
/// 负责插件的加载、卸载、Hook 调度。
/// 通过 `Arc<PluginManager>` 共享在 `AppState` 中。
pub struct PluginManager {
    #[cfg(feature = "plugin-wasm")]
    engine: wasmtime::Engine,
    #[cfg(feature = "plugin-js")]
    js_engine: JsEngine,
    #[cfg(feature = "plugin-lua")]
    lua_engine: LuaEngine,
    plugins: RwLock<HashMap<String, LoadedPlugin>>,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    watcher: RwLock<Option<notify::RecommendedWatcher>>,
    reload_tx: tokio::sync::mpsc::Sender<PathBuf>,
    reload_rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<PathBuf>>>,
    event_bus: tokio::sync::broadcast::Sender<Arc<PluginEvent>>,
}

/// 插件系统内部事件
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub enum PluginEvent {
    PluginLoaded {
        id: String,
        name: String,
    },
    PluginUnloaded {
        id: String,
    },
    PluginReloaded {
        id: String,
    },
    PluginDisabled {
        id: String,
        reason: String,
    },
    HookFailed {
        plugin_id: String,
        hook: String,
        error: String,
    },
}

/// 插件列表响应项（管理 API 使用）
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfoResponse {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub runtime: String,
    pub enabled: bool,
    pub health: PluginHealth,
    pub hooks: Vec<String>,
    pub metrics: HashMap<String, PluginMetrics>,
    pub permissions: Permissions,
}

/// 设置 `PluginManager` 的可选依赖
pub struct PluginManagerOptions {
    pub pool: Option<Pool>,
    pub event_bus: Option<crate::eventbus::EventBus>,
}

impl PluginManager {
    /// 创建新的 `PluginManager` 并加载插件目录，返回 `Arc<Self>`。
    ///
    /// 返回 `Arc` 是因为热重载 watcher 需要持有自引用来执行 reload。
    pub async fn new(config: Arc<AppConfig>) -> Arc<Self> {
        Self::new_with_options(
            config,
            PluginManagerOptions {
                pool: None,
                event_bus: None,
            },
        )
        .await
    }

    /// 带可选依赖创建 `PluginManager`
    pub async fn new_with_options(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self> {
        let manager = Self::build_instance(config, opts).await;

        if manager.config.plugin_dir.is_some() {
            manager.load_all().await;
            if manager.config.plugin_hot_reload {
                let mgr = manager.clone();
                let mut rx_guard = manager.reload_rx.lock().await;
                if let Some(reload_rx) = rx_guard.take() {
                    tokio::spawn(async move {
                        let mut rx = reload_rx;
                        while let Some(path) = rx.recv().await {
                            mgr.reload_changed_file(&path).await;
                        }
                    });
                }
                manager.start_watcher().await;
            }
        }

        manager
    }

    /// 创建空的 `PluginManager`（不扫描目录），由 ExtensionManager 调度加载
    pub async fn new_empty(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self> {
        Self::build_instance(config, opts).await
    }

    /// 构建 PluginManager 实例（引擎初始化，不加载插件）
    async fn build_instance(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self> {
        #[cfg(feature = "plugin-wasm")]
        let engine = {
            let mut engine_config = wasmtime::Config::new();
            engine_config.consume_fuel(true);
            engine_config.wasm_component_model(true);
            engine_config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
            wasmtime::Engine::new(&engine_config).expect("failed to create wasmtime engine")
        };

        #[cfg(feature = "plugin-js")]
        let js_engine = JsEngine::new(&config, opts.pool.clone(), opts.event_bus.clone())
            .await
            .expect("failed to create js engine");

        #[cfg(feature = "plugin-lua")]
        let lua_engine = LuaEngine::new(&config, opts.pool.clone(), opts.event_bus)
            .expect("failed to create lua engine");

        let (reload_tx, reload_rx) = tokio::sync::mpsc::channel::<PathBuf>(32);
        let (event_tx, _) = tokio::sync::broadcast::channel::<Arc<PluginEvent>>(256);

        Arc::new(Self {
            #[cfg(feature = "plugin-wasm")]
            engine,
            #[cfg(feature = "plugin-js")]
            js_engine,
            #[cfg(feature = "plugin-lua")]
            lua_engine,
            plugins: RwLock::new(HashMap::new()),
            config,
            pool: opts.pool,
            watcher: RwLock::new(None),
            reload_tx,
            reload_rx: tokio::sync::Mutex::new(Some(reload_rx)),
            event_bus: event_tx,
        })
    }

    /// 设置数据库连接池（如果创建时未提供）
    pub fn set_pool(&mut self, pool: Pool) {
        self.pool = Some(pool);
    }

    /// 订阅插件系统事件
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<Arc<PluginEvent>> {
        self.event_bus.subscribe()
    }

    /// 发布插件系统事件
    fn emit_event(&self, event: PluginEvent) {
        let _ = self.event_bus.send(Arc::new(event));
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

        let mut manifests: Vec<(PathBuf, PluginManifest)> = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }

            let manifest_path = entry.path().join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }

            let content = match std::fs::read_to_string(&manifest_path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("failed to read {}: {e}", manifest_path.display());
                    continue;
                }
            };

            let manifest: PluginManifest = match toml::from_str(&content) {
                Ok(m) => m,
                Err(e) => {
                    tracing::error!("failed to parse {}: {e}", manifest_path.display());
                    continue;
                }
            };

            if self.config.plugin_disabled.contains(&manifest.plugin.id) {
                tracing::info!("plugin {} is disabled, skipping", manifest.plugin.id);
                continue;
            }

            manifests.push((manifest_path, manifest));
        }

        let sorted_ids = topological_sort(
            &manifests
                .iter()
                .map(|(_, m)| (m.plugin.id.clone(), m.clone()))
                .collect(),
        );

        let manifest_map: HashMap<String, usize> = manifests
            .iter()
            .enumerate()
            .map(|(i, (_, m))| (m.plugin.id.clone(), i))
            .collect();

        for id in sorted_ids {
            if let Some(&idx) = manifest_map.get(&id) {
                let (manifest_path, _manifest) = &manifests[idx];
                match self.load_plugin_from_dir(manifest_path).await {
                    Ok(loaded_id) => {
                        tracing::info!("loaded plugin: {loaded_id}");
                    }
                    Err(err) => {
                        tracing::error!("failed to load plugin {id}: {err}");
                    }
                }
            }
        }
    }

    /// 从 plugin.toml 所在目录加载插件，根据 runtime 字段选择引擎
    pub async fn load_plugin_from_dir(&self, manifest_path: &Path) -> AppResult<String> {
        let dir = manifest_path.parent().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("manifest has no parent directory"))
        })?;

        let manifest_content = std::fs::read_to_string(manifest_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read manifest: {e}")))?;
        let manifest: PluginManifest = toml::from_str(&manifest_content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("parse manifest: {e}")))?;

        if self.config.plugin_disabled.contains(&manifest.plugin.id) {
            tracing::info!("plugin {} is disabled, skipping", manifest.plugin.id);
            return Ok(manifest.plugin.id);
        }

        match manifest.plugin.runtime.as_str() {
            #[cfg(feature = "plugin-wasm")]
            "wasm" => {
                let wasm_path = dir.join(&manifest.plugin.wasm);
                if !wasm_path.exists() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "wasm file not found: {}",
                        wasm_path.display()
                    )));
                }
                self.load_wasm_plugin(manifest, &wasm_path).await
            }
            #[cfg(feature = "plugin-js")]
            "js" => {
                let entry_path = dir.join(&manifest.plugin.entry);
                if !entry_path.exists() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "js entry file not found: {}",
                        entry_path.display()
                    )));
                }
                self.load_js_plugin(manifest, &entry_path).await
            }
            #[cfg(feature = "plugin-lua")]
            "lua" => {
                let entry_file = if manifest.plugin.entry == "index.js" {
                    "init.lua"
                } else {
                    &manifest.plugin.entry
                };
                let entry_path = dir.join(entry_file);
                if !entry_path.exists() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "lua entry file not found: {}",
                        entry_path.display()
                    )));
                }
                self.load_lua_plugin(manifest, &entry_path).await
            }
            runtime => {
                tracing::warn!(
                    "plugin {} has unsupported runtime '{runtime}', skipping",
                    manifest.plugin.id
                );
                Ok(manifest.plugin.id)
            }
        }
    }

    /// 加载 WASM 插件
    #[cfg(feature = "plugin-wasm")]
    async fn load_wasm_plugin(
        &self,
        manifest: PluginManifest,
        wasm_path: &Path,
    ) -> AppResult<String> {
        let id = manifest.plugin.id.clone();
        let name = manifest.plugin.name.clone();

        let wasm_bytes = std::fs::read(wasm_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read wasm: {e}")))?;

        let timeout_ms = manifest
            .permissions
            .timeout_ms
            .unwrap_or(self.config.plugin_default_timeout_ms);

        let host_ctx = std::sync::Arc::new(host_common::HostContext::new(
            "wasm",
            self.config.clone(),
            manifest.plugin.id.clone(),
            manifest.permissions.clone(),
            self.pool.clone(),
        ));

        let pool_size = self.config.plugin_wasm_pool_size.max(1) as usize;
        let instance_pool = WasmInstancePool::create_pool(
            &self.engine,
            &wasm_bytes,
            host_ctx,
            timeout_ms,
            pool_size,
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("instantiate wasm pool: {e}")))?;

        let mut plugins = self.plugins.write().await;
        plugins.insert(
            id.clone(),
            LoadedPlugin {
                manifest,
                instance: LoadedPluginInstance::Wasm(Arc::new(instance_pool)),
                health: RwLock::new(PluginHealth::default()),
                metrics: RwLock::new(HashMap::new()),
            },
        );
        let cron_entries = plugins
            .get(&id)
            .map(|p| p.manifest.cron.clone())
            .unwrap_or_default();
        drop(plugins);

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// 加载 JS 插件
    #[cfg(feature = "plugin-js")]
    async fn load_js_plugin(
        &self,
        manifest: PluginManifest,
        entry_path: &Path,
    ) -> AppResult<String> {
        let id = manifest.plugin.id.clone();
        let name = manifest.plugin.name.clone();

        let code = std::fs::read_to_string(entry_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read js entry: {e}")))?;

        self.js_engine
            .load_plugin(&id, &code, manifest.permissions.clone())
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("load js plugin: {e}")))?;

        let mut plugins = self.plugins.write().await;
        plugins.insert(
            id.clone(),
            LoadedPlugin {
                manifest,
                instance: LoadedPluginInstance::Js(id.clone()),
                health: RwLock::new(PluginHealth::default()),
                metrics: RwLock::new(HashMap::new()),
            },
        );
        let cron_entries = plugins
            .get(&id)
            .map(|p| p.manifest.cron.clone())
            .unwrap_or_default();
        drop(plugins);

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// 加载 Lua 插件
    #[cfg(feature = "plugin-lua")]
    async fn load_lua_plugin(
        &self,
        manifest: PluginManifest,
        entry_path: &Path,
    ) -> AppResult<String> {
        let id = manifest.plugin.id.clone();
        let name = manifest.plugin.name.clone();

        let code = std::fs::read_to_string(entry_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read lua entry: {e}")))?;

        let permissions = manifest.permissions.clone();
        self.lua_engine
            .load_plugin(&id, &code, permissions)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("load lua plugin: {e}")))?;

        let mut plugins = self.plugins.write().await;
        plugins.insert(
            id.clone(),
            LoadedPlugin {
                manifest,
                instance: LoadedPluginInstance::Lua(id.clone()),
                health: RwLock::new(PluginHealth::default()),
                metrics: RwLock::new(HashMap::new()),
            },
        );
        let cron_entries = plugins
            .get(&id)
            .map(|p| p.manifest.cron.clone())
            .unwrap_or_default();
        drop(plugins);

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// 卸载指定插件
    pub async fn unload_plugin(&self, id: &str) {
        let mut plugins = self.plugins.write().await;
        if let Some(removed) = plugins.remove(id) {
            match &removed.instance {
                #[cfg(feature = "plugin-js")]
                LoadedPluginInstance::Js(_) => {
                    drop(removed);
                    self.js_engine.unload_plugin(id).await;
                }
                #[cfg(feature = "plugin-lua")]
                LoadedPluginInstance::Lua(_) => {
                    drop(removed);
                    self.lua_engine.unload_plugin(id).await;
                }
                #[cfg(feature = "plugin-wasm")]
                LoadedPluginInstance::Wasm(_) => {}
            }
            tracing::info!("unloaded plugin: {id}");
            drop(plugins);
            self.remove_crons_for_plugin(id).await;
            self.emit_event(PluginEvent::PluginUnloaded { id: id.to_string() });
        }
    }

    /// 同步插件的 Cron 调度到数据库
    async fn sync_crons_for_plugin(&self, plugin_id: &str, entries: &[CronEntry]) {
        if let Some(ref pool) = self.pool
            && let Err(e) = crate::worker::sync_plugin_crons(pool, plugin_id, entries).await
        {
            tracing::warn!("failed to sync crons for plugin {plugin_id}: {e}");
        }
    }

    /// 删除插件关联的 Cron 调度
    async fn remove_crons_for_plugin(&self, plugin_id: &str) {
        if let Some(ref pool) = self.pool
            && let Err(e) = crate::worker::remove_plugin_crons(pool, plugin_id).await
        {
            tracing::warn!("failed to remove crons for plugin {plugin_id}: {e}");
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

    /// 启动文件监听器。
    ///
    /// 检测 `.wasm` / `.js` 文件变化，通过 channel 通知 reload task。
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

        let tx = self.reload_tx.clone();
        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                let event = match res {
                    Ok(e) => e,
                    Err(_) => return,
                };

                for changed in &event.paths {
                    let is_relevant = changed
                        .extension()
                        .is_some_and(|ext| ext == "wasm" || ext == "js" || ext == "lua");
                    if !is_relevant {
                        continue;
                    }

                    let mut last = debounced.lock().unwrap_or_else(|e| e.into_inner());
                    let now = std::time::Instant::now();
                    if let Some(t) = *last
                        && now.duration_since(t).as_millis() < 1000
                    {
                        return;
                    }
                    *last = Some(now);

                    let _ = tx.blocking_send(changed.clone());
                    break;
                }
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

    /// 处理文件变化事件，找到并重载对应插件目录。
    async fn reload_changed_file(&self, changed_file: &Path) {
        let plugin_dir = match &self.config.plugin_dir {
            Some(d) => PathBuf::from(d),
            None => return,
        };

        let changed_name = match changed_file.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => return,
        };

        for entry in match std::fs::read_dir(&plugin_dir) {
            Ok(e) => e,
            Err(_) => return,
        }
        .flatten()
        {
            if !entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                continue;
            }

            let manifest_path = entry.path().join("plugin.toml");
            if !manifest_path.exists() {
                continue;
            }

            let dir_path = entry.path();
            let candidate = dir_path.join(changed_name);
            if candidate.exists() || Some(dir_path.as_path()) == changed_file.parent() {
                let id = dir_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                tracing::info!("hot-reloading plugin from {id}...");
                self.reload_plugin(&dir_path).await;
                return;
            }
        }

        tracing::warn!(
            "file change detected but no matching plugin directory found: {}",
            changed_file.display()
        );
    }

    /// 调度 Filter 类型 Hook（链式调用）
    pub async fn dispatch_filter<T: Clone + Serialize + DeserializeOwned + Send>(
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
            if !plugin.manifest.hooks.contains_key(func_name) {
                continue;
            }

            let plugin_id = plugin.manifest.plugin.id.clone();
            if !self.is_plugin_enabled(&plugin_id).await {
                continue;
            }

            let start = std::time::Instant::now();
            let result = match &plugin.instance {
                #[cfg(feature = "plugin-wasm")]
                LoadedPluginInstance::Wasm(wasm) => {
                    let mut instance = wasm.acquire().await;
                    let timeout = std::time::Duration::from_millis(instance.timeout_ms());
                    tokio::time::timeout(timeout, async {
                        tokio::task::block_in_place(|| {
                            instance.call_json_filter(func_name, &current)
                        })
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "plugin {} exceeded wall-clock timeout ({}ms)",
                            plugin_id,
                            timeout.as_millis()
                        ))
                    })
                }
                #[cfg(feature = "plugin-js")]
                LoadedPluginInstance::Js(pid) => self
                    .js_engine
                    .call_filter(pid, func_name, &current)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[cfg(feature = "plugin-lua")]
                LoadedPluginInstance::Lua(pid) => self
                    .lua_engine
                    .call_filter(pid, func_name, &current)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
            };

            let elapsed = start.elapsed().as_micros() as u64;
            let is_error = result.is_err();

            match result {
                Ok(Some(result)) => {
                    current = result;
                    self.reset_error_count(&plugin_id).await;
                }
                Ok(None) => {
                    self.reset_error_count(&plugin_id).await;
                }
                Err(e) => {
                    let err_msg = format!("{e}");
                    tracing::warn!("plugin {} hook {} failed: {err_msg}", plugin_id, func_name,);
                    self.record_hook_error(&plugin_id, func_name, &err_msg)
                        .await;
                }
            }

            self.record_hook_metrics(&plugin_id, func_name, elapsed, is_error)
                .await;
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

            let plugin_id = plugin.manifest.plugin.id.clone();
            if !self.is_plugin_enabled(&plugin_id).await {
                continue;
            }

            let start = std::time::Instant::now();
            let result = match &plugin.instance {
                #[cfg(feature = "plugin-wasm")]
                LoadedPluginInstance::Wasm(wasm) => {
                    let mut instance = wasm.acquire().await;
                    let timeout = std::time::Duration::from_millis(instance.timeout_ms());
                    tokio::time::timeout(timeout, async {
                        tokio::task::block_in_place(|| instance.call_json_action(func_name, data))
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "plugin {} exceeded wall-clock timeout ({}ms)",
                            plugin_id,
                            timeout.as_millis()
                        ))
                    })
                }
                #[cfg(feature = "plugin-js")]
                LoadedPluginInstance::Js(pid) => self
                    .js_engine
                    .call_action(pid, func_name, data)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[cfg(feature = "plugin-lua")]
                LoadedPluginInstance::Lua(pid) => self
                    .lua_engine
                    .call_action(pid, func_name, data)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
            };

            let elapsed = start.elapsed().as_micros() as u64;
            let is_error = result.is_err();

            if let Err(e) = result {
                let err_msg = format!("{e}");
                tracing::warn!(
                    "plugin {} action {} failed: {err_msg}",
                    plugin_id,
                    func_name,
                );
                self.record_hook_error(&plugin_id, func_name, &err_msg)
                    .await;
            } else {
                self.reset_error_count(&plugin_id).await;
            }

            self.record_hook_metrics(&plugin_id, func_name, elapsed, is_error)
                .await;
        }
    }

    /// 调度 `render_markdown` Hook（第一个返回 Some 的插件胜出）
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

            let plugin_id = plugin.manifest.plugin.id.clone();
            if !self.is_plugin_enabled(&plugin_id).await {
                continue;
            }

            let start = std::time::Instant::now();
            let result = match &plugin.instance {
                #[cfg(feature = "plugin-wasm")]
                LoadedPluginInstance::Wasm(wasm) => {
                    let mut instance = wasm.acquire().await;
                    let content_str = content.to_string();
                    let timeout = std::time::Duration::from_millis(instance.timeout_ms());
                    tokio::time::timeout(timeout, async {
                        tokio::task::block_in_place(|| {
                            instance.call_json_filter::<String>(func_name, &content_str)
                        })
                    })
                    .await
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!(
                            "plugin {} exceeded wall-clock timeout ({}ms)",
                            plugin_id,
                            timeout.as_millis()
                        ))
                    })
                }
                #[cfg(feature = "plugin-js")]
                LoadedPluginInstance::Js(pid) => self
                    .js_engine
                    .call_string_filter(pid, func_name, content)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[cfg(feature = "plugin-lua")]
                LoadedPluginInstance::Lua(pid) => self
                    .lua_engine
                    .call_string_filter(pid, func_name, content)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
            };

            let elapsed = start.elapsed().as_micros() as u64;
            let is_error = result.is_err();

            match result {
                Ok(Some(r)) => {
                    self.reset_error_count(&plugin_id).await;
                    self.record_hook_metrics(&plugin_id, func_name, elapsed, false)
                        .await;
                    return Some(r);
                }
                Ok(None) => {
                    self.reset_error_count(&plugin_id).await;
                    self.record_hook_metrics(&plugin_id, func_name, elapsed, false)
                        .await;
                    continue;
                }
                Err(e) => {
                    let err_msg = format!("{e}");
                    tracing::warn!("plugin {} render_markdown failed: {err_msg}", plugin_id);
                    self.record_hook_error(&plugin_id, func_name, &err_msg)
                        .await;
                    self.record_hook_metrics(&plugin_id, func_name, elapsed, is_error)
                        .await;
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

    /// 获取所有插件的详细信息（含健康状态、指标、Hook 列表）
    pub async fn list_plugins_detail(&self) -> Vec<PluginInfoResponse> {
        let plugins = self.plugins.read().await;
        let mut result = Vec::new();
        for p in plugins.values() {
            let health = p.health.read().await.clone();
            let metrics = p.metrics.read().await.clone();
            let hooks: Vec<String> = p.manifest.hooks.keys().cloned().collect();
            result.push(PluginInfoResponse {
                id: p.manifest.plugin.id.clone(),
                name: p.manifest.plugin.name.clone(),
                version: p.manifest.plugin.version.clone(),
                description: p.manifest.plugin.description.clone(),
                runtime: p.manifest.plugin.runtime.clone(),
                enabled: !health.auto_disabled,
                health,
                hooks,
                metrics,
                permissions: p.manifest.permissions.clone(),
            });
        }
        result
    }

    /// 获取单个插件的详细信息
    pub async fn get_plugin_detail(&self, id: &str) -> Option<PluginInfoResponse> {
        let plugins = self.plugins.read().await;
        let p = plugins.get(id)?;
        let health = p.health.read().await.clone();
        let metrics = p.metrics.read().await.clone();
        let hooks: Vec<String> = p.manifest.hooks.keys().cloned().collect();
        Some(PluginInfoResponse {
            id: p.manifest.plugin.id.clone(),
            name: p.manifest.plugin.name.clone(),
            version: p.manifest.plugin.version.clone(),
            description: p.manifest.plugin.description.clone(),
            runtime: p.manifest.plugin.runtime.clone(),
            enabled: !health.auto_disabled,
            health,
            hooks,
            metrics,
            permissions: p.manifest.permissions.clone(),
        })
    }

    /// 启用被自动禁用的插件（重置错误计数）
    pub async fn enable_plugin(&self, id: &str) -> AppResult<()> {
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(id)
            .ok_or_else(|| AppError::not_found("plugin"))?;
        let mut health = plugin.health.write().await;
        health.auto_disabled = false;
        health.error_count = 0;
        health.last_error = None;
        health.last_error_at = None;
        Ok(())
    }

    /// 禁用插件（标记为自动禁用）
    pub async fn disable_plugin(&self, id: &str) -> AppResult<()> {
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(id)
            .ok_or_else(|| AppError::not_found("plugin"))?;
        let mut health = plugin.health.write().await;
        health.auto_disabled = true;
        Ok(())
    }

    /// 记录插件 Hook 错误，达到阈值自动禁用
    async fn record_hook_error(&self, plugin_id: &str, hook: &str, error: &str) {
        let plugins = self.plugins.read().await;
        let Some(plugin) = plugins.get(plugin_id) else {
            return;
        };
        let mut health = plugin.health.write().await;
        health.error_count += 1;
        health.last_error = Some(error.to_string());
        health.last_error_at = Some(crate::utils::tz::now_str());

        let should_disable = health.error_count >= AUTO_DISABLE_THRESHOLD && !health.auto_disabled;
        if should_disable {
            health.auto_disabled = true;
            tracing::warn!(
                "plugin {plugin_id} auto-disabled after {AUTO_DISABLE_THRESHOLD} consecutive errors"
            );
            drop(health);
            drop(plugins);
            self.emit_event(PluginEvent::PluginDisabled {
                id: plugin_id.to_string(),
                reason: format!("auto-disabled after {AUTO_DISABLE_THRESHOLD} errors"),
            });
        } else {
            drop(health);
            drop(plugins);
        }
        self.emit_event(PluginEvent::HookFailed {
            plugin_id: plugin_id.to_string(),
            hook: hook.to_string(),
            error: error.to_string(),
        });
    }

    /// 记录 Hook 执行性能数据
    async fn record_hook_metrics(
        &self,
        plugin_id: &str,
        hook: &str,
        duration_us: u64,
        is_error: bool,
    ) {
        let plugins = self.plugins.read().await;
        let Some(plugin) = plugins.get(plugin_id) else {
            return;
        };
        let mut metrics = plugin.metrics.write().await;
        let m = metrics.entry(hook.to_string()).or_default();
        m.total_calls += 1;
        m.total_duration_us += duration_us;
        if is_error {
            m.total_errors += 1;
        }
    }

    /// 检查插件是否被自动禁用
    async fn is_plugin_enabled(&self, plugin_id: &str) -> bool {
        let plugins = self.plugins.read().await;
        let Some(plugin) = plugins.get(plugin_id) else {
            return false;
        };
        let health = plugin.health.read().await;
        !health.auto_disabled
    }

    /// 重置插件的错误计数（成功调用后）
    async fn reset_error_count(&self, plugin_id: &str) {
        let plugins = self.plugins.read().await;
        let Some(plugin) = plugins.get(plugin_id) else {
            return;
        };
        let mut health = plugin.health.write().await;
        if health.error_count > 0 && !health.auto_disabled {
            health.error_count = 0;
        }
    }

    /// 获取数据库连接池引用（Host API 使用）
    pub fn pool(&self) -> Option<&Pool> {
        self.pool.as_ref()
    }

    /// 调度自定义路由请求。
    ///
    /// 遍历所有插件的 `manifest.routes`，按 method+path 匹配后调用对应的 handler 函数。
    pub async fn dispatch_route(
        &self,
        path: &str,
        method: &str,
        body: Option<&str>,
        headers: Option<&serde_json::Value>,
        auth: Option<&crate::middleware::auth::AuthIdentity>,
    ) -> Option<axum::response::Response> {
        let plugins = self.plugins.read().await;
        if plugins.is_empty() {
            return None;
        }

        for plugin in plugins.values() {
            let plugin_id = plugin.manifest.plugin.id.clone();
            if !self.is_plugin_enabled(&plugin_id).await {
                continue;
            }

            if let Some(route) = Self::match_route(&plugin.manifest.routes, method, path) {
                if let Err(resp) = crate::content_type::schema::check_api_access(route.auth, auth) {
                    let status = match &resp {
                        crate::errors::app_error::AppError::Unauthorized => {
                            axum::http::StatusCode::UNAUTHORIZED
                        }
                        crate::errors::app_error::AppError::Forbidden => {
                            axum::http::StatusCode::FORBIDDEN
                        }
                        _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    };
                    let code = status.as_u16() as i32 * 100;
                    return Some(
                        (
                            status,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            format!(r#"{{"code":{code},"message":"{resp}","data":null}}"#),
                        )
                            .into_response(),
                    );
                }
                let input = serde_json::json!({
                    "path": path,
                    "method": method,
                    "body": body.unwrap_or(""),
                    "headers": headers.unwrap_or(&serde_json::Value::Null),
                });

                let handler = &route.handler;
                let start = std::time::Instant::now();
                let result = self.call_plugin_json(plugin, handler, &input).await;
                let elapsed = start.elapsed().as_micros() as u64;

                match result {
                    Ok(Some(resp)) => {
                        self.reset_error_count(&plugin_id).await;
                        self.record_hook_metrics(&plugin_id, handler, elapsed, false)
                            .await;
                        return Some(resp);
                    }
                    Ok(None) => {
                        self.reset_error_count(&plugin_id).await;
                        self.record_hook_metrics(&plugin_id, handler, elapsed, false)
                            .await;
                        continue;
                    }
                    Err(e) => {
                        let err_msg = format!("{e}");
                        tracing::warn!(
                            "plugin {plugin_id} route handler {handler} failed: {err_msg}"
                        );
                        self.record_hook_error(&plugin_id, handler, &err_msg).await;
                        self.record_hook_metrics(&plugin_id, handler, elapsed, true)
                            .await;
                    }
                }
            }
        }

        None
    }

    /// 在 routes 列表中按 method + path 精确匹配，返回命中的 RouteDef
    fn match_route<'a>(routes: &'a [RouteDef], method: &str, path: &str) -> Option<&'a RouteDef> {
        routes
            .iter()
            .find(|r| r.method.eq_ignore_ascii_case(method) && path_matches_route(path, &r.path))
    }

    /// 调用插件的 JSON filter（统一 WASM/JS/Lua 三引擎），返回解析后的 Response
    async fn call_plugin_json(
        &self,
        plugin: &LoadedPlugin,
        handler: &str,
        input: &serde_json::Value,
    ) -> Result<Option<axum::response::Response>, anyhow::Error> {
        let result: Option<serde_json::Value> = match &plugin.instance {
            #[cfg(feature = "plugin-wasm")]
            LoadedPluginInstance::Wasm(wasm) => {
                let mut instance = wasm.acquire().await;
                let timeout = std::time::Duration::from_millis(instance.timeout_ms());
                tokio::time::timeout(timeout, async {
                    tokio::task::block_in_place(|| {
                        instance.call_json_filter::<serde_json::Value>(handler, input)
                    })
                })
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!(
                        "plugin exceeded wall-clock timeout ({}ms)",
                        timeout.as_millis()
                    ))
                })
                .map_err(|e| anyhow::anyhow!("{e}"))?
            }
            #[cfg(feature = "plugin-js")]
            LoadedPluginInstance::Js(pid) => self
                .js_engine
                .call_filter::<serde_json::Value>(pid, handler, input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            #[cfg(feature = "plugin-lua")]
            LoadedPluginInstance::Lua(pid) => self
                .lua_engine
                .call_filter::<serde_json::Value>(pid, handler, input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        };

        match result {
            Some(result) => {
                if let Some(body) = result.get("body").and_then(|b| b.as_str()) {
                    let status = result
                        .get("status")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(200) as u16;
                    let status_code = axum::http::StatusCode::from_u16(status)
                        .unwrap_or(axum::http::StatusCode::OK);

                    let normalized = if let Ok(mut plugin_resp) =
                        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(body)
                    {
                        if plugin_resp.contains_key("ok") {
                            let is_ok = plugin_resp
                                .get("ok")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let code = if is_ok { 0 } else { status as i32 * 100 };
                            let message = if is_ok {
                                "messages.success".to_string()
                            } else {
                                plugin_resp
                                    .get("error")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("errors.internal")
                                    .to_string()
                            };
                            let data = plugin_resp
                                .remove("data")
                                .unwrap_or(serde_json::Value::Null);
                            serde_json::json!({ "code": code, "message": message, "data": data })
                                .to_string()
                        } else {
                            body.to_string()
                        }
                    } else {
                        body.to_string()
                    };

                    Ok(Some(
                        (
                            status_code,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            normalized,
                        )
                            .into_response(),
                    ))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }
}

/// 路由路径匹配（支持 `:param` 占位段）
fn path_matches_route(path: &str, pattern: &str) -> bool {
    let path_parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let pattern_parts: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();

    if path_parts.len() != pattern_parts.len() {
        return false;
    }

    for (pp, rp) in path_parts.iter().zip(pattern_parts.iter()) {
        if rp.starts_with(':') {
            continue;
        }
        if pp != rp {
            return false;
        }
    }
    true
}

/// 拓扑排序：根据 dependencies 字段确定加载顺序
fn topological_sort(manifests: &HashMap<String, PluginManifest>) -> Vec<String> {
    let mut in_degree: HashMap<&str, u32> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in manifests.keys() {
        in_degree.entry(id.as_str()).or_insert(0);
        dependents.entry(id.as_str()).or_default();
    }

    for (id, manifest) in manifests {
        for dep in manifest.dependencies.keys() {
            if manifests.contains_key(dep.as_str()) {
                *in_degree.entry(id.as_str()).or_insert(0) += 1;
                dependents
                    .entry(dep.as_str())
                    .or_default()
                    .push(id.as_str());
            }
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(&id, _)| id)
        .collect();
    queue.sort_unstable();

    let mut result = Vec::new();
    while let Some(id) = queue.pop() {
        result.push(id.to_string());
        if let Some(deps) = dependents.get(id) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(dep);
                        queue.sort_unstable();
                    }
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::AppConfig;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
    }

    #[cfg(feature = "plugin-wasm")]
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
  (func $echo_lp (param $ptr i32) (param $len i32) (result i32)
    (local $out i32)
    (local $total i32)
    (local.set $total (i32.add (i32.const 4) (local.get $len)))
    (local.set $out (global.get $next_ptr))
    (global.set $next_ptr (i32.add (global.get $next_ptr) (local.get $total)))
    (i32.store (local.get $out) (local.get $len))
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
  (func (export "filter_html") (param $ptr i32) (param $len i32) (result i32)
    (call $echo_lp (local.get $ptr) (local.get $len))
  )
)
"#;
        wat.as_bytes().to_vec()
    }

    // ── path_matches_route tests ────────────────────────────────

    #[test]
    fn route_exact_match() {
        assert!(path_matches_route("/api/v1/posts", "/api/v1/posts"));
    }

    #[test]
    fn route_no_match() {
        assert!(!path_matches_route("/api/v1/posts", "/api/v1/users"));
    }

    #[test]
    fn route_param_match() {
        assert!(path_matches_route(
            "/api/v1/plugins/ecommerce/products/abc-123",
            "/api/v1/plugins/ecommerce/products/:id"
        ));
    }

    #[test]
    fn route_different_length_no_match() {
        assert!(!path_matches_route(
            "/api/v1/plugins/seo/a/b",
            "/api/v1/plugins/seo/:id"
        ));
    }

    #[test]
    fn route_empty_both() {
        assert!(path_matches_route("", ""));
    }

    #[test]
    fn route_trailing_slash_normalized() {
        assert!(path_matches_route("/api/v1/posts/", "/api/v1/posts"));
    }

    #[test]
    fn route_root_match() {
        assert!(path_matches_route("/", "/"));
    }

    #[test]
    fn route_trailing_segment_mismatch() {
        assert!(!path_matches_route(
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
        assert!(
            mgr.dispatch_route("/api/v1/test", "GET", None, None, None)
                .await
                .is_none()
        );
    }

    // ── WASM plugin tests ────────────────────────────────────────

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_load_wasm_plugin_from_directory() {
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

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_skip_disabled_wasm_plugin() {
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

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_skip_directory_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("incomplete-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-wasm")]
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

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_unload_wasm_plugin() {
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

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_reload_wasm_plugin() {
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

        mgr.reload_plugin(&plugin_dir).await;
        assert_eq!(mgr.plugin_count().await, 1);
    }

    #[cfg(feature = "plugin-wasm")]
    #[tokio::test]
    async fn manager_load_multiple_wasm_plugins() {
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

    // ── JS plugin tests ──────────────────────────────────────────

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_load_js_plugin_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-plugin"
name = "JS Test"
version = "1.0.0"
runtime = "js"

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"var Plugin = { on_post_creating: function(j) { return j; } };"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        let plugins = mgr.list_plugins().await;
        assert_eq!(plugins[0].0, "com.test.js-plugin");
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-filter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-filter"
name = "JS Filter"
version = "1.0.0"
runtime = "js"

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.title = input.title.toUpperCase();
        return JSON.stringify(input);
    }
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["title"], "HELLO");
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_string_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-strfilter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-strfilter"
name = "JS String Filter"
version = "1.0.0"
runtime = "js"

[hooks.render_markdown]
priority = 5
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
var Plugin = {
    render_markdown: function(content) {
        return content.replace("<head>", '<head><meta property="og:type" content="article">');
    }
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_render_override("<head><title>Test</title></head>")
            .await;
        assert!(result.is_some());
        assert!(result.unwrap().contains("og:type"));
    }

    // ── Mixed WASM + JS plugin tests ─────────────────────────────

    #[cfg(all(feature = "plugin-wasm", feature = "plugin-js"))]
    #[tokio::test]
    async fn manager_load_mixed_plugins() {
        let dir = tempfile::tempdir().unwrap();

        // WASM plugin
        let wasm_dir = dir.path().join("wasm-plugin");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        std::fs::write(
            wasm_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.wasm\"\nname=\"WASM\"\nversion=\"1.0.0\"\nruntime=\"wasm\"",
        )
        .unwrap();
        std::fs::write(wasm_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        // JS plugin
        let js_dir = dir.path().join("js-plugin");
        std::fs::create_dir_all(&js_dir).unwrap();
        std::fs::write(
            js_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.js\"\nname=\"JS\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        std::fs::write(
            js_dir.join("index.js"),
            r#"var Plugin = { on_post_creating: function(j) { return j; } };"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 2);
    }

    // ── JS plugin advanced tests ─────────────────────────────────

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_action_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-action-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-action"
name = "JS Action"
version = "1.0.0"
runtime = "js"

[hooks.on_post_created]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
var Plugin = {
    on_post_created: function(dataJson) {
        Host.log("info", "post created");
    }
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.dispatch_action(HookPoint::PostCreated, &serde_json::json!({"id": "abc"}))
            .await;
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_route_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-route-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-route"
name = "JS Route"
version = "1.0.0"
runtime = "js"

[[routes]]
method = "GET"
path = "/api/v1/custom/test"
handler = "handle_test"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
var Plugin = {};
Plugin.handle_test = function(input) {
    return JSON.stringify({ status: 200, body: '{"hello":"world"}' });
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_route("/api/v1/custom/test", "GET", None, None, None)
            .await;
        assert!(result.is_some());
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-unload");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.js-unload\"\nname=\"JSU\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.js"), r#"var Plugin = {};"#).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        mgr.unload_plugin("com.test.js-unload").await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_skip_directory_without_entry() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-noentry");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.noentry\"\nname=\"NE\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        // no index.js

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_js_plugin_get_config() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("js-config-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.js-config"
name = "JS Config"
version = "1.0.0"
runtime = "js"

[permissions]
config = ["app.*"]

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        var env = Host.getConfig("app.env");
        if (env) {
            input.env = env;
        }
        return JSON.stringify(input);
    }
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["env"], "test");
    }

    #[cfg(all(feature = "plugin-wasm", feature = "plugin-js"))]
    #[tokio::test(flavor = "multi_thread")]
    async fn manager_mixed_wasm_js_filter_chain() {
        let dir = tempfile::tempdir().unwrap();

        let wasm_dir = dir.path().join("wasm-chain");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        std::fs::write(
            wasm_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.chain.wasm\"\nname=\"WC\"\nversion=\"1.0.0\"\nruntime=\"wasm\"\n\n[hooks.on_post_creating]\npriority=10",
        )
        .unwrap();
        std::fs::write(wasm_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let js_dir = dir.path().join("js-chain");
        std::fs::create_dir_all(&js_dir).unwrap();
        std::fs::write(
            js_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.chain.js\"\nname=\"JC\"\nversion=\"1.0.0\"\nruntime=\"js\"\n\n[hooks.on_post_creating]\npriority=20",
        )
        .unwrap();
        std::fs::write(
            js_dir.join("index.js"),
            r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.js_processed = true;
        return JSON.stringify(input);
    }
};
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 2);

        let input = serde_json::json!({"title": "chain-test"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();

        assert_eq!(result["js_processed"], true);
    }

    // ── Lua plugin tests ─────────────────────────────────────────

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_load_lua_plugin_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-plugin"
name = "Lua Test"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[hooks.on_post_creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"Plugin = { on_post_creating = function(input) return input end }"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        let plugins = mgr.list_plugins().await;
        assert_eq!(plugins[0].0, "com.test.lua-plugin");
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-filter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-filter"
name = "Lua Filter"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[hooks.on_post_creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_creating = function(input)
        input.title = input.title:upper()
        return input
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["title"], "HELLO");
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_string_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-strfilter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-strfilter"
name = "Lua String Filter"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[hooks.render_markdown]
priority = 5
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    render_markdown = function(html)
        return html:gsub("<head>", '<head><meta property="og:type" content="article">')
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_render_override("<head><title>Test</title></head>")
            .await;
        assert!(result.is_some());
        assert!(result.unwrap().contains("og:type"));
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_action_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-action-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-action"
name = "Lua Action"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[hooks.on_post_created]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_created = function(data)
        Host.log("info", "post created")
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.dispatch_action(HookPoint::PostCreated, &serde_json::json!({"id": "abc"}))
            .await;
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-unload");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.lua-unload\"\nname=\"LU\"\nversion=\"1.0.0\"\nruntime=\"lua\"\nentry=\"init.lua\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("init.lua"), "Plugin = {}").unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        mgr.unload_plugin("com.test.lua-unload").await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_skip_directory_without_entry() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-noentry");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.noentry\"\nname=\"NE\"\nversion=\"1.0.0\"\nruntime=\"lua\"\nentry=\"init.lua\"",
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_get_config() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-config-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-config"
name = "Lua Config"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[permissions]
config = ["app.*"]

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_creating = function(input)
        local env = Host.getConfig("app.env")
        if env then
            input.env = env
        end
        return input
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["env"], "test");
    }

    #[cfg(all(feature = "plugin-wasm", feature = "plugin-js", feature = "plugin-lua"))]
    #[tokio::test(flavor = "multi_thread")]
    async fn manager_triple_engine_filter_chain() {
        let dir = tempfile::tempdir().unwrap();

        let wasm_dir = dir.path().join("wasm-chain");
        std::fs::create_dir_all(&wasm_dir).unwrap();
        std::fs::write(
            wasm_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.chain.wasm\"\nname=\"WC\"\nversion=\"1.0.0\"\nruntime=\"wasm\"\n\n[hooks.on_post_creating]\npriority=10",
        )
        .unwrap();
        std::fs::write(wasm_dir.join("plugin.wasm"), build_test_wasm()).unwrap();

        let js_dir = dir.path().join("js-chain");
        std::fs::create_dir_all(&js_dir).unwrap();
        std::fs::write(
            js_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.chain.js\"\nname=\"JC\"\nversion=\"1.0.0\"\nruntime=\"js\"\n\n[hooks.on_post_creating]\npriority=20",
        )
        .unwrap();
        std::fs::write(
            js_dir.join("index.js"),
            r#"
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        input.js_processed = true;
        return JSON.stringify(input);
    }
};
"#,
        )
        .unwrap();

        let lua_dir = dir.path().join("lua-chain");
        std::fs::create_dir_all(&lua_dir).unwrap();
        std::fs::write(
            lua_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.chain.lua\"\nname=\"LC\"\nversion=\"1.0.0\"\nruntime=\"lua\"\nentry=\"init.lua\"\n\n[hooks.on_post_creating]\npriority=30",
        )
        .unwrap();
        std::fs::write(
            lua_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_creating = function(input)
        input.lua_processed = true
        return input
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 3);

        let input = serde_json::json!({"title": "chain-test"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input)
            .await
            .unwrap();

        assert_eq!(result["js_processed"], true);
        assert_eq!(result["lua_processed"], true);
    }

    // ── EventBus tests ──────────────────────────────────────────

    #[tokio::test]
    async fn event_bus_subscribe_and_receive() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        let mut rx = mgr.subscribe_events();

        mgr.emit_event(PluginEvent::PluginLoaded {
            id: "test".into(),
            name: "Test".into(),
        });

        let event = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();
        match &*event {
            PluginEvent::PluginLoaded { id, name } => {
                assert_eq!(id, "test");
                assert_eq!(name, "Test");
            }
            _ => panic!("unexpected event"),
        }
    }

    // ── Health & Metrics tests ──────────────────────────────────

    #[tokio::test]
    async fn health_default_is_healthy() {
        let h = PluginHealth::default();
        assert_eq!(h.error_count, 0);
        assert!(!h.auto_disabled);
        assert!(h.last_error.is_none());
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_list_plugins_detail_basic() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("detail-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.detail"
name = "Detail"
version = "1.0.0"
runtime = "js"
description = "test plugin"

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"var Plugin = { on_post_creating: function(j) { return j; } };"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let details = mgr.list_plugins_detail().await;
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].id, "com.test.detail");
        assert!(details[0].enabled);
        assert!(details[0].hooks.contains(&"on_post_creating".to_string()));
    }

    #[cfg(feature = "plugin-js")]
    #[tokio::test]
    async fn manager_enable_disable_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("toggle-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("plugin.toml"),
            "[plugin]\nid=\"com.test.toggle\"\nname=\"T\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.js"), r#"var Plugin = {};"#).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.disable_plugin("com.test.toggle").await.unwrap();
        let detail = mgr.get_plugin_detail("com.test.toggle").await.unwrap();
        assert!(!detail.enabled);

        mgr.enable_plugin("com.test.toggle").await.unwrap();
        let detail = mgr.get_plugin_detail("com.test.toggle").await.unwrap();
        assert!(detail.enabled);
    }

    #[tokio::test]
    async fn manager_enable_nonexistent_returns_not_found() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        let result = mgr.enable_plugin("nonexistent").await;
        assert!(result.is_err());
    }

    // ── topological_sort tests ──────────────────────────────────

    #[test]
    fn topo_sort_no_deps() {
        let m1 = make_test_manifest("a", vec![]);
        let m2 = make_test_manifest("b", vec![]);
        let manifests = HashMap::from([("a".into(), m1), ("b".into(), m2)]);
        let order = topological_sort(&manifests);
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn topo_sort_with_deps() {
        let m1 = make_test_manifest("a", vec![("b", "1.0")]);
        let m2 = make_test_manifest("b", vec![]);
        let manifests = HashMap::from([("a".into(), m1), ("b".into(), m2)]);
        let order = topological_sort(&manifests);
        assert_eq!(order.len(), 2);
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        assert!(b_pos < a_pos);
    }

    #[test]
    fn topo_sort_missing_dep_ignored() {
        let m1 = make_test_manifest("a", vec![("missing", "1.0")]);
        let manifests = HashMap::from([("a".into(), m1)]);
        let order = topological_sort(&manifests);
        assert_eq!(order, vec!["a"]);
    }

    fn make_test_manifest(id: &str, deps: Vec<(&str, &str)>) -> PluginManifest {
        PluginManifest {
            plugin: manifest::PluginInfo {
                id: id.into(),
                name: id.into(),
                version: "1.0.0".into(),
                description: String::new(),
                author: None,
                license: None,
                runtime: "js".into(),
                language: "js".into(),
                wasm: "plugin.wasm".into(),
                entry: "index.js".into(),
            },
            permissions: Permissions::default(),
            hooks: HashMap::new(),
            dependencies: deps
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cron: vec![],
            content_types: vec![],
            routes: vec![],
            admin_pages: vec![],
        }
    }

    // ── VFS 集成测试 ──────────────────────────────────────────────

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_vfs_full_integration() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-vfs-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-vfs"
name = "VFS Test"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[permissions]
filesystem = ["read-write"]

[hooks.on_post_creating]
priority = 10

[hooks.on_post_created]
priority = 10

[hooks.on_post_deleted]
priority = 10
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let lua_code = r#"
Plugin = {
    on_post_creating = function(input)
        local slug = input.slug or ""
        if slug ~= "" then
            local exists = Host.fsExists("cache/" .. slug .. ".txt")
            if exists then
                input.cache_hit = true
            end
            local stat = Host.fsRead("stats.json")
            if stat then
                input.stats = stat
            end
        end
        return input
    end,

    on_post_created = function(input)
        local slug = input.slug or ""
        local title = input.title or ""
        if slug ~= "" then
            Host.fsWrite("cache/" .. slug .. ".txt", title .. "|" .. (input.content or ""))
            Host.fsWrite("stats.json", '{"writes":1}')

            local info = Host.fsStat("cache/" .. slug .. ".txt")
            if info then
                input.file_stat = info
            end

            local entries = Host.fsList("cache")
            if entries then
                input.cache_files = table.concat(entries, ",")
            end
        end
        return input
    end,

    on_post_deleted = function(input)
        local slug = input.slug or ""
        if slug ~= "" then
            Host.fsDelete("cache/" .. slug .. ".txt")
            local entries = Host.fsList("cache")
            if entries then
                input.remaining = table.concat(entries, ",")
            end
        end
        return input
    end,
}
"#;
        std::fs::write(plugin_dir.join("init.lua"), lua_code).unwrap();

        let mut config = (*test_config()).clone();
        let vfs_root = dir.path().join("vfs-root");
        std::fs::create_dir_all(&vfs_root).unwrap();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        config.plugin_vfs_root = vfs_root.to_string_lossy().to_string();
        config.plugin_vfs_max_file_size = 65536;
        config.plugin_vfs_max_total_size = 1048576;
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({
            "slug": "hello-world",
            "title": "Hello",
            "content": "world"
        });

        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result["cache_hit"], serde_json::Value::Null);
        assert!(result["stats"].is_null());

        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreated, input.clone())
            .await
            .unwrap();
        assert!(result["file_stat"].is_string());
        assert!(result["cache_files"].is_string());

        let vfs_plugin_dir = vfs_root.join("com.test.lua-vfs");
        let cache_file = vfs_plugin_dir.join("cache/hello-world.txt");
        assert!(cache_file.exists());
        let content = std::fs::read_to_string(&cache_file).unwrap();
        assert_eq!(content, "Hello|world");

        let stats_file = vfs_plugin_dir.join("stats.json");
        assert!(stats_file.exists());

        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result["cache_hit"], true);
        assert!(result["stats"].is_string());

        let delete_input = serde_json::json!({"slug": "hello-world"});
        let _result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostDeleted, delete_input)
            .await
            .unwrap();
        assert!(!cache_file.exists());

        let check_input = serde_json::json!({"slug": "hello-world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(HookPoint::PostCreating, check_input)
            .await
            .unwrap();
        assert!(result["cache_hit"].is_null());
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_routes() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-route-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.lua-route"
name = "Lua Route"
version = "1.0.0"
runtime = "lua"
entry = "init.lua"

[permissions]
database = ["read:posts"]

[[routes]]
method = "GET"
path = "/api/v1/plugins/stats/ping"
handler = "ping"

[[routes]]
method = "GET"
path = "/api/v1/plugins/stats/count"
handler = "count"
"#;
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let lua_code = r#"
Plugin = {}
Plugin.ping = function(input)
    return {
        status = 200,
        body = '{"code":0,"message":"ok","data":"pong"}'
    }
end

Plugin.count = function(input)
    local result = Host.dbQuery("SELECT COUNT(*) as total FROM posts")
    if result and result:sub(1, 6) ~= "error:" then
        return {
            status = 200,
            body = '{"code":0,"message":"ok","data":' .. result .. '}'
        }
    end
    return {
        status = 500,
        body = '{"code":50000,"message":"query failed","data":null}'
    }
end
"#;
        std::fs::write(plugin_dir.join("init.lua"), lua_code).unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_route("/api/v1/plugins/stats/ping", "GET", None, None, None)
            .await;
        assert!(result.is_some(), "should match declared route");

        let result = mgr
            .dispatch_route("/api/v1/plugins/stats/unknown", "GET", None, None, None)
            .await;
        assert!(result.is_none(), "should return none for undeclared path");

        let result = mgr
            .dispatch_route("/api/v1/plugins/stats/ping", "POST", None, None, None)
            .await;
        assert!(result.is_none(), "should not match wrong method");
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test(flavor = "multi_thread")]
    async fn lua_cron_plugin_syncs_schedules() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("cron-test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = concat!(
            "[plugin]\n",
            "id = \"com.test.cron-plugin\"\n",
            "name = \"Cron Test\"\n",
            "version = \"1.0.0\"\n",
            "runtime = \"lua\"\n",
            "language = \"lua\"\n",
            "entry = \"init.lua\"\n",
            "\n",
            "[hooks.on-cron-tick]\n",
            "priority = 10\n",
            "\n",
            "[[cron]]\n",
            "label = \"Cleanup\"\n",
            "job_type = \"cleanup_sessions\"\n",
            "payload = '{\"max_age_hours\": 12}'\n",
            "cron_expr = \"0 0 */6 * * *\"\n",
            "enabled = true\n",
            "\n",
            "[[cron]]\n",
            "label = \"Digest\"\n",
            "job_type = \"daily_digest\"\n",
            "cron_expr = \"0 0 3 * * *\"\n",
            "enabled = false\n",
        );
        std::fs::write(plugin_dir.join("plugin.toml"), manifest).unwrap();

        let lua_code = "Plugin = { on_cron_tick = function(data) Host.setData(\"last_job\", data.job_type or \"\") end }";
        std::fs::write(plugin_dir.join("init.lua"), lua_code).unwrap();

        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../migrations/007_cron_schedules.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let mut config = crate::config::app::AppConfig::test_defaults();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let config = Arc::new(config);

        let mgr = PluginManager::new_with_options(
            config,
            PluginManagerOptions {
                pool: Some(pool.clone()),
                event_bus: None,
            },
        )
        .await;

        let schedules = crate::worker::list_schedules(&pool).await.unwrap();
        assert_eq!(schedules.len(), 2);

        let cleanup = schedules
            .iter()
            .find(|s| s.job_type == "cleanup_sessions")
            .unwrap();
        assert_eq!(cleanup.label, "Cleanup");
        assert!(cleanup.enabled);
        assert_eq!(cleanup.plugin_id, Some("com.test.cron-plugin".into()));

        let digest = schedules
            .iter()
            .find(|s| s.job_type == "daily_digest")
            .unwrap();
        assert_eq!(digest.label, "Digest");
        assert!(!digest.enabled);

        mgr.dispatch_action(
            HookPoint::CronTick,
            &serde_json::json!({
                "job_type": "cleanup_sessions",
                "payload": {"max_age_hours": 12},
                "timestamp": "2026-01-01T00:00:00Z"
            }),
        )
        .await;

        mgr.unload_plugin("com.test.cron-plugin").await;

        let after_unload = crate::worker::list_schedules(&pool).await.unwrap();
        assert!(
            after_unload.is_empty(),
            "cron schedules should be removed after plugin unload"
        );
    }
}
