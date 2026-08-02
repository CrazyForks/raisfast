//! Lua host functions — engine binding layer
//!
//! Only responsible for binding the shared business logic of
//! [`HostContext`](super::host_common::HostContext) to the `PLUGIN_HOST_GLOBAL` property
//! of the Lua global table.

use std::sync::Arc;

use mlua::{Lua, LuaSerdeExt};

use crate::config::app::AppConfig;
use crate::constants::PLUGIN_HOST_GLOBAL;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::host_common::HostContext;

/// Register host functions into the Lua global scope.
pub fn register_host_functions(
    lua: &Lua,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
) -> anyhow::Result<()> {
    let globals = lua.globals();
    let host = lua.create_table()?;

    let mut hc_inner = HostContext::new("lua", config, plugin_id, permissions, pool);
    if let Some(bus) = event_bus {
        hc_inner.set_event_bus(bus);
    }
    let host_ctx = Arc::new(hc_inner);

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
    let db_query_fn = lua.create_function(move |lua, (sql, params): (String, String)| {
        Ok(mlua::Value::String(
            lua.create_string(hc.db_query(&sql, &params))?,
        ))
    })?;
    host.set("dbQuery", db_query_fn)?;

    let hc = host_ctx.clone();
    let db_execute_fn = lua.create_function(move |lua, (sql, params): (String, String)| {
        Ok(mlua::Value::String(
            lua.create_string(hc.db_execute(&sql, &params))?,
        ))
    })?;
    host.set("dbExecute", db_execute_fn)?;

    let hc = host_ctx.clone();
    let db_begin_fn = lua.create_function(move |lua, ()| {
        Ok(mlua::Value::String(lua.create_string(hc.db_begin())?))
    })?;
    host.set("dbBegin", db_begin_fn)?;

    let hc = host_ctx.clone();
    let db_commit_fn = lua.create_function(move |lua, ()| {
        Ok(mlua::Value::String(lua.create_string(hc.db_commit())?))
    })?;
    host.set("dbCommit", db_commit_fn)?;

    let hc = host_ctx.clone();
    let db_rollback_fn = lua.create_function(move |lua, ()| {
        Ok(mlua::Value::String(lua.create_string(hc.db_rollback())?))
    })?;
    host.set("dbRollback", db_rollback_fn)?;

    let hc = host_ctx.clone();
    let db_insert_fn = lua.create_function(
        move |lua, (table, data, options): (String, String, String)| {
            Ok(mlua::Value::String(
                lua.create_string(hc.db_insert(&table, &data, &options))?,
            ))
        },
    )?;
    host.set("dbInsert", db_insert_fn)?;

    let hc = host_ctx.clone();
    let db_fetch_one_fn = lua.create_function(
        move |lua, (table, r#where, options): (String, String, String)| {
            Ok(mlua::Value::String(lua.create_string(
                hc.db_fetch_one(&table, &r#where, &options),
            )?))
        },
    )?;
    host.set("dbFetchOne", db_fetch_one_fn)?;

    let hc = host_ctx.clone();
    let db_fetch_all_fn = lua.create_function(
        move |lua, (table, r#where, options): (String, String, String)| {
            Ok(mlua::Value::String(lua.create_string(
                hc.db_fetch_all(&table, &r#where, &options),
            )?))
        },
    )?;
    host.set("dbFetchAll", db_fetch_all_fn)?;

    let hc = host_ctx.clone();
    let db_update_fn = lua.create_function(
        move |lua, (table, data, r#where, options): (String, String, String, String)| {
            Ok(mlua::Value::String(lua.create_string(
                hc.db_update(&table, &data, &r#where, &options),
            )?))
        },
    )?;
    host.set("dbUpdate", db_update_fn)?;

    let hc = host_ctx.clone();
    let db_delete_fn = lua.create_function(
        move |lua, (table, r#where, options): (String, String, String)| {
            Ok(mlua::Value::String(
                lua.create_string(hc.db_delete(&table, &r#where, &options))?,
            ))
        },
    )?;
    host.set("dbDelete", db_delete_fn)?;

    let hc = host_ctx.clone();
    let db_count_fn = lua.create_function(
        move |lua, (table, r#where, options): (String, String, String)| {
            Ok(mlua::Value::String(
                lua.create_string(hc.db_count(&table, &r#where, &options))?,
            ))
        },
    )?;
    host.set("dbCount", db_count_fn)?;

    let hc = host_ctx.clone();
    let db_increment_fn = lua.create_function(
        move |lua, (table, columns, r#where, options): (String, String, String, String)| {
            Ok(mlua::Value::String(lua.create_string(
                hc.db_increment(&table, &columns, &r#where, &options),
            )?))
        },
    )?;
    host.set("dbIncrement", db_increment_fn)?;

    let hc = host_ctx.clone();
    let db_sum_fn = lua.create_function(
        move |lua, (table, column, r#where, options): (String, String, String, String)| {
            Ok(mlua::Value::String(lua.create_string(
                hc.db_sum(&table, &column, &r#where, &options),
            )?))
        },
    )?;
    host.set("dbSum", db_sum_fn)?;

    let hc = host_ctx.clone();
    let db_group_by_fn = lua.create_function(move |lua, (table, options): (String, String)| {
        Ok(mlua::Value::String(
            lua.create_string(hc.db_group_by(&table, &options))?,
        ))
    })?;
    host.set("dbGroupBy", db_group_by_fn)?;

    let hc = host_ctx.clone();
    let vfs_read_fn = lua.create_function(move |lua, path: String| match hc.vfs_read(&path) {
        Ok(content) => Ok(mlua::Value::String(lua.create_string(&content)?)),
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("vfsRead", vfs_read_fn)?;

    let hc = host_ctx.clone();
    let vfs_write_fn = lua.create_function(move |_, (path, content): (String, String)| {
        Ok(hc.vfs_write(&path, &content).is_ok())
    })?;
    host.set("vfsWrite", vfs_write_fn)?;

    let hc = host_ctx.clone();
    let vfs_delete_fn =
        lua.create_function(move |_, path: String| Ok(hc.vfs_delete(&path).is_ok()))?;
    host.set("vfsDelete", vfs_delete_fn)?;

    let hc = host_ctx.clone();
    let vfs_exists_fn =
        lua.create_function(move |_lua, path: String| match hc.vfs_exists(&path) {
            Ok(true) => Ok(mlua::Value::Boolean(true)),
            Ok(false) => Ok(mlua::Value::Boolean(false)),
            Err(_) => Ok(mlua::Value::Nil),
        })?;
    host.set("vfsExists", vfs_exists_fn)?;

    let hc = host_ctx.clone();
    let vfs_list_fn = lua.create_function(move |lua, path: String| match hc.vfs_list(&path) {
        Ok(entries) => {
            let tbl = lua.create_table()?;
            for (i, entry) in entries.into_iter().enumerate() {
                tbl.set(i + 1, entry)?;
            }
            Ok(mlua::Value::Table(tbl))
        }
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("vfsList", vfs_list_fn)?;

    let hc = host_ctx.clone();
    let vfs_stat_fn = lua.create_function(move |lua, path: String| match hc.vfs_stat(&path) {
        Ok(json) => Ok(mlua::Value::String(lua.create_string(&json)?)),
        Err(_) => Ok(mlua::Value::Nil),
    })?;
    host.set("vfsStat", vfs_stat_fn)?;

    let hc = host_ctx.clone();
    let emit_event_fn = lua.create_function(move |lua, (event_type, data): (String, String)| {
        Ok(mlua::Value::String(
            lua.create_string(hc.emit_event(&event_type, &data))?,
        ))
    })?;
    host.set("emitEvent", emit_event_fn)?;

    let hc = host_ctx.clone();
    let new_id_fn = lua.create_function(move |lua, ()| -> mlua::Result<mlua::String> {
        lua.create_string(hc.new_uuid())
    })?;
    host.set("newId", new_id_fn)?;

    let hc = host_ctx.clone();
    let db_ph_fn = lua.create_function(move |lua, idx: usize| -> mlua::Result<mlua::String> {
        lua.create_string(hc.db_ph(idx))
    })?;
    host.set("dbPh", db_ph_fn)?;

    let json_encode_fn =
        lua.create_function(move |lua, val: mlua::Value| -> mlua::Result<String> {
            let json_val: serde_json::Value = lua.from_value(val)?;
            serde_json::to_string(&json_val)
                .map_err(|e| mlua::Error::runtime(format!("json encode error: {e}")))
        })?;
    host.set("jsonEncode", json_encode_fn)?;

    let json_decode_fn =
        lua.create_function(move |lua, json_str: String| -> mlua::Result<mlua::Value> {
            let json_val: serde_json::Value = serde_json::from_str(&json_str)
                .map_err(|e| mlua::Error::runtime(format!("json decode error: {e}")))?;
            lua.to_value(&json_val)
        })?;
    host.set("jsonDecode", json_decode_fn)?;

    globals.set(PLUGIN_HOST_GLOBAL, host)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
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
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();

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
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let get_cfg_fn: mlua::Function = host.get("getConfig").unwrap();

        let env: String = get_cfg_fn.call(("app.env",)).unwrap();
        assert_eq!(env, "test");

        let port: String = get_cfg_fn.call(("app.port",)).unwrap();
        assert_eq!(port, "9898");

        let unknown: mlua::Value = get_cfg_fn.call(("nonexistent.key",)).unwrap();
        assert!(unknown.is_nil());
    }

    #[test]
    fn host_http_get_blocked_without_permission() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let http_fn: mlua::Function = host.get("httpGet").unwrap();

        let result: String = http_fn.call(("https://evil.com",)).unwrap();
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_http_post_blocked_without_permission() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let http_fn: mlua::Function = host.get("httpPost").unwrap();

        let result: String = http_fn.call(("https://evil.com", "{}")).unwrap();
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_get_data_returns_nil_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let get_data_fn: mlua::Function = host.get("getData").unwrap();

        let result: mlua::Value = get_data_fn.call(("some.key",)).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn host_set_data_returns_false_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let set_data_fn: mlua::Function = host.get("setData").unwrap();

        let result: bool = set_data_fn.call(("key", "val")).unwrap();
        assert!(!result);
    }

    #[test]
    fn host_get_post_returns_nil_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let get_post_fn: mlua::Function = host.get("getPost").unwrap();

        let result: mlua::Value = get_post_fn.call(("some-slug",)).unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn host_db_query_returns_error_without_pool() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let db_fn: mlua::Function = host.get("dbQuery").unwrap();

        let result: String = db_fn.call(("SELECT 1", "[]")).unwrap();
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_db_query_rejects_non_select() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        let db_fn: mlua::Function = host.get("dbQuery").unwrap();

        let result: String = db_fn.call(("DELETE FROM posts", "[]")).unwrap();
        assert!(result.contains("only SELECT"));
    }

    #[test]
    fn host_all_functions_registered() {
        let lua = create_sandboxed_lua();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&lua, config, "test-plugin".into(), perms, None, None).unwrap();

        let globals = lua.globals();
        let host: mlua::Table = globals.get(PLUGIN_HOST_GLOBAL).unwrap();
        for name in [
            "log",
            "getConfig",
            "httpGet",
            "httpPost",
            "getData",
            "setData",
            "getPost",
            "dbQuery",
            "dbExecute",
            "dbBegin",
            "dbCommit",
            "dbRollback",
            "dbPh",
            "vfsRead",
            "vfsWrite",
            "vfsDelete",
            "vfsExists",
            "vfsList",
            "vfsStat",
        ] {
            let _: mlua::Function = host.get(name).unwrap();
        }
    }
}
