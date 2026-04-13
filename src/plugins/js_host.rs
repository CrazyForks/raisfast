//! JS 宿主函数
//!
//! 注册到 JS 全局对象的宿主函数，供插件通过 `Host` 对象调用。
//! 当前提供 `Host.log()` 和 `Host.getConfig()`。

use std::sync::Arc;

use rquickjs::{Function, Object};

use crate::config::app::AppConfig;

/// 注册宿主函数到 JS 全局作用域。
///
/// 在每个插件上下文中调用，注入全局 `Host` 对象。
/// `config` 用于 `Host.getConfig()` 运行时读取配置值。
pub fn register_host_functions(ctx: rquickjs::Ctx, config: Arc<AppConfig>) -> rquickjs::Result<()> {
    let global = ctx.globals();
    let host = Object::new(ctx.clone())?;

    let log_fn = Function::new(ctx.clone(), |level: String, msg: String| {
        match level.as_str() {
            "warn" => tracing::warn!("[plugin:js] {msg}"),
            "error" => tracing::error!("[plugin:js] {msg}"),
            _ => tracing::info!("[plugin:js] {msg}"),
        }
    })?;
    host.set("log", log_fn)?;

    let get_config_fn = Function::new(ctx, move |key: String| -> Option<String> {
        get_config_value(&config, &key)
    })?;
    host.set("getConfig", get_config_fn)?;

    global.set("Host", host)?;
    Ok(())
}

/// 根据 key 路径从 AppConfig 读取配置值。
///
/// 支持的 key：
/// - `app.host` / `app.port` / `app.env` / `app.base_url`
/// - `jwt.access_expires` / `jwt.refresh_expires`
/// - `upload.dir` / `upload.max_size`
/// - `plugin.max_memory_mb` / `plugin.default_timeout_ms`
fn get_config_value(config: &AppConfig, key: &str) -> Option<String> {
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
    use rquickjs::{AsyncContext, AsyncRuntime};

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

    #[tokio::test]
    async fn register_host_functions_in_context() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config).unwrap();

            let global = ctx.globals();
            let host: Object = global.get("Host").unwrap();

            let log_fn: Function = host.get("log").unwrap();
            let _: () = log_fn.call(("info", "test")).unwrap();

            let get_cfg_fn: Function = host.get("getConfig").unwrap();
            let result: Option<String> = get_cfg_fn.call(("some.key",)).unwrap();
            assert!(result.is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn host_get_config_returns_known_values() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config).unwrap();

            let global = ctx.globals();
            let host: Object = global.get("Host").unwrap();
            let get_cfg_fn: Function = host.get("getConfig").unwrap();

            let env: Option<String> = get_cfg_fn.call(("app.env",)).unwrap();
            assert_eq!(env, Some("test".to_string()));

            let port: Option<String> = get_cfg_fn.call(("app.port",)).unwrap();
            assert_eq!(port, Some("3000".to_string()));

            let base_url: Option<String> = get_cfg_fn.call(("app.base_url",)).unwrap();
            assert_eq!(base_url, Some("http://localhost:3000".to_string()));

            let unknown: Option<String> = get_cfg_fn.call(("nonexistent.key",)).unwrap();
            assert!(unknown.is_none());
        })
        .await;
    }

    #[test]
    fn get_config_value_all_keys() {
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
        assert_eq!(
            get_config_value(&config, "jwt.access_expires"),
            Some("900".into())
        );
        assert_eq!(
            get_config_value(&config, "jwt.refresh_expires"),
            Some("604800".into())
        );
        assert_eq!(
            get_config_value(&config, "upload.dir"),
            Some("./uploads".into())
        );
        assert_eq!(
            get_config_value(&config, "upload.max_size"),
            Some("5242880".into())
        );
        assert_eq!(
            get_config_value(&config, "plugin.max_memory_mb"),
            Some("32".into())
        );
        assert_eq!(
            get_config_value(&config, "plugin.default_timeout_ms"),
            Some("5000".into())
        );
        assert!(get_config_value(&config, "jwt.secret").is_none());
        assert!(get_config_value(&config, "database_url").is_none());
    }
}
