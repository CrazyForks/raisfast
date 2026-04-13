//! Lua 宿主函数
//!
//! 注册到 Lua 全局 table 的宿主函数，供插件通过 `Host` table 调用。
//! 当前提供 `Host.log()` 和 `Host.getConfig()`。

use std::sync::Arc;

use mlua::Lua;

use crate::config::app::AppConfig;

/// 注册宿主函数到 Lua 全局作用域。
///
/// 在每个插件 Lua 状态中调用，注入全局 `Host` table。
pub fn register_host_functions(lua: &Lua, config: Arc<AppConfig>) -> anyhow::Result<()> {
    let globals = lua.globals();
    let host = lua.create_table()?;

    let log_fn = lua.create_function(|_, (level, msg): (String, String)| {
        match level.as_str() {
            "warn" => tracing::warn!("[plugin:lua] {msg}"),
            "error" => tracing::error!("[plugin:lua] {msg}"),
            _ => tracing::info!("[plugin:lua] {msg}"),
        }
        Ok(())
    })?;
    host.set("log", log_fn)?;

    let get_config_fn =
        lua.create_function(
            move |lua, key: String| match get_config_value(&config, &key) {
                Some(val) => Ok(mlua::Value::String(lua.create_string(&val)?)),
                None => Ok(mlua::Value::Nil),
            },
        )?;
    host.set("getConfig", get_config_fn)?;

    globals.set("Host", host)?;
    Ok(())
}

/// 根据 key 路径从 AppConfig 读取配置值
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

    fn create_sandboxed_lua() -> Lua {
        Lua::new_with(
            mlua::StdLib::TABLE | mlua::StdLib::STRING | mlua::StdLib::MATH,
            mlua::LuaOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn register_host_functions_in_context() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        register_host_functions(&lua, config).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();

        let log_fn: mlua::Function = host.get("log").unwrap();
        let _: () = log_fn.call(("info", "test")).unwrap();

        let get_cfg_fn: mlua::Function = host.get("getConfig").unwrap();
        let result: mlua::Value = get_cfg_fn.call(("some.key",)).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn host_get_config_returns_known_values() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        register_host_functions(&lua, config).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let get_cfg_fn: mlua::Function = host.get("getConfig").unwrap();

        let env: String = get_cfg_fn.call(("app.env",)).unwrap();
        assert_eq!(env, "test");

        let port: String = get_cfg_fn.call(("app.port",)).unwrap();
        assert_eq!(port, "3000");

        let base_url: String = get_cfg_fn.call(("app.base_url",)).unwrap();
        assert_eq!(base_url, "http://localhost:3000");

        let unknown: mlua::Value = get_cfg_fn.call(("nonexistent.key",)).unwrap();
        assert!(unknown.is_nil());
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
        assert!(get_config_value(&config, "jwt.secret").is_none());
    }
}
