//! WASM 宿主函数 — Component Model 绑定层
//!
//! 通过 wasmtime 26 bindgen 生成的 Host trait 实现导入接口。
//! 所有函数委托给 HostContext 的业务逻辑方法。

use std::sync::Arc;

use crate::plugins::bindings::rust_blog::plugin_protocol::host_api::Host;
use crate::plugins::bindings::rust_blog::plugin_protocol::types::Host as TypesHost;
use crate::plugins::host_common::HostContext;

impl TypesHost for Arc<HostContext> {}

impl Host for Arc<HostContext> {
    fn log(&mut self, level: String, msg: String) {
        (**self).log(&level, &msg);
    }

    fn get_config(&mut self, key: String) -> Option<String> {
        (**self).get_config(&key)
    }

    fn http_get(&mut self, url: String) -> Option<String> {
        Some((**self).http_get(&url))
    }

    fn http_post(&mut self, url: String, body: String) -> Option<String> {
        Some((**self).http_post(&url, &body))
    }

    fn get_data(&mut self, key: String) -> Option<String> {
        (**self).get_data(&key)
    }

    fn set_data(&mut self, key: String, value: String) -> bool {
        (**self).set_data(&key, &value)
    }

    fn get_post(&mut self, slug: String) -> Option<String> {
        (**self).get_post(&slug)
    }

    fn db_query(&mut self, sql: String, params: Option<String>) -> String {
        (**self).db_query(&sql, params.as_deref())
    }

    fn db_execute(&mut self, sql: String, params: Option<String>) -> String {
        (**self).db_execute(&sql, params.as_deref())
    }

    fn db_begin(&mut self) -> String {
        (**self).db_begin()
    }

    fn db_commit(&mut self) -> String {
        (**self).db_commit()
    }

    fn db_rollback(&mut self) -> String {
        (**self).db_rollback()
    }

    fn fs_read(&mut self, path: String) -> Option<String> {
        (**self).fs_read(&path).ok()
    }

    fn fs_write(&mut self, path: String, content: String) -> bool {
        (**self).fs_write(&path, &content).is_ok()
    }

    fn fs_delete(&mut self, path: String) -> bool {
        (**self).fs_delete(&path).is_ok()
    }

    fn fs_exists(&mut self, path: String) -> bool {
        (**self).fs_exists(&path).unwrap_or(false)
    }

    fn fs_list(&mut self, path: String) -> Option<String> {
        (**self)
            .fs_list(&path)
            .ok()
            .map(|entries| serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
    }

    fn fs_stat(&mut self, path: String) -> Option<String> {
        (**self).fs_stat(&path).ok()
    }

    fn emit_event(&mut self, event_type: String, data: String) -> String {
        (**self).emit_event(&event_type, &data)
    }
}
