//! WASM host functions — Component Model binding layer
//!
//! Host trait implementation for the import interface generated via wasmtime 26 bindgen.
//! All functions delegate to the business logic methods of HostContext.

use std::sync::Arc;

use crate::plugins::bindings::raisfast::plugin_wit::host_api::Host;
use crate::plugins::bindings::raisfast::plugin_wit::types::Host as TypesHost;
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
        let p = params.as_deref().unwrap_or("");
        (**self).db_query(&sql, p)
    }

    fn db_execute(&mut self, sql: String, params: Option<String>) -> String {
        let p = params.as_deref().unwrap_or("");
        (**self).db_execute(&sql, p)
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

    fn vfs_read(&mut self, path: String) -> Option<String> {
        (**self).vfs_read(&path).ok()
    }

    fn vfs_write(&mut self, path: String, content: String) -> bool {
        (**self).vfs_write(&path, &content).is_ok()
    }

    fn vfs_delete(&mut self, path: String) -> bool {
        (**self).vfs_delete(&path).is_ok()
    }

    fn vfs_exists(&mut self, path: String) -> bool {
        (**self).vfs_exists(&path).unwrap_or(false)
    }

    fn vfs_list(&mut self, path: String) -> Option<String> {
        (**self)
            .vfs_list(&path)
            .ok()
            .map(|entries| serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
    }

    fn vfs_stat(&mut self, path: String) -> Option<String> {
        (**self).vfs_stat(&path).ok()
    }

    fn emit_event(&mut self, event_type: String, data: String) -> String {
        (**self).emit_event(&event_type, &data)
    }
}
