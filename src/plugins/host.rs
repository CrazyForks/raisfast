//! WASM 宿主函数 — Component Model 绑定层
//!
//! 通过 wasmtime component Linker 注册 `host-api` 接口实现。
//! 所有函数通过 `Caller<'_, Arc<HostContext>>` 访问公共业务逻辑。

use std::sync::Arc;

use wasmtime::component::Linker;

use crate::plugins::host_common::HostContext;

/// 注册所有 Host Functions 到 component Linker。
pub fn register_host_functions(linker: &mut Linker<Arc<HostContext>>) -> anyhow::Result<()> {
    linker
        .root()
        .instance("host-api")?
        .func_wrap("log", host_log)?
        .func_wrap("get-config", host_get_config)?
        .func_wrap("http-get", host_http_get)?
        .func_wrap("http-post", host_http_post)?
        .func_wrap("get-data", host_get_data)?
        .func_wrap("set-data", host_set_data)?
        .func_wrap("get-post", host_get_post)?
        .func_wrap("db-query", host_db_query)?
        .func_wrap("db-execute", host_db_execute)?
        .func_wrap("db-begin", host_db_begin)?
        .func_wrap("db-commit", host_db_commit)?
        .func_wrap("db-rollback", host_db_rollback)?
        .func_wrap("fs-read", host_fs_read)?
        .func_wrap("fs-write", host_fs_write)?
        .func_wrap("fs-delete", host_fs_delete)?
        .func_wrap("fs-exists", host_fs_exists)?
        .func_wrap("fs-list", host_fs_list)?
        .func_wrap("fs-stat", host_fs_stat)?;
    Ok(())
}

fn host_log(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    level: String,
    msg: String,
) {
    caller.data().log(&level, &msg);
}

fn host_get_config(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    key: String,
) -> Option<String> {
    caller.data().get_config(&key)
}

fn host_http_get(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    url: String,
) -> Option<String> {
    let result = caller.data().http_get(&url);
    Some(result)
}

fn host_http_post(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    url: String,
    body: String,
) -> Option<String> {
    let result = caller.data().http_post(&url, &body);
    Some(result)
}

fn host_get_data(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    key: String,
) -> Option<String> {
    caller.data().get_data(&key)
}

fn host_set_data(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    key: String,
    value: String,
) -> bool {
    caller.data().set_data(&key, &value)
}

fn host_get_post(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    slug: String,
) -> Option<String> {
    caller.data().get_post(&slug)
}

fn host_db_query(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    sql: String,
) -> String {
    caller.data().db_query(&sql)
}

fn host_db_execute(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    sql: String,
    params: Option<String>,
) -> String {
    caller.data().db_execute(&sql, params.as_deref())
}

fn host_db_begin(mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>) -> String {
    caller.data().db_begin()
}

fn host_db_commit(mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>) -> String {
    caller.data().db_commit()
}

fn host_db_rollback(mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>) -> String {
    caller.data().db_rollback()
}

fn host_fs_read(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
) -> Option<String> {
    caller.data().fs_read(&path).ok()
}

fn host_fs_write(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
    content: String,
) -> bool {
    caller.data().fs_write(&path, &content).is_ok()
}

fn host_fs_delete(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
) -> bool {
    caller.data().fs_delete(&path).is_ok()
}

fn host_fs_exists(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
) -> bool {
    caller.data().fs_exists(&path).unwrap_or(false)
}

fn host_fs_list(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
) -> Option<String> {
    caller
        .data()
        .fs_list(&path)
        .ok()
        .map(|entries| serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
}

fn host_fs_stat(
    mut caller: wasmtime::StoreContextMut<'_, Arc<HostContext>>,
    path: String,
) -> Option<String> {
    caller.data().fs_stat(&path).ok()
}
