//! 插件宿主公共逻辑
//!
//! 将 JS / Lua 两套 Host API 的共享业务逻辑抽取到 [`HostContext`] 中。
//! 各引擎的 `register_host_functions` 只负责引擎特定的参数绑定，
//! 所有权限校验、HTTP 请求、DB 查询均委托给 `HostContext` 方法。

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::permissions::PermissionChecker;
use crate::plugins::vfs::VirtualFs;

/// 插件宿主上下文
///
/// 持有单个插件所需的全部共享状态，业务逻辑方法均为同步接口
/// （内部通过 `block_in_place` + `block_on` 执行异步操作）。
pub struct HostContext {
    /// 运行时标签，用于日志前缀（`"js"` / `"lua"`）
    pub runtime_label: &'static str,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
}

impl HostContext {
    /// 创建新的宿主上下文
    #[must_use]
    pub fn new(
        runtime_label: &'static str,
        config: Arc<AppConfig>,
        plugin_id: String,
        permissions: Permissions,
        pool: Option<Pool>,
    ) -> Self {
        Self {
            runtime_label,
            config,
            plugin_id,
            permissions,
            pool,
        }
    }

    /// 返回插件 ID
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// 返回内存上限（字节），基于 permissions 或默认 32 MB
    #[must_use]
    pub fn max_memory_bytes(&self) -> usize {
        self.permissions
            .max_memory_mb
            .map_or(32 * 1024 * 1024, |mb| mb as usize * 1024 * 1024)
    }

    /// 日志输出
    pub fn log(&self, level: &str, msg: &str) {
        let tag = self.runtime_label;
        match level {
            "warn" => tracing::warn!("[plugin:{tag}] {msg}"),
            "error" => tracing::error!("[plugin:{tag}] {msg}"),
            _ => tracing::info!("[plugin:{tag}] {msg}"),
        }
    }

    /// 读取配置项
    #[must_use]
    pub fn get_config(&self, key: &str) -> Option<String> {
        if !PermissionChecker::is_config_key_allowed(&self.permissions, key) {
            return None;
        }
        get_config_value(&self.config, key)
    }

    /// HTTP GET 请求
    #[must_use]
    pub fn http_get(&self, url: &str) -> String {
        if !PermissionChecker::is_url_allowed(&self.permissions, url) {
            return format!("error: URL not allowed: {url}");
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::plugins::http_client::http_get(url)) {
                Ok(body) => body,
                Err(e) => format!("error: {e}"),
            }
        })
    }

    /// HTTP POST 请求
    #[must_use]
    pub fn http_post(&self, url: &str, body: &str) -> String {
        if !PermissionChecker::is_url_allowed(&self.permissions, url) {
            return format!("error: URL not allowed: {url}");
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::plugins::http_client::http_post(url, body, None)) {
                Ok(resp) => resp,
                Err(e) => format!("error: {e}"),
            }
        })
    }

    /// 读取插件 KV 存储
    pub fn get_data(&self, key: &str) -> Option<String> {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] Host.getData called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return None;
        };
        let handle = tokio::runtime::Handle::current();
        let pid = self.plugin_id.clone();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::plugin_storage::get(pool, &pid, key)) {
                Ok(val) => val,
                Err(e) => {
                    tracing::error!("[plugin:{}] getData error: {e}", self.runtime_label);
                    None
                }
            }
        })
    }

    /// 写入插件 KV 存储
    pub fn set_data(&self, key: &str, value: &str) -> bool {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] Host.setData called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return false;
        };
        let handle = tokio::runtime::Handle::current();
        let pid = self.plugin_id.clone();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::plugin_storage::set(
                pool, &pid, key, value, None,
            )) {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!("[plugin:{}] setData error: {e}", self.runtime_label);
                    false
                }
            }
        })
    }

    /// 根据 slug 获取文章（JSON）
    pub fn get_post(&self, slug: &str) -> Option<String> {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] Host.getPost called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return None;
        };
        if !PermissionChecker::is_table_readable(&self.permissions, "posts") {
            tracing::debug!(
                "[plugin:{}] getPost denied: no read:posts permission",
                self.runtime_label
            );
            return None;
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::post::find_by_slug(
                pool,
                slug,
                Some(crate::db::tenant::DEFAULT_TENANT),
            )) {
                Ok(Some(post)) => serde_json::to_string(&post).ok(),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("[plugin:{}] getPost error: {e}", self.runtime_label);
                    None
                }
            }
        })
    }

    /// 执行只读 SQL 查询（返回 JSON 数组字符串）
    #[must_use]
    pub fn db_query(&self, sql: &str) -> String {
        if !PermissionChecker::is_readonly_query(sql) {
            return "error: only SELECT queries are allowed".to_string();
        }
        let Some(pool) = &self.pool else {
            return "error: no database access".to_string();
        };
        let table = crate::plugins::permissions::extract_table_name(sql);
        if let Some(tbl) = &table
            && !PermissionChecker::is_table_readable(&self.permissions, tbl)
        {
            return format!("error: no read permission for table: {tbl}");
        }
        let handle = tokio::runtime::Handle::current();
        let sql = crate::db::dialect::translate(sql).into_owned();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let rows = sqlx::query(&sql).fetch_all(pool).await?;
                let json = crate::plugins::rows_to_json(&rows);
                Ok::<_, sqlx::Error>(json)
            }) {
                Ok(json) => json,
                Err(e) => format!("error: {e}"),
            }
        })
    }

    /// 读取虚拟文件系统中的文件
    pub fn fs_read(&self, path: &str) -> Result<String, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.read_file(path).map_err(|e| e.to_string())
    }

    /// 写入虚拟文件系统中的文件
    pub fn fs_write(&self, path: &str, content: &str) -> Result<(), String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.write_file(path, content).map_err(|e| e.to_string())
    }

    /// 删除虚拟文件系统中的文件
    pub fn fs_delete(&self, path: &str) -> Result<(), String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.delete_file(path).map_err(|e| e.to_string())
    }

    /// 检查虚拟文件系统中的文件是否存在
    pub fn fs_exists(&self, path: &str) -> Result<bool, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.exists(path).map_err(|e| e.to_string())
    }

    /// 列出虚拟文件系统目录内容
    pub fn fs_list(&self, path: &str) -> Result<Vec<String>, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.list_dir(path).map_err(|e| e.to_string())
    }

    /// 获取虚拟文件系统文件元信息（JSON）
    pub fn fs_stat(&self, path: &str) -> Result<String, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        let info = vfs.stat(path).map_err(|e| e.to_string())?;
        serde_json::to_string(&info).map_err(|e| format!("error: {e}"))
    }
}

/// 根据 key 路径从 `AppConfig` 读取配置值
#[must_use]
pub fn get_config_value(config: &AppConfig, key: &str) -> Option<String> {
    match key {
        "app.host" => Some(config.host.clone()),
        "app.port" => Some(config.port.to_string()),
        "app.env" => Some(config.env.clone()),
        "app.base_url" => Some(config.base_url.clone()),
        "jwt.access_expires" => Some(config.jwt_access_expires.to_string()),
        "jwt.refresh_expires" => Some(config.jwt_refresh_expires.to_string()),
        "upload.dir" => Some(config.upload_dir.clone()),
        "upload.max_size" => Some(config.max_upload_size.to_string()),
        "plugin.max_memory_mb" => Some(config.plugin_max_memory_mb.to_string()),
        "plugin.default_timeout_ms" => Some(config.plugin_default_timeout_ms.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig {
            host: "127.0.0.1".into(),
            port: 3000,
            env: "test".into(),
            database_url: "sqlite::memory:".into(),
            db_pool_size: 1,
            jwt_secret: "test-secret-key-at-least-32-characters-long".into(),
            jwt_access_expires: 900,
            jwt_refresh_expires: 604800,
            upload_dir: "./uploads".into(),
            max_upload_size: 5242880,
            static_dir: "./static".into(),
            base_url: "http://localhost:3000".into(),
            cors_origins: None,
            tls_cert_path: None,
            tls_key_path: None,
            plugin_dir: None,
            plugin_hot_reload: false,
            plugin_max_memory_mb: 32,
            plugin_default_timeout_ms: 5000,
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
        })
    }

    #[test]
    fn get_config_value_returns_known_keys() {
        let config = make_test_config();
        assert_eq!(
            get_config_value(&config, "app.host"),
            Some("127.0.0.1".into())
        );
        assert_eq!(get_config_value(&config, "app.port"), Some("3000".into()));
        assert_eq!(get_config_value(&config, "app.env"), Some("test".into()));
        assert_eq!(
            get_config_value(&config, "app.base_url"),
            Some("http://localhost:3000".into())
        );
        assert!(get_config_value(&config, "jwt.secret").is_none());
        assert!(get_config_value(&config, "database_url").is_none());
    }

    #[test]
    fn host_context_get_config_checks_permissions() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(ctx.get_config("app.env").is_some());
        assert!(ctx.get_config("unknown.key").is_none());
    }

    #[test]
    fn host_context_http_get_blocked_without_permission() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.http_get("https://evil.com");
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_context_db_query_rejects_non_select() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_query("DELETE FROM posts");
        assert!(result.contains("error"));
        assert!(!result.contains("status"));
    }

    #[test]
    fn host_context_get_data_returns_none_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(ctx.get_data("key").is_none());
    }

    #[test]
    fn host_context_set_data_returns_false_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(!ctx.set_data("key", "val"));
    }

    #[test]
    fn host_context_get_post_returns_none_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(ctx.get_post("slug").is_none());
    }

    #[test]
    fn host_context_db_query_returns_error_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_query("SELECT 1");
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_context_log_does_not_panic() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        ctx.log("info", "hello");
        ctx.log("warn", "warning");
        ctx.log("error", "error");
    }

    #[test]
    fn host_context_http_post_blocked_without_permission() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.http_post("https://evil.com", "{}");
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_context_get_config_with_restricted_permissions() {
        let config = make_test_config();
        let perms = Permissions {
            config: vec!["seo.*".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert!(ctx.get_config("seo.title").is_none()); // seo.title doesn't exist
        assert!(ctx.get_config("app.env").is_none()); // blocked by permission
    }

    #[test]
    fn host_context_db_query_rejects_write() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(
            ctx.db_query("INSERT INTO posts VALUES(1)")
                .contains("error")
        );
        assert!(ctx.db_query("UPDATE posts SET title='x'").contains("error"));
        assert!(ctx.db_query("DELETE FROM posts").contains("error"));
    }

    #[test]
    fn host_context_db_query_table_permission_blocked() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        // No pool → first check is "no database access", but even with pool it should fail
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_query("SELECT * FROM posts");
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_context_get_post_blocked_without_db_permission() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert!(ctx.get_post("any-slug").is_none());
    }

    #[test]
    fn host_context_plugin_id_accessor() {
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "my-plugin".into(),
            Permissions::default(),
            None,
        );
        assert_eq!(ctx.plugin_id(), "my-plugin");
    }

    #[test]
    fn host_context_max_memory_bytes_default() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert_eq!(ctx.max_memory_bytes(), 32 * 1024 * 1024);
    }

    #[test]
    fn host_context_max_memory_bytes_custom() {
        let config = make_test_config();
        let perms = Permissions {
            max_memory_mb: Some(64),
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert_eq!(ctx.max_memory_bytes(), 64 * 1024 * 1024);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_set_data_with_real_db() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/003_plugin_storage.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "plugin-a".into(),
            Permissions::default(),
            Some(pool.clone()),
        );

        assert!(ctx.get_data("greeting").is_none());
        assert!(ctx.set_data("greeting", "hello world"));
        assert_eq!(ctx.get_data("greeting"), Some("hello world".into()));

        assert!(ctx.set_data("greeting", "updated"));
        assert_eq!(ctx.get_data("greeting"), Some("updated".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_isolation_between_plugins() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/003_plugin_storage.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let config1 = make_test_config();
        let config2 = make_test_config();
        let ctx_a = HostContext::new(
            "test",
            config1,
            "plugin-a".into(),
            Permissions::default(),
            Some(pool.clone()),
        );
        let ctx_b = HostContext::new(
            "test",
            config2,
            "plugin-b".into(),
            Permissions::default(),
            Some(pool.clone()),
        );

        ctx_a.set_data("key", "value-a");
        ctx_b.set_data("key", "value-b");

        assert_eq!(ctx_a.get_data("key"), Some("value-a".into()));
        assert_eq!(ctx_b.get_data("key"), Some("value-b".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_with_real_db() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["posts".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT COUNT(*) as cnt FROM posts");
        assert!(!result.contains("error"));
        assert!(result.contains("cnt"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_table_not_permitted() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT * FROM posts");
        assert!(result.contains("no read permission"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_wildcard_permission() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(include_str!("../../migrations/001_init.sql"))
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["*".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT COUNT(*) as cnt FROM posts");
        assert!(!result.contains("error"));
    }
}
