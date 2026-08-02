//! JS host functions — engine binding layer
//!
//! Only responsible for binding the shared business logic of
//! [`HostContext`](super::host_common::HostContext) to the `PLUGIN_HOST_GLOBAL` property
//! of the `QuickJS` global object.

use std::sync::Arc;

use rquickjs::{Function, Object};

use crate::config::app::AppConfig;
use crate::constants::PLUGIN_HOST_GLOBAL;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::host_common::HostContext;

/// Register host functions into the JS global scope.
pub fn register_host_functions(
    ctx: rquickjs::Ctx,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
    event_bus: Option<crate::eventbus::EventBus>,
) -> rquickjs::Result<()> {
    let global = ctx.globals();
    let host = Object::new(ctx.clone())?;

    let mut hc_inner = HostContext::new("js", config, plugin_id, permissions, pool);
    if let Some(bus) = event_bus {
        hc_inner.set_event_bus(bus);
    }
    let host_ctx = Arc::new(hc_inner);

    let hc = host_ctx.clone();
    let log_fn = Function::new(ctx.clone(), move |level: String, msg: String| {
        hc.log(&level, &msg);
    })?;
    host.set("log", log_fn)?;

    let hc = host_ctx.clone();
    let get_config_fn = Function::new(ctx.clone(), move |key: String| -> Option<String> {
        hc.get_config(&key)
    })?;
    host.set("getConfig", get_config_fn)?;

    let hc = host_ctx.clone();
    let http_get_fn = Function::new(ctx.clone(), move |url: String| -> String {
        hc.http_get(&url)
    })?;
    host.set("httpGet", http_get_fn)?;

    let hc = host_ctx.clone();
    let http_post_fn = Function::new(ctx.clone(), move |url: String, body: String| -> String {
        hc.http_post(&url, &body)
    })?;
    host.set("httpPost", http_post_fn)?;

    let hc = host_ctx.clone();
    let get_data_fn = Function::new(ctx.clone(), move |key: String| -> Option<String> {
        hc.get_data(&key)
    })?;
    host.set("getData", get_data_fn)?;

    let hc = host_ctx.clone();
    let set_data_fn = Function::new(ctx.clone(), move |key: String, value: String| -> bool {
        hc.set_data(&key, &value)
    })?;
    host.set("setData", set_data_fn)?;

    let hc = host_ctx.clone();
    let get_post_fn = Function::new(ctx.clone(), move |slug: String| -> Option<String> {
        hc.get_post(&slug)
    })?;
    host.set("getPost", get_post_fn)?;

    let hc = host_ctx.clone();
    let db_query_fn = Function::new(ctx.clone(), move |sql: String, params: String| -> String {
        hc.db_query(&sql, &params)
    })?;
    host.set("dbQuery", db_query_fn)?;

    let hc = host_ctx.clone();
    let db_execute_fn = Function::new(ctx.clone(), move |sql: String, params: String| -> String {
        hc.db_execute(&sql, &params)
    })?;
    host.set("dbExecute", db_execute_fn)?;

    let hc = host_ctx.clone();
    let db_begin_fn = Function::new(ctx.clone(), move || -> String { hc.db_begin() })?;
    host.set("dbBegin", db_begin_fn)?;

    let hc = host_ctx.clone();
    let db_commit_fn = Function::new(ctx.clone(), move || -> String { hc.db_commit() })?;
    host.set("dbCommit", db_commit_fn)?;

    let hc = host_ctx.clone();
    let db_rollback_fn = Function::new(ctx.clone(), move || -> String { hc.db_rollback() })?;
    host.set("dbRollback", db_rollback_fn)?;

    let hc = host_ctx.clone();
    let db_insert_fn = Function::new(
        ctx.clone(),
        move |table: String, data: String, options: String| -> String {
            hc.db_insert(&table, &data, &options)
        },
    )?;
    host.set("dbInsert", db_insert_fn)?;

    let hc = host_ctx.clone();
    let db_fetch_one_fn = Function::new(
        ctx.clone(),
        move |table: String, r#where: String, options: String| -> String {
            hc.db_fetch_one(&table, &r#where, &options)
        },
    )?;
    host.set("dbFetchOne", db_fetch_one_fn)?;

    let hc = host_ctx.clone();
    let db_fetch_all_fn = Function::new(
        ctx.clone(),
        move |table: String, r#where: String, options: String| -> String {
            hc.db_fetch_all(&table, &r#where, &options)
        },
    )?;
    host.set("dbFetchAll", db_fetch_all_fn)?;

    let hc = host_ctx.clone();
    let db_update_fn = Function::new(
        ctx.clone(),
        move |table: String, data: String, r#where: String, options: String| -> String {
            hc.db_update(&table, &data, &r#where, &options)
        },
    )?;
    host.set("dbUpdate", db_update_fn)?;

    let hc = host_ctx.clone();
    let db_delete_fn = Function::new(
        ctx.clone(),
        move |table: String, r#where: String, options: String| -> String {
            hc.db_delete(&table, &r#where, &options)
        },
    )?;
    host.set("dbDelete", db_delete_fn)?;

    let hc = host_ctx.clone();
    let db_count_fn = Function::new(
        ctx.clone(),
        move |table: String, r#where: String, options: String| -> String {
            hc.db_count(&table, &r#where, &options)
        },
    )?;
    host.set("dbCount", db_count_fn)?;

    let hc = host_ctx.clone();
    let db_increment_fn = Function::new(
        ctx.clone(),
        move |table: String, columns: String, r#where: String, options: String| -> String {
            hc.db_increment(&table, &columns, &r#where, &options)
        },
    )?;
    host.set("dbIncrement", db_increment_fn)?;

    let hc = host_ctx.clone();
    let db_sum_fn = Function::new(
        ctx.clone(),
        move |table: String, column: String, r#where: String, options: String| -> String {
            hc.db_sum(&table, &column, &r#where, &options)
        },
    )?;
    host.set("dbSum", db_sum_fn)?;

    let hc = host_ctx.clone();
    let db_group_by_fn = Function::new(
        ctx.clone(),
        move |table: String, options: String| -> String { hc.db_group_by(&table, &options) },
    )?;
    host.set("dbGroupBy", db_group_by_fn)?;

    let hc = host_ctx.clone();
    let vfs_read_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.vfs_read(&path).ok()
    })?;
    host.set("vfsRead", vfs_read_fn)?;

    let hc = host_ctx.clone();
    let vfs_write_fn = Function::new(ctx.clone(), move |path: String, content: String| -> bool {
        hc.vfs_write(&path, &content).is_ok()
    })?;
    host.set("vfsWrite", vfs_write_fn)?;

    let hc = host_ctx.clone();
    let vfs_delete_fn = Function::new(ctx.clone(), move |path: String| -> bool {
        hc.vfs_delete(&path).is_ok()
    })?;
    host.set("vfsDelete", vfs_delete_fn)?;

    let hc = host_ctx.clone();
    let vfs_exists_fn = Function::new(ctx.clone(), move |path: String| -> Option<bool> {
        hc.vfs_exists(&path).ok()
    })?;
    host.set("vfsExists", vfs_exists_fn)?;

    let hc = host_ctx.clone();
    let vfs_list_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.vfs_list(&path).ok().map(|entries| entries.join(","))
    })?;
    host.set("vfsList", vfs_list_fn)?;

    let hc = host_ctx.clone();
    let vfs_stat_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.vfs_stat(&path).ok()
    })?;
    host.set("vfsStat", vfs_stat_fn)?;

    let hc = host_ctx.clone();
    let new_id_fn = Function::new(ctx.clone(), move || -> String { hc.new_uuid() })?;
    host.set("newId", new_id_fn)?;

    let hc = host_ctx.clone();
    let db_ph_fn = Function::new(ctx.clone(), move |idx: usize| -> String { hc.db_ph(idx) })?;
    host.set("dbPh", db_ph_fn)?;

    let hc = host_ctx;
    let emit_event_fn = Function::new(ctx, move |event_type: String, data: String| -> String {
        hc.emit_event(&event_type, &data)
    })?;
    host.set("emitEvent", emit_event_fn)?;

    global.set(PLUGIN_HOST_GLOBAL, host)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rquickjs::{AsyncContext, AsyncRuntime};

    fn make_test_config() -> Arc<AppConfig> {
        Arc::new(AppConfig::test_defaults())
    }

    #[tokio::test]
    async fn register_host_functions_in_context() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();

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
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let get_cfg_fn: Function = host.get("getConfig").unwrap();

            let env: Option<String> = get_cfg_fn.call(("app.env",)).unwrap();
            assert_eq!(env, Some("test".to_string()));

            let port: Option<String> = get_cfg_fn.call(("app.port",)).unwrap();
            assert_eq!(port, Some("9898".to_string()));
        })
        .await;
    }

    #[tokio::test]
    async fn host_http_get_blocked_without_permission() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let http_fn: Function = host.get("httpGet").unwrap();

            let result: String = http_fn.call(("https://evil.com",)).unwrap();
            assert!(result.contains("not allowed"));
        })
        .await;
    }

    #[tokio::test]
    async fn host_http_post_blocked_without_permission() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let http_fn: Function = host.get("httpPost").unwrap();

            let result: String = http_fn.call(("https://evil.com", "{}")).unwrap();
            assert!(result.contains("not allowed"));
        })
        .await;
    }

    #[tokio::test]
    async fn host_get_data_returns_none_without_pool() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let get_data_fn: Function = host.get("getData").unwrap();

            let result: Option<String> = get_data_fn.call(("some.key",)).unwrap();
            assert!(result.is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn host_set_data_returns_false_without_pool() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let set_data_fn: Function = host.get("setData").unwrap();

            let result: bool = set_data_fn.call(("key", "val")).unwrap();
            assert!(!result);
        })
        .await;
    }

    #[tokio::test]
    async fn host_get_post_returns_none_without_pool() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let get_post_fn: Function = host.get("getPost").unwrap();

            let result: Option<String> = get_post_fn.call(("some-slug",)).unwrap();
            assert!(result.is_none());
        })
        .await;
    }

    #[tokio::test]
    async fn host_db_query_returns_error_without_pool() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let db_fn: Function = host.get("dbQuery").unwrap();

            let result: String = db_fn.call(("SELECT 1", "[]")).unwrap();
            assert!(result.contains("no database access"));
        })
        .await;
    }

    #[tokio::test]
    async fn host_db_query_rejects_non_select() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
            let db_fn: Function = host.get("dbQuery").unwrap();

            let result: String = db_fn.call(("DELETE FROM posts", "[]")).unwrap();
            assert!(result.contains("only SELECT"));
        })
        .await;
    }

    #[tokio::test]
    async fn host_all_functions_registered() {
        let runtime = AsyncRuntime::new().unwrap();
        let ctx = AsyncContext::full(&runtime).await.unwrap();
        let config = make_test_config();
        let perms = Permissions::default();

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get(PLUGIN_HOST_GLOBAL).unwrap();
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
                let _: Function = host.get(name).unwrap();
            }
        })
        .await;
    }
}
