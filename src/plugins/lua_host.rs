//! Lua 宿主函数 — 引擎绑定层
//!
//! 仅负责将 [`HostContext`](super::host_common::HostContext) 的公共业务逻辑
//! 绑定到 Lua 全局 table 的 `Host` 属性上。

use std::sync::Arc;

use mlua::Lua;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::host_common::HostContext;

/// 注册宿主函数到 Lua 全局作用域。
pub fn register_host_functions(
    lua: &Lua,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
) -> anyhow::Result<()> {
    let globals = lua.globals();
    let host = lua.create_table()?;

    let host_ctx = Arc::new(HostContext::new(
        "lua",
        config,
        plugin_id,
        permissions,
        pool,
    ));

    let hc = host_ctx.clone();
    let log_fn = lua.create_function(move |_, (level, msg): (String, String)| {
        hc.log(&level, &msg);
        Ok(())
    })?;
    host.set("log", log_fn)?;

    let hc = host_ctx.clone();
    let get_config_fn = lua.create_function(move |lua, key: String| match hc.get_config(&key) {
        Some(val) => Ok(mlua::Value::String(lua.create_string(&val)?)),
        None => Ok(mlua::Value::Nil),
    })?;
    host.set("getConfig", get_config_fn)?;

    let hc = host_ctx.clone();
    let http_get_fn = lua.create_function(move |lua, url: String| {
        Ok(mlua::Value::String(lua.create_string(hc.http_get(&url))?))
    })?;
    host.set("httpGet", http_get_fn)?;

    let hc = host_ctx.clone();
    let http_post_fn = lua.create_function(move |lua, (url, body): (String, String)| {
        Ok(mlua::Value::String(
            lua.create_string(hc.http_post(&url, &body))?,
        ))
    })?;
    host.set("httpPost", http_post_fn)?;

    let hc = host_ctx.clone();
    let get_data_fn = lua.create_function(move |lua, key: String| match hc.get_data(&key) {
        Some(val) => Ok(mlua::Value::String(lua.create_string(&val)?)),
        None => Ok(mlua::Value::Nil),
    })?;
    host.set("getData", get_data_fn)?;

    let hc = host_ctx.clone();
    let set_data_fn = lua
        .create_function(move |_, (key, value): (String, String)| Ok(hc.set_data(&key, &value)))?;
    host.set("setData", set_data_fn)?;

    let hc = host_ctx.clone();
    let get_post_fn = lua.create_function(move |lua, slug: String| match hc.get_post(&slug) {
        Some(json) => Ok(mlua::Value::String(lua.create_string(&json)?)),
        None => Ok(mlua::Value::Nil),
    })?;
    host.set("getPost", get_post_fn)?;

    let hc = host_ctx.clone();
    let db_query_fn = lua.create_function(move |lua, sql: String| {
        Ok(mlua::Value::String(lua.create_string(hc.db_query(&sql))?))
    })?;
    host.set("dbQuery", db_query_fn)?;

    let hc = host_ctx.clone();
    let fs_read_fn = lua.create_function(move |lua, path: String| match hc.fs_read(&path) {
        Ok(content) => Ok(mlua::Value::String(lua.create_string(&content)?)),
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("fsRead", fs_read_fn)?;

    let hc = host_ctx.clone();
    let fs_write_fn = lua.create_function(move |_, (path, content): (String, String)| {
        Ok(hc.fs_write(&path, &content).is_ok())
    })?;
    host.set("fsWrite", fs_write_fn)?;

    let hc = host_ctx.clone();
    let fs_delete_fn =
        lua.create_function(move |_, path: String| Ok(hc.fs_delete(&path).is_ok()))?;
    host.set("fsDelete", fs_delete_fn)?;

    let hc = host_ctx.clone();
    let fs_exists_fn =
        lua.create_function(move |_lua, path: String| match hc.fs_exists(&path) {
            Ok(true) => Ok(mlua::Value::Boolean(true)),
            Ok(false) => Ok(mlua::Value::Boolean(false)),
            Err(_) => Ok(mlua::Value::Nil),
        })?;
    host.set("fsExists", fs_exists_fn)?;

    let hc = host_ctx.clone();
    let fs_list_fn = lua.create_function(move |lua, path: String| match hc.fs_list(&path) {
        Ok(entries) => {
            let tbl = lua.create_table()?;
            for (i, entry) in entries.into_iter().enumerate() {
                tbl.set(i + 1, entry)?;
            }
            Ok(mlua::Value::Table(tbl))
        }
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("fsList", fs_list_fn)?;

    let hc = host_ctx;
    let fs_stat_fn = lua.create_function(move |lua, path: String| match hc.fs_stat(&path) {
        Ok(json) => Ok(mlua::Value::String(lua.create_string(&json)?)),
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("fsStat", fs_stat_fn)?;

    globals.set("Host", host)?;
    Ok(())
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
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

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
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let get_cfg_fn: mlua::Function = host.get("getConfig").unwrap();

        let env: String = get_cfg_fn.call(("app.env",)).unwrap();
        assert_eq!(env, "test");

        let port: String = get_cfg_fn.call(("app.port",)).unwrap();
        assert_eq!(port, "3000");

        let unknown: mlua::Value = get_cfg_fn.call(("nonexistent.key",)).unwrap();
        assert!(unknown.is_nil());
    }

    #[test]
    fn host_http_get_blocked_without_permission() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let http_fn: mlua::Function = host.get("httpGet").unwrap();

        let result: String = http_fn.call(("https://evil.com",)).unwrap();
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_http_post_blocked_without_permission() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let http_fn: mlua::Function = host.get("httpPost").unwrap();

        let result: String = http_fn.call(("https://evil.com", "{}")).unwrap();
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_get_data_returns_nil_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let get_data_fn: mlua::Function = host.get("getData").unwrap();

        let result: mlua::Value = get_data_fn.call(("some.key",)).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn host_set_data_returns_false_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let set_data_fn: mlua::Function = host.get("setData").unwrap();

        let result: bool = set_data_fn.call(("key", "val")).unwrap();
        assert!(!result);
    }

    #[test]
    fn host_get_post_returns_nil_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let get_post_fn: mlua::Function = host.get("getPost").unwrap();

        let result: mlua::Value = get_post_fn.call(("some-slug",)).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn host_db_query_returns_error_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let db_fn: mlua::Function = host.get("dbQuery").unwrap();

        let result: String = db_fn.call(("SELECT 1",)).unwrap();
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_db_query_rejects_non_select() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        let db_fn: mlua::Function = host.get("dbQuery").unwrap();

        let result: String = db_fn.call(("DELETE FROM posts",)).unwrap();
        assert!(result.contains("only SELECT"));
    }

    #[test]
    fn host_all_functions_registered() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get("Host").unwrap();
        for name in [
            "log",
            "getConfig",
            "httpGet",
            "httpPost",
            "getData",
            "setData",
            "getPost",
            "dbQuery",
            "fsRead",
            "fsWrite",
            "fsDelete",
            "fsExists",
            "fsList",
            "fsStat",
        ] {
            let _: mlua::Function = host.get(name).unwrap();
        }
    }
}
