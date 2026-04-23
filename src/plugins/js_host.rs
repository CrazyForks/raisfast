//! JS 宿主函数 — 引擎绑定层
//!
//! 仅负责将 [`HostContext`](super::host_common::HostContext) 的公共业务逻辑
//! 绑定到 `QuickJS` 全局对象的 `Host` 属性上。

use std::sync::Arc;

use rquickjs::{Function, Object};

use crate::config::app::AppConfig;
use crate::db::Pool;
use crate::plugins::Permissions;
use crate::plugins::host_common::HostContext;

/// 注册宿主函数到 JS 全局作用域。
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
    let db_query_fn = Function::new(
        ctx.clone(),
        move |sql: String, params: Option<String>| -> String {
            hc.db_query(&sql, params.as_deref())
        },
    )?;
    host.set("dbQuery", db_query_fn)?;

    let hc = host_ctx.clone();
    let db_execute_fn = Function::new(
        ctx.clone(),
        move |sql: String, params: Option<String>| -> String {
            hc.db_execute(&sql, params.as_deref())
        },
    )?;
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
    let fs_read_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.fs_read(&path).ok()
    })?;
    host.set("fsRead", fs_read_fn)?;

    let hc = host_ctx.clone();
    let fs_write_fn = Function::new(ctx.clone(), move |path: String, content: String| -> bool {
        hc.fs_write(&path, &content).is_ok()
    })?;
    host.set("fsWrite", fs_write_fn)?;

    let hc = host_ctx.clone();
    let fs_delete_fn = Function::new(ctx.clone(), move |path: String| -> bool {
        hc.fs_delete(&path).is_ok()
    })?;
    host.set("fsDelete", fs_delete_fn)?;

    let hc = host_ctx.clone();
    let fs_exists_fn = Function::new(ctx.clone(), move |path: String| -> Option<bool> {
        hc.fs_exists(&path).ok()
    })?;
    host.set("fsExists", fs_exists_fn)?;

    let hc = host_ctx.clone();
    let fs_list_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.fs_list(&path).ok().map(|entries| entries.join(","))
    })?;
    host.set("fsList", fs_list_fn)?;

    let hc = host_ctx.clone();
    let fs_stat_fn = Function::new(ctx.clone(), move |path: String| -> Option<String> {
        hc.fs_stat(&path).ok()
    })?;
    host.set("fsStat", fs_stat_fn)?;

    let hc = host_ctx;
    let emit_event_fn = Function::new(ctx, move |event_type: String, data: String| -> String {
        hc.emit_event(&event_type, &data)
    })?;
    host.set("emitEvent", emit_event_fn)?;

    global.set("Host", host)?;
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
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };

        ctx.with(|ctx| {
            register_host_functions(ctx.clone(), config, "test-plugin".into(), perms, None, None)
                .unwrap();

            let global = ctx.globals();
            let host: Object = global.get("Host").unwrap();
            let get_cfg_fn: Function = host.get("getConfig").unwrap();

            let env: Option<String> = get_cfg_fn.call(("app.env",)).unwrap();
            assert_eq!(env, Some("test".to_string()));

            let port: Option<String> = get_cfg_fn.call(("app.port",)).unwrap();
            assert_eq!(port, Some("3000".to_string()));
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
            let host: Object = global.get("Host").unwrap();
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
            let host: Object = global.get("Host").unwrap();
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
            let host: Object = global.get("Host").unwrap();
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
            let host: Object = global.get("Host").unwrap();
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
            let host: Object = global.get("Host").unwrap();
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
            let host: Object = global.get("Host").unwrap();
            let db_fn: Function = host.get("dbQuery").unwrap();

            let result: String = db_fn.call(("SELECT 1", rquickjs::Undefined)).unwrap();
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
            let host: Object = global.get("Host").unwrap();
            let db_fn: Function = host.get("dbQuery").unwrap();

            let result: String = db_fn
                .call(("DELETE FROM posts", rquickjs::Undefined))
                .unwrap();
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
            let host: Object = global.get("Host").unwrap();
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
                "fsRead",
                "fsWrite",
                "fsDelete",
                "fsExists",
                "fsList",
                "fsStat",
            ] {
                let _: Function = host.get(name).unwrap();
            }
        })
        .await;
    }
}
