//! Extension 管理器 — 发现、加载、编排 Content Type 和 Plugin

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::config::app::AppConfig;
use crate::content_type::ContentTypeRegistry;
use crate::content_type::repository::ContentRepository;
use crate::content_type::schema::ContentTypeSchema;
use crate::db::Pool;
use crate::errors::app_error::{AppError, AppResult};
use crate::extension::manifest::ExtensionManifest;
use crate::extension::model;
use crate::plugins::PluginManager;

/// 已加载的 Extension 运行时信息
#[derive(Debug, Clone)]
pub struct LoadedExtension {
    /// 清单
    pub manifest: ExtensionManifest,
    /// Extension 根目录绝对路径
    pub root_dir: PathBuf,
    /// 已加载的 Content Type singular 列表
    pub content_type_names: Vec<String>,
    /// 是否包含 Plugin
    pub has_plugin: bool,
    /// 是否启用
    pub enabled: bool,
}

/// Extension 管理器
///
/// 统一管理 Extension 的发现、加载、启用、禁用。
/// 内部协调 ContentTypeRegistry 和 PluginManager。
pub struct ExtensionManager {
    /// 已加载的 Extension
    extensions: RwLock<HashMap<String, LoadedExtension>>,
    /// Content Type 注册表
    ct_registry: Arc<ContentTypeRegistry>,
    /// Plugin 管理器
    plugin_manager: Arc<PluginManager>,
    /// 数据库连接池
    pool: Pool,
    /// Extension 根目录
    extension_dir: PathBuf,
}

impl ExtensionManager {
    /// 创建 ExtensionManager 并加载所有 Extension
    pub async fn new(
        ct_registry: Arc<ContentTypeRegistry>,
        plugin_manager: Arc<PluginManager>,
        pool: Pool,
        config: &AppConfig,
    ) -> Arc<Self> {
        let extension_dir = PathBuf::from(&config.extension_dir);

        let mgr = Arc::new(Self {
            extensions: RwLock::new(HashMap::new()),
            ct_registry,
            plugin_manager,
            pool,
            extension_dir,
        });

        if mgr.extension_dir.exists() {
            if let Err(e) = mgr.load_all().await {
                tracing::error!("failed to load extensions: {e}");
            }
        } else {
            tracing::info!(
                "extension_dir '{}' not found, skipping",
                config.extension_dir
            );
        }

        mgr
    }

    /// 扫描 extensions/ 目录，按依赖拓扑排序后依次加载
    pub async fn load_all(&self) -> AppResult<()> {
        let manifests = self.discover()?;

        let ordered = topological_sort(&manifests)?;

        for ext_id in &ordered {
            let manifest = manifests.get(ext_id).cloned().unwrap();
            let ext_root = self.extension_dir.join(ext_id);

            match self.load_one(&ext_root, &manifest).await {
                Ok(()) => {
                    tracing::info!(
                        "loaded extension: {} v{} (ct={}, plugin={})",
                        manifest.extension.id,
                        manifest.extension.version,
                        manifest.has_content_types(),
                        manifest.has_plugin(),
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "failed to load extension '{}': {e:?}",
                        manifest.extension.id
                    );
                }
            }
        }

        Ok(())
    }

    /// 扫描目录发现所有 extension.toml
    fn discover(&self) -> AppResult<HashMap<String, ExtensionManifest>> {
        let mut manifests = HashMap::new();

        let entries = std::fs::read_dir(&self.extension_dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "cannot read extensions dir {:?}: {e}",
                self.extension_dir
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let manifest_path = path.join("extension.toml");
            if !manifest_path.exists() {
                continue;
            }

            let manifest = ExtensionManifest::parse_from_file(&manifest_path)?;
            let ext_id = manifest.extension.id.clone();

            if ext_id
                != path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref()
            {
                tracing::warn!(
                    "extension directory name '{}' does not match manifest id '{}', using manifest id",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    ext_id,
                );
            }

            manifests.insert(ext_id, manifest);
        }

        tracing::info!("discovered {} extension(s)", manifests.len());
        Ok(manifests)
    }

    /// 加载单个 Extension（CT + Plugin）
    async fn load_one(&self, ext_root: &Path, manifest: &ExtensionManifest) -> AppResult<()> {
        let ext_id = &manifest.extension.id;
        let mut ct_names = Vec::new();
        let mut has_plugin = false;

        let db_record = model::find_by_id(&self.pool, ext_id).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "extension '{}': DB query failed: {e}",
                ext_id
            ))
        })?;
        let is_enabled = db_record.as_ref().is_none_or(|r| r.enabled == 1);

        if !is_enabled {
            tracing::info!("extension '{}' is disabled, skipping load", ext_id);
            let loaded = LoadedExtension {
                manifest: manifest.clone(),
                root_dir: ext_root.to_path_buf(),
                content_type_names: vec![],
                has_plugin: false,
                enabled: false,
            };
            self.extensions
                .write()
                .expect("lock poisoned")
                .insert(ext_id.clone(), loaded);
            return Ok(());
        }

        if db_record.is_none() {
            let (_, now) = crate::utils::id::new_id_and_timestamp();
            let record = model::ExtensionRecord {
                id: ext_id.clone(),
                name: manifest.extension.name.clone(),
                version: manifest.extension.version.clone(),
                enabled: 1,
                config: None,
                installed_at: now.clone(),
                updated_at: now,
                tenant_id: None,
            };
            model::insert(&self.pool, &record).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "extension '{}': insert DB record failed: {e}",
                    ext_id
                ))
            })?;
        }

        if let Some(ct_dir) = manifest.content_types_dir(ext_root)
            && ct_dir.exists()
        {
            ct_names = self
                .load_content_types(&ct_dir, ext_id)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!(
                        "extension '{}': load content types failed: {e}",
                        ext_id
                    ))
                })?;
        }

        if let Some(plugin_manifest_path) = manifest.plugin_manifest_path(ext_root)
            && plugin_manifest_path.exists()
        {
            self.plugin_manager
                .load_plugin_from_dir(&plugin_manifest_path)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!(
                        "failed to load plugin for extension '{}': {e}",
                        ext_id
                    ))
                })?;
            has_plugin = true;
        }

        let loaded = LoadedExtension {
            manifest: manifest.clone(),
            root_dir: ext_root.to_path_buf(),
            content_type_names: ct_names,
            has_plugin,
            enabled: true,
        };

        self.extensions
            .write()
            .expect("lock poisoned")
            .insert(ext_id.clone(), loaded);

        Ok(())
    }

    /// 加载一个 Content Type 目录下的所有 TOML 文件
    async fn load_content_types(&self, ct_dir: &Path, ext_id: &str) -> AppResult<Vec<String>> {
        let entries = std::fs::read_dir(ct_dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "cannot read content_types dir {ct_dir:?}: {e}"
            ))
        })?;

        let mut names = Vec::new();
        let repo = ContentRepository::new(self.pool.clone());

        for entry in entries {
            let entry = entry.map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let mut schema = ContentTypeSchema::parse_from_file(&path)?;
                let singular = schema.singular.clone();
                schema.extension_id = Some(ext_id.to_string());
                self.ct_registry.register(schema.clone());
                repo.migrate(&schema).await.map_err(|e| {
                    AppError::Internal(anyhow::anyhow!(
                        "migration failed for content type '{}': {e}",
                        singular
                    ))
                })?;
                tracing::debug!("loaded content type: {singular}");
                names.push(singular);
            }
        }

        Ok(names)
    }

    /// 获取所有已加载 Extension 列表
    pub fn list_loaded(&self) -> Vec<LoadedExtension> {
        self.extensions
            .read()
            .expect("lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// 按 ID 查询已加载 Extension
    pub fn get_loaded(&self, id: &str) -> Option<LoadedExtension> {
        self.extensions
            .read()
            .expect("lock poisoned")
            .get(id)
            .cloned()
    }

    /// 启用 Extension
    pub async fn enable(&self, id: &str) -> AppResult<()> {
        let ext = self
            .get_loaded(id)
            .ok_or_else(|| AppError::not_found("extension"))?;

        if ext.enabled {
            return Err(AppError::Conflict("extension.already_enabled".into()));
        }

        let mut ct_names = Vec::new();
        let mut has_plugin = false;

        if let Some(ct_dir) = ext.manifest.content_types_dir(&ext.root_dir)
            && ct_dir.exists()
        {
            ct_names = self.load_content_types(&ct_dir, id).await.map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "extension '{}': load content types failed: {e}",
                    id
                ))
            })?;
        }

        if let Some(plugin_manifest_path) = ext.manifest.plugin_manifest_path(&ext.root_dir)
            && plugin_manifest_path.exists()
        {
            self.plugin_manager
                .load_plugin_from_dir(&plugin_manifest_path)
                .await
                .map_err(|e| {
                    AppError::Internal(anyhow::anyhow!(
                        "failed to load plugin for extension '{}': {e}",
                        id
                    ))
                })?;
            has_plugin = true;
        }

        let (_, now) = crate::utils::id::new_id_and_timestamp();
        model::set_enabled(&self.pool, id, true, &now).await?;

        self.extensions
            .write()
            .expect("lock poisoned")
            .entry(id.to_string())
            .and_modify(|e| {
                e.enabled = true;
                e.content_type_names = ct_names;
                e.has_plugin = has_plugin;
            });

        tracing::info!("extension '{}' enabled", id);
        Ok(())
    }

    /// 禁用 Extension
    pub async fn disable(&self, id: &str) -> AppResult<()> {
        let ext = self
            .get_loaded(id)
            .ok_or_else(|| AppError::not_found("extension"))?;

        if !ext.enabled {
            return Err(AppError::Conflict("extension.already_disabled".into()));
        }

        let all = self.list_loaded();
        for other in &all {
            if other.manifest.extension.id == id {
                continue;
            }
            if !other.enabled {
                continue;
            }
            if other.manifest.extension.dependencies.contains_key(id) {
                return Err(AppError::Conflict(format!(
                    "extension '{}' depends on '{}', cannot disable",
                    other.manifest.extension.id, id
                )));
            }
        }

        // 卸载 Plugin
        if ext.has_plugin {
            let plugin_id = format!("ext.{}", id);
            let _ = self.plugin_manager.unload_plugin(&plugin_id).await;
        }

        // 注销 Content Types
        for ct_name in &ext.content_type_names {
            self.ct_registry.unregister(ct_name);
        }

        let (_, now) = crate::utils::id::new_id_and_timestamp();
        model::set_enabled(&self.pool, id, false, &now).await?;

        self.extensions
            .write()
            .expect("lock poisoned")
            .entry(id.to_string())
            .and_modify(|e| {
                e.enabled = false;
                e.content_type_names.clear();
                e.has_plugin = false;
            });

        tracing::info!("extension '{}' disabled", id);
        Ok(())
    }

    /// 卸载 Extension（禁用 + 删除文件 + 删除 DB 记录）
    pub async fn uninstall(&self, id: &str, drop_tables: bool) -> AppResult<()> {
        let ext = self
            .get_loaded(id)
            .ok_or_else(|| AppError::not_found("extension"))?;

        // 先禁用
        if ext.enabled {
            self.disable(id).await?;
        }

        // 可选：删除 CT 数据表
        if drop_tables {
            for ct_name in &ext.content_type_names {
                if let Some(schema) = self.ct_registry.get(ct_name) {
                    let drop_sql = format!("DROP TABLE IF EXISTS {}", schema.table);
                    sqlx::query(&drop_sql)
                        .execute(&self.pool)
                        .await
                        .map_err(|e| {
                            AppError::Internal(anyhow::anyhow!(
                                "failed to drop table '{}': {e}",
                                schema.table
                            ))
                        })?;
                }
            }
        }

        // 删除文件
        if ext.root_dir.exists() {
            std::fs::remove_dir_all(&ext.root_dir).map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "failed to remove extension dir {:?}: {e}",
                    ext.root_dir
                ))
            })?;
        }

        // 删除 DB 记录
        model::delete(&self.pool, id).await?;

        self.extensions.write().expect("lock poisoned").remove(id);

        tracing::info!("extension '{}' uninstalled", id);
        Ok(())
    }

    /// 获取 Pool 引用
    pub fn pool(&self) -> &Pool {
        &self.pool
    }
}

/// 拓扑排序 Extension 按依赖关系
fn topological_sort(manifests: &HashMap<String, ExtensionManifest>) -> AppResult<Vec<String>> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();

    for id in manifests.keys() {
        in_degree.entry(id.as_str()).or_insert(0);
        graph.entry(id.as_str()).or_default();
    }

    for (id, manifest) in manifests {
        for dep_id in manifest.extension.dependencies.keys() {
            if !manifests.contains_key(dep_id.as_str()) {
                tracing::warn!(
                    "extension '{}' depends on '{}' which is not installed",
                    id,
                    dep_id
                );
                continue;
            }
            graph.entry(dep_id.as_str()).or_default().push(id.as_str());
            *in_degree.entry(id.as_str()).or_insert(0) += 1;
        }
    }

    let mut queue: Vec<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(id, _)| *id)
        .collect();
    queue.sort();

    let mut result = Vec::new();

    while let Some(id) = queue.pop() {
        result.push(id.to_string());
        if let Some(dependents) = graph.get(id) {
            for &dep in dependents {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(dep);
                        queue.sort();
                    }
                }
            }
        }
    }

    if result.len() != manifests.len() {
        let missing: Vec<&str> = manifests
            .keys()
            .map(|k| k.as_str())
            .filter(|k| !result.contains(&k.to_string()))
            .collect();
        return Err(AppError::Internal(anyhow::anyhow!(
            "circular dependency detected among extensions: {:?}",
            missing
        )));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topological_sort_no_deps() {
        let mut manifests = HashMap::new();
        for id in &["c", "a", "b"] {
            manifests.insert(
                id.to_string(),
                ExtensionManifest::parse_from_str(&format!(
                    "[extension]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\n"
                ))
                .unwrap(),
            );
        }
        let order = topological_sort(&manifests).unwrap();
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn topological_sort_with_deps() {
        let mut manifests = HashMap::new();
        manifests.insert(
            "a".to_string(),
            ExtensionManifest::parse_from_str(
                "[extension]\nid = \"a\"\nname = \"A\"\nversion = \"1.0.0\"\n",
            )
            .unwrap(),
        );
        manifests.insert(
            "b".to_string(),
            ExtensionManifest::parse_from_str(
                "[extension]\nid = \"b\"\nname = \"B\"\nversion = \"1.0.0\"\n\n[extension.dependencies]\na = \">=1.0.0\"\n",
            )
            .unwrap(),
        );
        manifests.insert(
            "c".to_string(),
            ExtensionManifest::parse_from_str(
                "[extension]\nid = \"c\"\nname = \"C\"\nversion = \"1.0.0\"\n\n[extension.dependencies]\nb = \">=1.0.0\"\n",
            )
            .unwrap(),
        );
        let order = topological_sort(&manifests).unwrap();
        let a_pos = order.iter().position(|x| x == "a").unwrap();
        let b_pos = order.iter().position(|x| x == "b").unwrap();
        let c_pos = order.iter().position(|x| x == "c").unwrap();
        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }
}
