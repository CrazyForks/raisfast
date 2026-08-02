//! Plugin system
//!
//! Supports four runtimes: WASM (wasmtime), JavaScript (QuickJS/rquickjs), Lua (mlua), Rhai.
//! Controlled via feature flags `plugin-wasm` / `plugin-js` / `plugin-lua` / `plugin-rhai`.
//! Plugins interact with the host through hook points and run in sandboxes.
//!
//! # New Capabilities (v2)
//!
//! - **Host API extensions**: `httpGet`, `httpPost`, `getPost`, `getData`, `setData`, `dbQuery`
//! - **Permission enforcement**: permissions declared in manifest are enforced at runtime
//! - **Lifecycle hooks**: `on_load` / `on_unload` callbacks
//! - **Error recovery**: plugins are auto-disabled after reaching consecutive error threshold
//! - **`EventBus`**: event-driven architecture; plugins can subscribe to internal events
//! - **Performance metrics**: per-plugin hook execution time and error count tracking
//! - **Dependency management**: manifest can declare plugin dependencies, checked at load time
//! - **Management API**: enable/disable/reload plugins at runtime

#[cfg(feature = "plugin-wasm")]
mod bindings;
#[cfg(feature = "plugin-wasm")]
mod engine;
#[cfg(feature = "plugin-js")]
mod engine_js;
#[cfg(feature = "plugin-lua")]
mod engine_lua;
#[cfg(feature = "plugin-rhai")]
mod engine_rhai;
#[cfg(feature = "plugin-wasm")]
mod host;
pub mod host_common;
#[cfg(feature = "plugin-js")]
mod js_host;
#[cfg(feature = "plugin-lua")]
mod lua_host;
#[cfg(feature = "plugin-rhai")]
mod rhai_host;

pub mod http_client;
mod manifest;
pub mod permissions;
pub mod sdk_v1;
pub mod vfs;

pub use manifest::{CronEntry, HookConfig, Permissions, PluginManifest, RouteDef};
pub use permissions::PermissionChecker;

#[cfg(feature = "export-types")]
export_types!(
    PluginHealth,
    PluginMetrics,
    PluginEvent,
    PluginInfoResponse,
    Permissions,
);

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::response::IntoResponse;
use dashmap::DashMap;
use notify::Watcher;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::RwLock;
#[cfg(feature = "export-types")]
use ts_rs::TS;

#[cfg(feature = "plugin-wasm")]
use engine::WasmInstancePool;
#[cfg(feature = "plugin-js")]
use engine_js::JsEngine;
#[cfg(feature = "plugin-lua")]
use engine_lua::LuaEngine;
#[cfg(feature = "plugin-rhai")]
use engine_rhai::RhaiEngine;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};

/// Convert sqlx arbitrary query result rows to a JSON array string
///
/// Works with the currently compiled database backend via the `DbRow` type alias.
pub(crate) fn rows_to_json(rows: &[crate::db::DbRow]) -> String {
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
        .map(|row: &crate::db::DbRow| {
            let mut map = serde_json::Map::new();
            for (i, col) in columns.iter().enumerate() {
                let i: usize = i;
                if let Ok(v) = row.try_get::<Option<i64>, usize>(i) {
                    map.insert(
                        col.clone(),
                        v.map_or(serde_json::Value::Null, |n: i64| {
                            serde_json::Value::Number(n.into())
                        }),
                    );
                } else if let Ok(v) = row.try_get::<Option<f64>, usize>(i) {
                    map.insert(
                        col.clone(),
                        v.and_then(|f: f64| {
                            serde_json::Number::from_f64(f).map(serde_json::Value::Number)
                        })
                        .unwrap_or(serde_json::Value::Null),
                    );
                } else if let Ok(v) = row.try_get::<Option<bool>, usize>(i) {
                    map.insert(
                        col.clone(),
                        v.map_or(serde_json::Value::Null, |b: bool| {
                            serde_json::Value::Bool(b)
                        }),
                    );
                } else if let Ok(v) = row.try_get::<Option<String>, usize>(i) {
                    map.insert(
                        col.clone(),
                        v.map_or(serde_json::Value::Null, |s: String| {
                            serde_json::Value::String(s)
                        }),
                    );
                } else {
                    map.insert(col.clone(), serde_json::Value::Null);
                }
            }
            map
        })
        .collect();
    serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string())
}

/// Auto-disable plugin after this many consecutive errors
const AUTO_DISABLE_THRESHOLD: u32 = 5;

/// Loaded plugin instance (WASM, JS, or Lua)
enum LoadedPluginInstance {
    #[cfg(feature = "plugin-wasm")]
    Wasm(Arc<WasmInstancePool>),
    #[cfg(feature = "plugin-js")]
    Js(String),
    #[cfg(feature = "plugin-lua")]
    Lua(String),
    #[cfg(feature = "plugin-rhai")]
    Rhai(String),
}

/// Plugin health status and performance metrics
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Default)]
pub struct PluginHealth {
    pub error_count: u32,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
    pub auto_disabled: bool,
}

/// Plugin hook execution performance metrics
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Default, Serialize)]
pub struct PluginMetrics {
    pub total_calls: u64,
    pub total_errors: u64,
    pub total_duration_us: u64,
}

/// Loaded plugin
struct LoadedPlugin {
    manifest: PluginManifest,
    instance: LoadedPluginInstance,
    health: RwLock<PluginHealth>,
    metrics: RwLock<HashMap<String, PluginMetrics>>,
}

/// Plugin system core manager
///
/// Responsible for plugin loading, unloading, and hook dispatch.
/// Shared in `AppState` via `Arc<PluginManager>`.
pub struct PluginManager {
    #[cfg(feature = "plugin-wasm")]
    engine: wasmtime::Engine,
    #[cfg(feature = "plugin-js")]
    js_engine: JsEngine,
    #[cfg(feature = "plugin-lua")]
    lua_engine: LuaEngine,
    #[cfg(feature = "plugin-rhai")]
    rhai_engine: RhaiEngine,
    plugins: RwLock<HashMap<String, LoadedPlugin>>,
    route_index: DashMap<String, Vec<RouteIndexEntry>>,
    hook_index: DashMap<String, Vec<HookIndexEntry>>,
    config: Arc<AppConfig>,
    pool: Option<Pool>,
    watcher: RwLock<Option<notify::RecommendedWatcher>>,
    reload_tx: tokio::sync::mpsc::Sender<PathBuf>,
    reload_rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<PathBuf>>>,
    event_bus: tokio::sync::broadcast::Sender<Arc<PluginEvent>>,
}

#[derive(Clone)]
struct RouteIndexEntry {
    plugin_id: String,
    method: String,
    pattern: String,
    handler: String,
    auth: crate::content_type::schema::ApiAccess,
    segment_count: usize,
    static_segments: Vec<(usize, String)>,
}

#[derive(Clone)]
struct HookIndexEntry {
    plugin_id: String,
    priority: i32,
    content_types: Vec<String>,
}

/// Plugin system internal events
#[cfg_attr(feature = "export-types", derive(TS))]
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

/// Plugin list response item (used by management API)
#[cfg_attr(feature = "export-types", derive(TS))]
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

/// Optional dependencies for setting up `PluginManager`
pub struct PluginManagerOptions {
    pub pool: Option<Pool>,
    pub event_bus: Option<crate::eventbus::EventBus>,
}

impl PluginManager {
    /// Create a new `PluginManager` and load the plugin directory, returning `Arc<Self>`.
    ///
    /// Returns `Arc` because the hot-reload watcher needs a self-reference to perform reloads.
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

    /// Create a `PluginManager` with optional dependencies
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

    /// Create an empty `PluginManager` (no directory scan); call load_plugin manually
    pub async fn new_empty(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self> {
        Self::build_instance(config, opts).await
    }

    /// Build PluginManager instance (engine initialization, no plugin loading)
    async fn build_instance(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self> {
        #[cfg(feature = "plugin-wasm")]
        let engine = {
            let mut engine_config = wasmtime::Config::new();
            engine_config.consume_fuel(true);
            engine_config.wasm_component_model(true);
            engine_config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
            wasmtime::Engine::new(&engine_config)
                .unwrap_or_else(|e| panic!("failed to create wasmtime engine: {e}"))
        };

        #[cfg(feature = "plugin-js")]
        let js_engine = JsEngine::new(&config, opts.pool.clone(), opts.event_bus.clone())
            .await
            .unwrap_or_else(|e| panic!("failed to create js engine: {e}"));

        #[cfg(feature = "plugin-lua")]
        let lua_engine = LuaEngine::new(&config, opts.pool.clone(), opts.event_bus.clone())
            .unwrap_or_else(|e| panic!("failed to create lua engine: {e}"));

        #[cfg(feature = "plugin-rhai")]
        let rhai_engine = RhaiEngine::new(&config, opts.pool.clone(), opts.event_bus.clone())
            .unwrap_or_else(|e| panic!("failed to create rhai engine: {e}"));

        let (reload_tx, reload_rx) = tokio::sync::mpsc::channel::<PathBuf>(32);
        let (event_tx, _) = tokio::sync::broadcast::channel::<Arc<PluginEvent>>(256);

        Arc::new(Self {
            #[cfg(feature = "plugin-wasm")]
            engine,
            #[cfg(feature = "plugin-js")]
            js_engine,
            #[cfg(feature = "plugin-lua")]
            lua_engine,
            #[cfg(feature = "plugin-rhai")]
            rhai_engine,
            plugins: RwLock::new(HashMap::new()),
            route_index: DashMap::new(),
            hook_index: DashMap::new(),
            config,
            pool: opts.pool,
            watcher: RwLock::new(None),
            reload_tx,
            reload_rx: tokio::sync::Mutex::new(Some(reload_rx)),
            event_bus: event_tx,
        })
    }

    /// Set the database connection pool (if not provided at creation)
    pub fn set_pool(&mut self, pool: Pool) {
        self.pool = Some(pool);
    }

    /// Subscribe to plugin system events
    pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<Arc<PluginEvent>> {
        self.event_bus.subscribe()
    }

    /// Emit a plugin system event
    fn emit_event(&self, event: PluginEvent) {
        let _ = self.event_bus.send(Arc::new(event));
    }

    /// Scan the plugin directory and load all valid plugins
    pub async fn load_all(&self) {
        let plugin_dir = match &self.config.plugin_dir {
            Some(d) => d,
            None => return,
        };

        let plugin_dir = Path::new(plugin_dir);
        if !plugin_dir.exists() {
            let _ = std::fs::create_dir_all(plugin_dir);
            tracing::info!("created plugin directory: {}", plugin_dir.display());
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

            let manifest_path = entry.path().join("manifest.toml");
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

    /// Load plugin from the directory containing manifest.toml, selecting engine by runtime field
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
            #[cfg(feature = "plugin-rhai")]
            "rhai" => {
                let entry_file = if manifest.plugin.entry == "index.js" {
                    "init.rhai"
                } else {
                    &manifest.plugin.entry
                };
                let entry_path = dir.join(entry_file);
                if !entry_path.exists() {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "rhai entry file not found: {}",
                        entry_path.display()
                    )));
                }
                self.load_rhai_plugin(manifest, &entry_path).await
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

    /// Load a WASM plugin
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
        self.rebuild_route_index();
        self.rebuild_hook_index();

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// Load a JS plugin
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

        let sdk_source =
            sdk_v1::get_sdk_source("js", &manifest.plugin.sdk_version).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "unknown SDK version: js/{}",
                    manifest.plugin.sdk_version
                ))
            })?;

        let plugin_dir = entry_path
            .parent()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("entry has no parent directory")))?;

        self.js_engine
            .load_plugin(
                &id,
                &code,
                manifest.permissions.clone(),
                plugin_dir,
                sdk_source,
            )
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
        self.rebuild_route_index();
        self.rebuild_hook_index();

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// Load a Lua plugin
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

        let sdk_source =
            sdk_v1::get_sdk_source("lua", &manifest.plugin.sdk_version).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "unknown SDK version: lua/{}",
                    manifest.plugin.sdk_version
                ))
            })?;

        let plugin_dir = entry_path
            .parent()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("entry has no parent directory")))?;

        let permissions = manifest.permissions.clone();
        self.lua_engine
            .load_plugin(&id, &code, permissions, plugin_dir, sdk_source)
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
        self.rebuild_route_index();
        self.rebuild_hook_index();

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// Load a Rhai plugin
    #[cfg(feature = "plugin-rhai")]
    async fn load_rhai_plugin(
        &self,
        manifest: PluginManifest,
        entry_path: &Path,
    ) -> AppResult<String> {
        let id = manifest.plugin.id.clone();
        let name = manifest.plugin.name.clone();

        let code = std::fs::read_to_string(entry_path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read rhai entry: {e}")))?;

        let plugin_dir = entry_path
            .parent()
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("entry has no parent directory")))?;

        let permissions = manifest.permissions.clone();
        self.rhai_engine
            .load_plugin(&id, &code, permissions, plugin_dir, "")
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("load rhai plugin: {e}")))?;

        let mut plugins = self.plugins.write().await;
        plugins.insert(
            id.clone(),
            LoadedPlugin {
                manifest,
                instance: LoadedPluginInstance::Rhai(id.clone()),
                health: RwLock::new(PluginHealth::default()),
                metrics: RwLock::new(HashMap::new()),
            },
        );
        let cron_entries = plugins
            .get(&id)
            .map(|p| p.manifest.cron.clone())
            .unwrap_or_default();
        drop(plugins);
        self.rebuild_route_index();
        self.rebuild_hook_index();

        self.sync_crons_for_plugin(&id, &cron_entries).await;

        self.emit_event(PluginEvent::PluginLoaded {
            id: id.clone(),
            name,
        });
        Ok(id)
    }

    /// Unload the specified plugin
    /// Rebuild route index from all loaded plugins.
    fn rebuild_route_index(&self) {
        let plugins = match self.plugins.try_read() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        self.route_index.clear();
        for (plugin_id, plugin) in plugins.iter() {
            for route in &plugin.manifest.routes {
                let key = route_index_key(&route.path);
                let pattern_parts: Vec<&str> =
                    route.path.trim_end_matches('/').split('/').collect();
                let static_segments: Vec<(usize, String)> = pattern_parts
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.starts_with(':'))
                    .map(|(i, s)| (i, s.to_string()))
                    .collect();
                let entry = RouteIndexEntry {
                    plugin_id: plugin_id.clone(),
                    method: route.method.to_uppercase(),
                    pattern: route.path.clone(),
                    handler: route.handler.clone(),
                    auth: route.auth,
                    segment_count: pattern_parts.len(),
                    static_segments,
                };
                self.route_index.entry(key).or_default().push(entry);
            }
        }
    }

    fn rebuild_hook_index(&self) {
        let plugins = match self.plugins.try_read() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        self.hook_index.clear();
        for (plugin_id, plugin) in plugins.iter() {
            for (func_name, hook_cfg) in &plugin.manifest.hooks {
                let entry = HookIndexEntry {
                    plugin_id: plugin_id.clone(),
                    priority: hook_cfg.priority.unwrap_or(100),
                    content_types: hook_cfg.content_types.clone(),
                };
                self.hook_index
                    .entry(func_name.clone())
                    .or_default()
                    .push(entry);
            }
        }
        for mut entry in self.hook_index.iter_mut() {
            entry.value_mut().sort_by_key(|e| e.priority);
        }
    }

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
                #[cfg(feature = "plugin-rhai")]
                LoadedPluginInstance::Rhai(_) => {
                    drop(removed);
                    self.rhai_engine.unload_plugin(id).await;
                }
                #[cfg(feature = "plugin-wasm")]
                LoadedPluginInstance::Wasm(_) => {}
                #[allow(unreachable_patterns)]
                _ => {}
            }
            tracing::info!("unloaded plugin: {id}");
            drop(plugins);
            self.rebuild_route_index();
            self.rebuild_hook_index();
            self.rebuild_hook_index();
            self.remove_crons_for_plugin(id).await;
            self.emit_event(PluginEvent::PluginUnloaded { id: id.to_string() });
        }
    }

    /// Sync plugin Cron schedules to the database
    async fn sync_crons_for_plugin(&self, plugin_id: &str, entries: &[CronEntry]) {
        if let Some(ref pool) = self.pool
            && let Err(e) = crate::worker::sync_plugin_crons(pool, plugin_id, entries).await
        {
            tracing::warn!("failed to sync crons for plugin {plugin_id}: {e}");
        }
    }

    /// Remove Cron schedules associated with a plugin
    async fn remove_crons_for_plugin(&self, plugin_id: &str) {
        if let Some(ref pool) = self.pool
            && let Err(e) = crate::worker::remove_plugin_crons(pool, plugin_id).await
        {
            tracing::warn!("failed to remove crons for plugin {plugin_id}: {e}");
        }
    }

    /// Reload a specified plugin (hot update)
    pub async fn reload_plugin(&self, plugin_dir: &Path) {
        let manifest_path = plugin_dir.join("manifest.toml");
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

    /// Start the file watcher.
    ///
    /// Detects `.wasm` / `.js` file changes and notifies the reload task via channel.
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
        let mut watcher = match notify::RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                let event = match res {
                    Ok(e) => e,
                    Err(_) => return,
                };

                for changed in &event.paths {
                    let is_relevant = changed.extension().is_some_and(|ext| {
                        ext == "wasm" || ext == "js" || ext == "lua" || ext == "rhai"
                    });
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
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("failed to create file watcher, hot-reload disabled: {e}");
                return;
            }
        };

        if let Err(e) = watcher.watch(&path, notify::RecursiveMode::Recursive) {
            tracing::warn!("failed to start watching plugin directory: {e}");
            return;
        }

        let mut w = self.watcher.write().await;
        *w = Some(watcher);
    }

    /// Handle file change events, find and reload the corresponding plugin directory.
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

            let manifest_path = entry.path().join("manifest.toml");
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

    /// Dispatch Filter-type hooks (chain invocation)
    pub async fn dispatch_filter<T: Clone + Serialize + DeserializeOwned + Send>(
        &self,
        hook: &crate::event::Event,
        input: T,
    ) -> AppResult<T> {
        let func_name = hook.name();
        let content_type = self.extract_content_type(&input);
        self.dispatch_filter_inner(&func_name, input, content_type.as_deref())
            .await
    }

    /// Dispatch Filter-type hooks with explicit content_type
    pub async fn dispatch_filter_with_content_type<
        T: Clone + Serialize + DeserializeOwned + Send,
    >(
        &self,
        hook: &crate::event::Event,
        input: T,
        content_type: &str,
    ) -> AppResult<T> {
        self.dispatch_filter_inner(&hook.name(), input, Some(content_type))
            .await
    }

    async fn dispatch_filter_inner<T: Clone + Serialize + DeserializeOwned + Send>(
        &self,
        func_name: &str,
        input: T,
        content_type: Option<&str>,
    ) -> AppResult<T> {
        let entries = match self.hook_index.get(func_name) {
            Some(e) => e,
            None => return Ok(input),
        };
        if entries.is_empty() {
            return Ok(input);
        }

        let plugins = self.plugins.read().await;
        let mut current = input;

        for entry in entries.iter() {
            if let Some(ct) = content_type
                && !entry.content_types.is_empty()
                && !entry.content_types.iter().any(|t| t == ct)
            {
                continue;
            }

            let plugin_id = &entry.plugin_id;
            if !self.is_plugin_enabled(plugin_id).await {
                continue;
            }

            let Some(plugin) = plugins.get(plugin_id) else {
                continue;
            };

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
                #[cfg(feature = "plugin-rhai")]
                LoadedPluginInstance::Rhai(pid) => self
                    .rhai_engine
                    .call_filter(pid, func_name, &current)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[allow(unreachable_patterns)]
                _ => Err(anyhow::anyhow!("no plugin engine enabled")),
            };

            let elapsed = start.elapsed().as_micros() as u64;
            let is_error = result.is_err();

            match result {
                Ok(Some(result)) => {
                    current = result;
                    self.reset_error_count(plugin_id).await;
                }
                Ok(None) => {
                    self.reset_error_count(plugin_id).await;
                }
                Err(e) => {
                    let err_msg = format!("{e}");
                    tracing::warn!("plugin {} hook {} failed: {err_msg}", plugin_id, func_name,);
                    self.record_hook_error(plugin_id, func_name, &err_msg).await;
                }
            }

            self.record_hook_metrics(plugin_id, func_name, elapsed, is_error)
                .await;
        }

        Ok(current)
    }

    /// Dispatch Action-type hooks (sequential execution, return value ignored)
    pub async fn dispatch_action<T: Serialize>(&self, hook: &str, data: &T) {
        let content_type = self.extract_content_type(data);
        self.dispatch_action_inner(hook, data, content_type.as_deref())
            .await
    }

    /// Dispatch Action-type hooks with explicit content_type
    pub async fn dispatch_action_with_content_type<T: Serialize>(
        &self,
        hook: &str,
        data: &T,
        content_type: &str,
    ) {
        self.dispatch_action_inner(hook, data, Some(content_type))
            .await
    }

    async fn dispatch_action_inner<T: Serialize>(
        &self,
        func_name: &str,
        data: &T,
        content_type: Option<&str>,
    ) {
        let entries = match self.hook_index.get(func_name) {
            Some(e) => e,
            None => return,
        };
        if entries.is_empty() {
            return;
        }

        let plugins = self.plugins.read().await;

        for entry in entries.iter() {
            if let Some(ct) = content_type
                && !entry.content_types.is_empty()
                && !entry.content_types.iter().any(|t| t == ct)
            {
                continue;
            }

            let plugin_id = &entry.plugin_id;
            if !self.is_plugin_enabled(plugin_id).await {
                continue;
            }

            let Some(plugin) = plugins.get(plugin_id) else {
                continue;
            };

            let start = std::time::Instant::now();
            let result: anyhow::Result<()> = match &plugin.instance {
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
                #[cfg(feature = "plugin-rhai")]
                LoadedPluginInstance::Rhai(pid) => self
                    .rhai_engine
                    .call_action(pid, func_name, data)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[allow(unreachable_patterns)]
                _ => Err(anyhow::anyhow!("no plugin engine enabled")),
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
                self.record_hook_error(plugin_id, func_name, &err_msg).await;
            } else {
                self.reset_error_count(plugin_id).await;
            }

            self.record_hook_metrics(plugin_id, func_name, elapsed, is_error)
                .await;
        }
    }

    /// Dispatch `render_markdown` hook (first plugin returning Some wins)
    pub async fn dispatch_render_override(&self, content: &str) -> Option<String> {
        let func_name = "render_markdown";
        let entries = self.hook_index.get(func_name)?;
        if entries.is_empty() {
            return None;
        }

        let plugins = self.plugins.read().await;

        for entry in entries.iter() {
            let plugin_id = &entry.plugin_id;
            if !self.is_plugin_enabled(plugin_id).await {
                continue;
            }

            let Some(plugin) = plugins.get(plugin_id) else {
                continue;
            };

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
                #[cfg(feature = "plugin-rhai")]
                LoadedPluginInstance::Rhai(pid) => self
                    .rhai_engine
                    .call_string_filter(pid, func_name, content)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}")),
                #[allow(unreachable_patterns)]
                _ => Err(anyhow::anyhow!("no plugin engine enabled")),
            };

            let elapsed = start.elapsed().as_micros() as u64;
            let is_error = result.is_err();

            match result {
                Ok(Some(r)) => {
                    self.reset_error_count(plugin_id).await;
                    self.record_hook_metrics(plugin_id, func_name, elapsed, false)
                        .await;
                    return Some(r);
                }
                Ok(None) => {
                    self.reset_error_count(plugin_id).await;
                    self.record_hook_metrics(plugin_id, func_name, elapsed, false)
                        .await;
                    continue;
                }
                Err(e) => {
                    let err_msg = format!("{e}");
                    tracing::warn!("plugin {} render_markdown failed: {err_msg}", plugin_id);
                    self.record_hook_error(plugin_id, func_name, &err_msg).await;
                    self.record_hook_metrics(plugin_id, func_name, elapsed, is_error)
                        .await;
                }
            }
        }

        None
    }

    /// Get the number of loaded plugins
    pub async fn plugin_count(&self) -> usize {
        self.plugins.read().await.len()
    }

    /// Get all declarative routes from plugins (for route registry)
    pub async fn all_plugin_routes(&self) -> Vec<(String, String, String)> {
        let plugins = self.plugins.read().await;
        let mut routes = Vec::new();
        for p in plugins.values() {
            let ext_id = p.manifest.plugin.id.clone();
            for route in &p.manifest.routes {
                routes.push((route.method.clone(), route.path.clone(), ext_id.clone()));
            }
        }
        routes
    }

    /// Get metadata for all loaded plugins
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

    /// Get detailed information for all plugins (including health status, metrics, hook list)
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

    /// Get detail for a single plugin
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

    /// Enable an auto-disabled plugin (reset error count)
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

    /// Disable a plugin (mark as auto-disabled)
    pub async fn disable_plugin(&self, id: &str) -> AppResult<()> {
        let plugins = self.plugins.read().await;
        let plugin = plugins
            .get(id)
            .ok_or_else(|| AppError::not_found("plugin"))?;
        let mut health = plugin.health.write().await;
        health.auto_disabled = true;
        Ok(())
    }

    /// Record a plugin hook error; auto-disable when threshold is reached
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

    /// Record hook execution performance data
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

    /// Check if a plugin is auto-disabled
    fn extract_content_type<T: Serialize>(&self, data: &T) -> Option<String> {
        let val = serde_json::to_value(data).ok()?;
        val.get("content_type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn is_plugin_enabled(&self, plugin_id: &str) -> bool {
        let plugins = self.plugins.read().await;
        let Some(plugin) = plugins.get(plugin_id) else {
            return false;
        };
        let health = plugin.health.read().await;
        !health.auto_disabled
    }

    /// Reset a plugin's error count (after successful invocation)
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

    /// Get a reference to the database connection pool (used by Host API)
    pub fn pool(&self) -> Option<&Pool> {
        self.pool.as_ref()
    }

    /// Dispatch a custom route request.
    ///
    /// Iterates all plugins' `manifest.routes`, matches by method+path, and calls the corresponding handler function.
    pub async fn dispatch_route(
        &self,
        path: &str,
        method: &str,
        body: Option<&str>,
        headers: Option<&serde_json::Value>,
        auth: &crate::middleware::auth::AuthUser,
    ) -> Option<axum::response::Response> {
        if self.route_index.is_empty() {
            return None;
        }

        let path_parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
        let path_key: String = path_parts
            .iter()
            .take(3)
            .copied()
            .collect::<Vec<_>>()
            .join("/");
        let candidates = self.route_index.get(&path_key)?;
        let matched = candidates
            .iter()
            .filter(|e| e.method == method && e.segment_count == path_parts.len())
            .filter(|e| {
                e.static_segments
                    .iter()
                    .all(|(i, s)| path_parts.get(*i).is_some_and(|p| p == s))
            })
            .collect::<Vec<_>>();

        if matched.is_empty() {
            return None;
        }

        let plugins = self.plugins.read().await;
        for entry in &matched {
            let Some(plugin) = plugins.get(&entry.plugin_id) else {
                continue;
            };
            let plugin_id = entry.plugin_id.clone();
            if !self.is_plugin_enabled(&plugin_id).await {
                continue;
            }

            if let Err(resp) = crate::content_type::schema::check_api_access(entry.auth, auth) {
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
            let params = extract_route_params(path, &entry.pattern);
            let input = serde_json::json!({
                "path": path,
                "method": method,
                "body": body.unwrap_or(""),
                "headers": headers.unwrap_or(&serde_json::Value::Null),
                "params": params,
            });

            let handler = &entry.handler;
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
                    tracing::warn!("plugin {plugin_id} route handler {handler} failed: {err_msg}");
                    self.record_hook_error(&plugin_id, handler, &err_msg).await;
                    self.record_hook_metrics(&plugin_id, handler, elapsed, true)
                        .await;
                }
            }
        }

        None
    }

    /// Find a route by exact method + path match in the routes list
    #[allow(dead_code)]
    fn match_route<'a>(routes: &'a [RouteDef], method: &str, path: &str) -> Option<&'a RouteDef> {
        routes
            .iter()
            .find(|r| r.method.eq_ignore_ascii_case(method) && path_matches_route(path, &r.path))
    }

    /// Call a plugin's JSON filter and return a uniformly wrapped Response.
    ///
    /// The plugin returns raw data; the framework wraps it as `{code, message, data}`.
    /// If the plugin returns `{__plugin_error: true, __status, __message}`, it is treated as an error.
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
            #[cfg(feature = "plugin-rhai")]
            LoadedPluginInstance::Rhai(pid) => self
                .rhai_engine
                .call_filter::<serde_json::Value>(pid, handler, input)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            #[allow(unreachable_patterns)]
            _ => None,
        };

        match result {
            Some(result) => {
                if let Some(obj) = result.as_object()
                    && obj.get("__plugin_error").and_then(|v| v.as_bool()) == Some(true)
                {
                    let status = obj.get("__status").and_then(|v| v.as_u64()).unwrap_or(400) as u16;
                    let msg = obj
                        .get("__message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("errors.internal");
                    let status_code = axum::http::StatusCode::from_u16(status)
                        .unwrap_or(axum::http::StatusCode::BAD_REQUEST);
                    let code = status as i32 * 100;
                    let body = format!(r#"{{"code":{code},"message":"{msg}","data":null}}"#);
                    return Ok(Some(
                        (
                            status_code,
                            [(axum::http::header::CONTENT_TYPE, "application/json")],
                            body,
                        )
                            .into_response(),
                    ));
                }

                let body = serde_json::json!({
                    "code": 0,
                    "message": "messages.success",
                    "data": result,
                })
                .to_string();
                Ok(Some(
                    (
                        axum::http::StatusCode::OK,
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        body,
                    )
                        .into_response(),
                ))
            }
            None => Ok(None),
        }
    }
}

/// Route path matching (supports `:param` placeholder segments)
/// Compute route index key: take the first two static segments as prefix.
fn route_index_key(pattern: &str) -> String {
    let parts: Vec<&str> = pattern.trim_end_matches('/').split('/').take(3).collect();
    parts.join("/")
}

#[allow(dead_code)]
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

/// Extract named parameters from a matched route
///
/// ```text
/// path:    /api/v1/plugins/crm/pipeline/deal-123
/// pattern: /api/v1/plugins/crm/pipeline/:dealId
/// → {"dealId": "deal-123"}
/// ```
fn extract_route_params(path: &str, pattern: &str) -> serde_json::Map<String, serde_json::Value> {
    let path_parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let pattern_parts: Vec<&str> = pattern.trim_end_matches('/').split('/').collect();
    let mut params = serde_json::Map::new();
    for (pp, rp) in path_parts.iter().zip(pattern_parts.iter()) {
        if let Some(name) = rp.strip_prefix(':') {
            params.insert(name.to_string(), serde_json::Value::String(pp.to_string()));
        }
    }
    params
}

/// Topological sort: determine load order based on dependencies field
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
    use crate::event::Event;

    fn test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
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
        config.plugin_dir = Some("/tmp/raisfast-plugin-test-nonexistent".into());
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
            .dispatch_filter(&Event::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result, input);
    }

    #[tokio::test]
    async fn dispatch_action_with_no_plugins_does_nothing() {
        let config = test_config();
        let mgr = PluginManager::new(config).await;
        mgr.dispatch_action("on_post_created", &serde_json::json!({"id": "123"}))
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
        let auth = crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        );
        assert!(
            mgr.dispatch_route("/api/v1/test", "GET", None, None, &auth)
                .await
                .is_none()
        );
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"export function on_post_creating(j) { return j; }"#,
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    input.title = input.title.toUpperCase();
    return input;
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input)
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
export function render_markdown(content) {
    return content.replace("<head>", '<head><meta property="og:type" content="article">');
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
import { log } from 'sdk';

export function on_post_created(dataJson) {
    log("info", "post created");
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.dispatch_action("on_post_created", &serde_json::json!({"id": "abc"}))
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
export function handle_test(input) {
    return { hello: "world" };
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_route(
                "/api/v1/custom/test",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
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
            plugin_dir.join("manifest.toml"),
            "[plugin]\nid=\"com.test.js-unload\"\nname=\"JSU\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.js"), r#"export const noop = 1;"#).unwrap();

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
            plugin_dir.join("manifest.toml"),
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"
import { configGet } from 'sdk';

export function on_post_creating(inputJson) {
    var input = JSON.parse(inputJson);
    var env = configGet("app.env");
    if (env) {
        input.env = env;
    }
    return input;
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello"});
        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["env"], "test");
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
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
            .dispatch_filter(&Event::PostCreating, input)
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_created = function(data)
        RaisFastHost.log("info", "post created")
    end
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.dispatch_action("on_post_created", &serde_json::json!({"id": "abc"}))
            .await;
    }

    #[cfg(feature = "plugin-lua")]
    #[tokio::test]
    async fn manager_lua_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("lua-unload");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("manifest.toml"),
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
            plugin_dir.join("manifest.toml"),
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.lua"),
            r#"
Plugin = {
    on_post_creating = function(input)
        local env = RaisFastHost.getConfig("app.env")
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
            .dispatch_filter(&Event::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["env"], "test");
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("index.js"),
            r#"export function on_post_creating(j) { return j; }"#,
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
            plugin_dir.join("manifest.toml"),
            "[plugin]\nid=\"com.test.toggle\"\nname=\"T\"\nversion=\"1.0.0\"\nruntime=\"js\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("index.js"), r#"export const noop = 1;"#).unwrap();

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
                sdk_version: "v1".into(),
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

    // ── VFS integration tests ──────────────────────────────────────────────

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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();

        let lua_code = r#"
Plugin = {
    on_post_creating = function(input)
        local slug = input.slug or ""
        if slug ~= "" then
            local exists = RaisFastHost.vfsExists("cache/" .. slug .. ".txt")
            if exists then
                input.cache_hit = true
            end
            local stat = RaisFastHost.vfsRead("stats.json")
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
            RaisFastHost.vfsWrite("cache/" .. slug .. ".txt", title .. "|" .. (input.content or ""))
            RaisFastHost.vfsWrite("stats.json", '{"writes":1}')

            local info = RaisFastHost.vfsStat("cache/" .. slug .. ".txt")
            if info then
                input.file_stat = info
            end

            local entries = RaisFastHost.vfsList("cache")
            if entries then
                input.cache_files = table.concat(entries, ",")
            end
        end
        return input
    end,

    on_post_deleted = function(input)
        local slug = input.slug or ""
        if slug ~= "" then
            RaisFastHost.vfsDelete("cache/" .. slug .. ".txt")
            local entries = RaisFastHost.vfsList("cache")
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
            .dispatch_filter(&Event::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result["cache_hit"], serde_json::Value::Null);
        assert!(result["stats"].is_null());

        mgr.dispatch_action("on_post_created", &input).await;

        let vfs_plugin_dir = vfs_root.join("com.test.lua-vfs");
        let cache_file = vfs_plugin_dir.join("cache/hello-world.txt");
        assert!(cache_file.exists());
        let content = std::fs::read_to_string(&cache_file).unwrap();
        assert_eq!(content, "Hello|world");

        let stats_file = vfs_plugin_dir.join("stats.json");
        assert!(stats_file.exists());

        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input.clone())
            .await
            .unwrap();
        assert_eq!(result["cache_hit"], true);
        assert!(result["stats"].is_string());

        let delete_input = serde_json::json!({"slug": "hello-world"});
        mgr.dispatch_action("on_post_deleted", &delete_input).await;
        assert!(!cache_file.exists());

        let check_input = serde_json::json!({"slug": "hello-world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, check_input)
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();

        let lua_code = r#"
Plugin = {}
Plugin.ping = function(input)
    return {
        status = 200,
        body = '{"code":0,"message":"ok","data":"pong"}'
    }
end

Plugin.count = function(input)
    local result = RaisFastHost.dbQuery("SELECT COUNT(*) as total FROM posts")
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
            .dispatch_route(
                "/api/v1/plugins/stats/ping",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(result.is_some(), "should match declared route");

        let result = mgr
            .dispatch_route(
                "/api/v1/plugins/stats/unknown",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(result.is_none(), "should return none for undeclared path");

        let result = mgr
            .dispatch_route(
                "/api/v1/plugins/stats/ping",
                "POST",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
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
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();

        let lua_code = "Plugin = { on_cron_tick = function(data) RaisFastHost.setData(\"last_job\", data.job_type or \"\") end }";
        std::fs::write(plugin_dir.join("init.lua"), lua_code).unwrap();

        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(crate::db::schema::SCHEMA_SQL)
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
            "on_cron_tick",
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

    // ── Rhai plugin tests ─────────────────────────────────────────

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_load_rhai_plugin_from_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-test-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-plugin"
name = "Rhai Test"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[hooks.on_post_creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"fn on_post_creating(j) { j }"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        let plugins = mgr.list_plugins().await;
        assert_eq!(plugins[0].0, "com.test.rhai-plugin");
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_rhai_plugin_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-filter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-filter"
name = "Rhai Filter"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[hooks.on_post_creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn on_post_creating(input) {
    input.title = to_upper(input.title);
    input
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({"title": "hello", "content": "world"});
        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input)
            .await
            .unwrap();
        assert_eq!(result["title"], "HELLO");
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_rhai_plugin_string_filter() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-strfilter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-strfilter"
name = "Rhai String Filter"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[hooks.render_markdown]
priority = 5
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn render_markdown(content) {
    replace(content, "<head>", `<head><meta property="og:type" content="article">`)
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

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_rhai_plugin_action_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-action-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-action"
name = "Rhai Action"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[hooks.on_post_created]
priority = 10
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn on_post_created(data_json) {
    log("info", "post created");
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        mgr.dispatch_action("on_post_created", &serde_json::json!({"id": "abc"}))
            .await;
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_rhai_plugin_unload() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-unload");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("manifest.toml"),
            "[plugin]\nid=\"com.test.rhai-unload\"\nname=\"RU\"\nversion=\"1.0.0\"\nruntime=\"rhai\"\nentry=\"init.rhai\"",
        )
        .unwrap();
        std::fs::write(plugin_dir.join("init.rhai"), "fn noop() { 42 }").unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        assert_eq!(mgr.plugin_count().await, 1);
        mgr.unload_plugin("com.test.rhai-unload").await;
        assert_eq!(mgr.plugin_count().await, 0);
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn manager_rhai_plugin_route_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-route-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-route"
name = "Rhai Route"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[[routes]]
method = "GET"
path = "/api/v1/custom/rhai-test"
handler = "handle_test"
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn handle_test(input) {
    `{"hello":"world"}`
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let result = mgr
            .dispatch_route(
                "/api/v1/custom/rhai-test",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(result.is_some());
    }

    // ── Rhai real plugin integration test (seo-rhai)──────────────────────────

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn rhai_seo_plugin_filter_auto_slug_and_meta() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("rhai-filter-plugin");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = r#"
[plugin]
id = "com.test.rhai-seo"
name = "SEO"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[hooks.on-post-creating]
priority = 10
"#;
        std::fs::write(plugin_dir.join("manifest.toml"), manifest).unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn on_post_creating(input) {
    input.slug = to_lower(replace(input.title, " ", "-"));
    input._seo_optimized = true;
    let content = input.content;
    if content.len() > 120 {
        input.meta_description = content.sub_string(0, 120) + "...";
    } else {
        input.meta_description = content;
    }
    input
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let input = serde_json::json!({
            "title": "Hello World from Rhai",
            "content": "This is a test post to verify the Rhai plugin engine works correctly with the full plugin lifecycle."
        });
        let result: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input)
            .await
            .unwrap();

        assert_eq!(result["slug"], "hello-world-from-rhai");
        assert_eq!(result["_seo_optimized"], true);
        assert!(result["meta_description"].is_string());
        assert!(
            result["meta_description"]
                .as_str()
                .unwrap()
                .contains("This is a test"),
            "meta_description should contain content"
        );
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn rhai_seo_plugin_render_markdown_injects_og() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("seo-rhai");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("manifest.toml"),
            r#"
[plugin]
id = "com.raisfast.seo-rhai"
name = "SEO Rhai"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[permissions]
config = ["app.*", "seo.*"]

[hooks.render_markdown]
priority = 5
"#,
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn render_markdown(html) {
    let og_type = getConfig("seo.og_type");
    let og_str = "" + og_type;
    if og_str == "" || og_str == "()" {
        og_str = "article";
    }
    let injection = `<meta property="og:type" content="` + og_str + `">` +
        `<meta name="generator" content="raisfast-seo-rhai/1.0.0">`;
    replace(html, "<head>", "<head>" + injection)
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        let mgr = PluginManager::new(Arc::new(config)).await;

        let html = "<head><title>Test Post</title></head><body><h1>Hello</h1></body>";
        let result = mgr.dispatch_render_override(html).await;

        assert!(result.is_some(), "should modify HTML");
        let modified = result.unwrap();
        assert!(
            modified.contains(r#"property="og:type""#),
            "should inject og:type meta, got: {modified}"
        );
        assert!(
            modified.contains(r#"content="article""#),
            "default og_type should be article, got: {modified}"
        );
        assert!(
            modified.contains(r#"name="generator""#),
            "should inject generator meta, got: {modified}"
        );
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn rhai_seo_plugin_action_vfs_cache() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("seo-rhai");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("manifest.toml"),
            r#"
[plugin]
id = "com.raisfast.seo-vfs"
name = "SEO VFS"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[permissions]
filesystem = ["read-write"]

[hooks.on-post-created]
priority = 10

[hooks.on-post-creating]
priority = 10
"#,
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn on_post_creating(input) {
    input.slug = to_lower(replace(input.title, " ", "-"));
    input
}

fn on_post_created(data) {
    let slug = data.slug;
    let cache_key = "cache/posts/" + slug + ".json";
    vfsWrite(cache_key, to_json(data));

    let stats_key = "cache/seo_stats.json";
    let existing = "0";
    if vfsExists(stats_key) {
        existing = vfsRead(stats_key);
    }
    let count = parse_int(existing) + 1;
    vfsWrite(stats_key, count.to_string());
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        let vfs_root = dir.path().join("vfs");
        std::fs::create_dir_all(&vfs_root).unwrap();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        config.plugin_vfs_root = vfs_root.to_string_lossy().to_string();
        config.plugin_vfs_max_file_size = 65536;
        config.plugin_vfs_max_total_size = 1048576;
        let mgr = PluginManager::new(Arc::new(config)).await;

        // Create the first post
        let input1 = serde_json::json!({
            "title": "First Post",
            "content": "Hello world"
        });
        let created1: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input1)
            .await
            .unwrap();
        mgr.dispatch_action("on_post_created", &created1).await;

        // Verify VFS cache file was written
        let vfs_plugin = vfs_root.join("com.raisfast.seo-vfs");
        let cache_file = vfs_plugin.join("cache/posts/first-post.json");
        assert!(cache_file.exists(), "cache file should exist");
        let cached: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&cache_file).unwrap()).unwrap();
        assert_eq!(cached["title"], "First Post");

        let stats_file = vfs_plugin.join("cache/seo_stats.json");
        assert!(stats_file.exists(), "stats file should exist");
        assert_eq!(std::fs::read_to_string(&stats_file).unwrap(), "1");

        // Create the second post
        let input2 = serde_json::json!({
            "title": "Second Post",
            "content": "Another post"
        });
        let created2: serde_json::Value = mgr
            .dispatch_filter(&Event::PostCreating, input2)
            .await
            .unwrap();
        mgr.dispatch_action("on_post_created", &created2).await;

        // Verify counter incremented
        assert_eq!(
            std::fs::read_to_string(&stats_file).unwrap(),
            "2",
            "counter should increment to 2"
        );

        // Verify filter output slug
        assert_eq!(created2["slug"], "second-post");
    }

    #[cfg(feature = "plugin-rhai")]
    #[tokio::test]
    async fn rhai_seo_plugin_custom_routes() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("seo-rhai");
        std::fs::create_dir_all(&plugin_dir).unwrap();

        std::fs::write(
            plugin_dir.join("manifest.toml"),
            r#"
[plugin]
id = "com.raisfast.seo-routes"
name = "SEO Routes"
version = "1.0.0"
runtime = "rhai"
entry = "init.rhai"

[permissions]
filesystem = ["read-write"]

[[routes]]
method = "GET"
path = "/api/v1/plugins/seo/stats"
handler = "seo_stats"

[[routes]]
method = "GET"
path = "/api/v1/plugins/seo/health"
handler = "seo_health"
"#,
        )
        .unwrap();
        std::fs::write(
            plugin_dir.join("init.rhai"),
            r#"
fn seo_stats(input) {
    let count = "0";
    if vfsExists("cache/seo_stats.json") {
        count = vfsRead("cache/seo_stats.json");
    }
    `{"total_optimized":${count},"engine":"rhai","version":"1.0.0"}`
}

fn seo_health(input) {
    `{"status":"ok","engine":"rhai"}`
}
"#,
        )
        .unwrap();

        let mut config = (*test_config()).clone();
        let vfs_root = dir.path().join("vfs");
        std::fs::create_dir_all(&vfs_root).unwrap();
        config.plugin_dir = Some(dir.path().to_string_lossy().to_string());
        config.plugin_vfs_root = vfs_root.to_string_lossy().to_string();
        config.plugin_vfs_max_file_size = 65536;
        config.plugin_vfs_max_total_size = 1048576;
        let mgr = PluginManager::new(Arc::new(config)).await;

        // health route
        let health = mgr
            .dispatch_route(
                "/api/v1/plugins/seo/health",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(health.is_some(), "health route should match");

        // stats route (count=0 when no cache)
        let stats = mgr
            .dispatch_route(
                "/api/v1/plugins/seo/stats",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(stats.is_some(), "stats route should match");

        // Non-existent route
        let none = mgr
            .dispatch_route(
                "/api/v1/plugins/seo/nonexistent",
                "GET",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(none.is_none(), "unknown route should not match");

        // Wrong method
        let wrong_method = mgr
            .dispatch_route(
                "/api/v1/plugins/seo/health",
                "POST",
                None,
                None,
                &crate::middleware::auth::AuthUser::new_test(
                    0,
                    crate::models::user::UserRole::Reader,
                    "",
                ),
            )
            .await;
        assert!(wrong_method.is_none(), "wrong method should not match");
    }
}
