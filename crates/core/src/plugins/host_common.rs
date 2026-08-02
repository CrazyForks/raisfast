//! Plugin host common logic
//!
//! Extracts shared business logic from both JS and Lua Host APIs into [`HostContext`].
//! Each engine's `register_host_functions` only handles engine-specific parameter binding;
//! all permission checks, HTTP requests, and DB queries are delegated to `HostContext` methods.

use crate::constants::PLUGIN_HOST_GLOBAL;

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::db::DbDriver;
use crate::db::{DbArguments, DbConnection, DbPoolConnection, DbQueryResult, DbRow, Pool};
use crate::event::Event;
use crate::eventbus::EventBus;
use crate::plugins::Permissions;
use crate::plugins::permissions::PermissionChecker;
use crate::plugins::vfs::VirtualFs;
use sqlx::Arguments;
use std::sync::Mutex;

/// Transaction state (holds an exclusive connection borrowed from the pool)
struct TxState {
    conn: DbPoolConnection,
}

struct WhereResult {
    clause: String,
    params: Vec<serde_json::Value>,
}

enum CrudTenant {
    Auto,
    Disabled,
    Explicit(String),
}

struct CrudOptions {
    tenant: CrudTenant,
    order_by: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl CrudOptions {
    fn parse(json: &str) -> Self {
        let mut opts = Self {
            tenant: CrudTenant::Auto,
            order_by: None,
            limit: None,
            offset: None,
        };
        let Ok(obj) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(json)
        else {
            return opts;
        };
        match obj.get("tenant") {
            Some(serde_json::Value::Bool(false)) => opts.tenant = CrudTenant::Disabled,
            Some(serde_json::Value::String(s)) => opts.tenant = CrudTenant::Explicit(s.clone()),
            _ => {}
        }
        if let Some(serde_json::Value::String(s)) = obj.get("order_by") {
            opts.order_by = Some(s.clone());
        }
        if let Some(serde_json::Value::Number(n)) = obj.get("limit") {
            opts.limit = n.as_u64().map(|v| v as usize);
        }
        if let Some(serde_json::Value::Number(n)) = obj.get("offset") {
            opts.offset = n.as_u64().map(|v| v as usize);
        }
        opts
    }

    fn tenant_is_disabled(&self) -> bool {
        matches!(self.tenant, CrudTenant::Disabled)
    }

    fn tenant_value_owned(&self) -> Option<String> {
        match &self.tenant {
            CrudTenant::Explicit(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// Plugin host context
///
/// Holds all shared state needed by a single plugin. Business logic methods are synchronous
/// (internally using `block_in_place` + `block_on` to execute async operations).
pub struct HostContext {
    pub runtime_label: &'static str,
    config: Arc<AppConfig>,
    plugin_id: String,
    permissions: Permissions,
    pool: Option<Pool>,
    tx: Mutex<Option<TxState>>,
    vfs: Option<Arc<VirtualFs>>,
    event_bus: Option<EventBus>,
    content_type_registry: Option<Arc<crate::content_type::ContentTypeRegistry>>,
}

impl Clone for HostContext {
    fn clone(&self) -> Self {
        Self {
            runtime_label: self.runtime_label,
            config: self.config.clone(),
            plugin_id: self.plugin_id.clone(),
            permissions: self.permissions.clone(),
            pool: self.pool.clone(),
            tx: Mutex::new(None),
            vfs: self.vfs.clone(),
            event_bus: self.event_bus.clone(),
            content_type_registry: self.content_type_registry.clone(),
        }
    }
}

impl HostContext {
    /// Create a new host context
    #[must_use]
    pub fn new(
        runtime_label: &'static str,
        config: Arc<AppConfig>,
        plugin_id: String,
        permissions: Permissions,
        pool: Option<Pool>,
    ) -> Self {
        Self {
            runtime_label,
            config,
            plugin_id,
            permissions,
            pool,
            tx: Mutex::new(None),
            vfs: None,
            event_bus: None,
            content_type_registry: None,
        }
    }

    /// Set the event bus (called after PluginManager initialization)
    pub fn set_event_bus(&mut self, bus: EventBus) {
        self.event_bus = Some(bus);
    }

    /// Set the content type registry (called after PluginManager initialization)
    pub fn set_content_type_registry(
        &mut self,
        reg: Arc<crate::content_type::ContentTypeRegistry>,
    ) {
        self.content_type_registry = Some(reg);
    }

    /// Return the plugin ID
    #[must_use]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    /// Return memory limit in bytes, based on permissions or default 32 MB
    #[must_use]
    pub fn max_memory_bytes(&self) -> usize {
        self.permissions
            .max_memory_mb
            .map_or(32 * 1024 * 1024, |mb| mb as usize * 1024 * 1024)
    }

    /// Log output
    pub fn new_uuid(&self) -> String {
        uuid::Uuid::now_v7().to_string()
    }

    /// Return the placeholder for the given index in the current database.
    ///
    /// - SQLite / MySQL: `?`
    /// - PostgreSQL: `$idx`
    ///
    /// Plugins use this function to build parameterized SQL:
    /// ```js
    /// const sql = `SELECT * FROM tags WHERE id = ${host.dbPh(1)} AND name = ${host.dbPh(2)}`;
    /// host.dbQuery(sql, JSON.stringify(["tag-1", "Rust"]));
    /// ```
    #[must_use]
    pub fn db_ph(&self, idx: usize) -> String {
        crate::db::Driver::ph(idx)
    }

    pub fn log(&self, level: &str, msg: &str) {
        let tag = self.runtime_label;
        match level {
            "warn" => tracing::warn!("[plugin:{tag}] {msg}"),
            "error" => tracing::error!("[plugin:{tag}] {msg}"),
            _ => tracing::info!("[plugin:{tag}] {msg}"),
        }
    }

    /// Read a config value
    #[must_use]
    pub fn get_config(&self, key: &str) -> Option<String> {
        if !PermissionChecker::is_config_key_allowed(&self.permissions, key) {
            return None;
        }
        get_config_value(&self.config, key)
    }

    /// HTTP GET request
    #[must_use]
    pub fn http_get(&self, url: &str) -> String {
        if !PermissionChecker::is_url_allowed(&self.permissions, url) {
            return format!("error: URL not allowed: {url}");
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::plugins::http_client::http_get(url)) {
                Ok(body) => body,
                Err(e) => format!("error: {e}"),
            }
        })
    }

    /// HTTP POST request
    #[must_use]
    pub fn http_post(&self, url: &str, body: &str) -> String {
        if !PermissionChecker::is_url_allowed(&self.permissions, url) {
            return format!("error: URL not allowed: {url}");
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::plugins::http_client::http_post(url, body, None)) {
                Ok(resp) => resp,
                Err(e) => format!("error: {e}"),
            }
        })
    }

    /// Read from plugin KV store
    pub fn get_data(&self, key: &str) -> Option<String> {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] {PLUGIN_HOST_GLOBAL}.getData called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return None;
        };
        let handle = tokio::runtime::Handle::current();
        let pid = self.plugin_id.clone();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::plugin_storage::get(pool, &pid, key)) {
                Ok(val) => val,
                Err(e) => {
                    tracing::error!("[plugin:{}] getData error: {e}", self.runtime_label);
                    None
                }
            }
        })
    }

    /// Write to plugin KV store
    pub fn set_data(&self, key: &str, value: &str) -> bool {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] {PLUGIN_HOST_GLOBAL}.setData called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return false;
        };
        let handle = tokio::runtime::Handle::current();
        let pid = self.plugin_id.clone();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::plugin_storage::set(
                pool, &pid, key, value, None,
            )) {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!("[plugin:{}] setData error: {e}", self.runtime_label);
                    false
                }
            }
        })
    }

    /// Get a post by slug (returns JSON)
    pub fn get_post(&self, slug: &str) -> Option<String> {
        let Some(pool) = &self.pool else {
            tracing::debug!(
                "[plugin:{}] {PLUGIN_HOST_GLOBAL}.getPost called by {} but no DB pool",
                self.runtime_label,
                self.plugin_id
            );
            return None;
        };
        if !PermissionChecker::is_table_readable(&self.permissions, "posts") {
            tracing::debug!(
                "[plugin:{}] getPost denied: no read:posts permission",
                self.runtime_label
            );
            return None;
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(crate::models::post::find_by_slug(
                pool,
                slug,
                Some(crate::constants::DEFAULT_TENANT),
            )) {
                Ok(Some(post)) => serde_json::to_string(&post).ok(),
                Ok(None) => None,
                Err(e) => {
                    tracing::error!("[plugin:{}] getPost error: {e}", self.runtime_label);
                    None
                }
            }
        })
    }

    /// Execute a read-only SQL query (returns a JSON array string).
    ///
    /// `params_json` is a JSON array string, corresponding in order to the `host.ph(N)` placeholders in the SQL.
    /// Example:
    /// ```js
    /// const sql = `SELECT * FROM tags WHERE id = ${host.ph(1)}`;
    /// host.dbQuery(sql, JSON.stringify(["tag-1"]));
    /// ```
    #[must_use]
    pub fn db_query(&self, sql: &str, params_json: &str) -> String {
        if !PermissionChecker::is_readonly_query(sql) {
            return "error: only SELECT queries are allowed".to_string();
        }
        let Some(pool) = &self.pool else {
            return "error: no database access".to_string();
        };
        let table = match crate::plugins::permissions::extract_table_name(sql) {
            Some(t) => t,
            None => return "error: cannot parse table name from SQL".to_string(),
        };
        if !PermissionChecker::is_table_readable(&self.permissions, &table) {
            return format!("error: no read permission for table: {table}");
        }
        let params = match Self::parse_params(params_json) {
            Ok(Some(p)) => p,
            Ok(None) => Vec::new(),
            Err(e) => return format!(r#"{{"error":"invalid params: {e}"}}"#),
        };
        let handle = tokio::runtime::Handle::current();
        let sql = sql.to_string();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let mut args = DbArguments::default();
                for p in &params {
                    Self::add_param(&mut args, p);
                }
                let rows = sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .fetch_all(pool)
                    .await?;
                let json = crate::plugins::rows_to_json(&rows);
                Ok::<_, sqlx::Error>(json)
            }) {
                Ok(json) => json,
                Err(_) => "error: database query failed".to_string(),
            }
        })
    }

    /// Execute a write SQL operation (INSERT/UPDATE/DELETE), returns a JSON result.
    ///
    /// `params_json` is a JSON array string, corresponding in order to the `host.ph(N)` placeholders in the SQL.
    /// Example:
    /// ```js
    /// const sql = `INSERT INTO tags (id, name) VALUES (${host.ph(1)}, ${host.ph(2)})`;
    /// host.dbExecute(sql, JSON.stringify(["tag-1", "Rust"]));
    /// ```
    #[must_use]
    pub fn db_execute(&self, sql: &str, params_json: &str) -> String {
        if !PermissionChecker::is_write_query(sql) {
            return r#"{"error":"only INSERT/UPDATE/DELETE are allowed"}"#.to_string();
        }
        if PermissionChecker::is_ddl_query(sql) {
            return r#"{"error":"DDL operations are not allowed"}"#.to_string();
        }
        let table = match crate::plugins::permissions::extract_write_table_name(sql) {
            Some(t) => t,
            None => return r#"{"error":"cannot parse table name from SQL"}"#.to_string(),
        };
        if !PermissionChecker::is_table_writable(&self.permissions, &table) {
            return format!(r#"{{"error":"no write permission for table: {table}"}}"#);
        }
        let params = match Self::parse_params(params_json) {
            Ok(Some(p)) => p,
            Ok(None) => Vec::new(),
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        let tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx_guard.is_some() {
            drop(tx_guard);
            let sql = sql.to_string();
            let handle = tokio::runtime::Handle::current();
            return tokio::task::block_in_place(|| {
                let mut tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
                let Some(tx_state) = tx_guard.as_mut() else {
                    return r#"{"error":"transaction lost"}"#.to_string();
                };
                let result: Result<DbQueryResult, sqlx::Error> =
                    build_and_exec(&mut tx_state.conn, &sql, &params, &handle);
                match result {
                    Ok(r) => {
                        let affected: u64 = r.rows_affected();
                        format!(r#"{{"rows_affected":{affected}}}"#)
                    }
                    Err(_) => r#"{"error":"database write failed"}"#.to_string(),
                }
            });
        }
        drop(tx_guard);

        let Some(pool) = &self.pool else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let handle = tokio::runtime::Handle::current();
        let sql = sql.to_string();
        tokio::task::block_in_place(|| {
            let mut args = DbArguments::default();
            for p in &params {
                Self::add_param(&mut args, p);
            }
            let result: Result<DbQueryResult, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .execute(pool)
                    .await
            });
            match result {
                Ok(r) => {
                    let affected: u64 = r.rows_affected();
                    format!(r#"{{"rows_affected":{affected}}}"#)
                }
                Err(_) => r#"{"error":"database write failed"}"#.to_string(),
            }
        })
    }

    /// Begin a database transaction.
    ///
    /// Acquires an exclusive connection from the pool and executes `BEGIN`.
    /// Only one active transaction is allowed at a time; repeated calls return an error.
    /// On plugin timeout or crash, [`HostContext::cleanup_tx`] will automatically roll back.
    #[must_use]
    pub fn db_begin(&self) -> String {
        let Some(pool) = &self.pool else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let mut tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx_guard.is_some() {
            return r#"{"error":"transaction already active"}"#.to_string();
        }
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let acquire: Result<DbPoolConnection, sqlx::Error> =
                handle.block_on(async { pool.acquire().await });
            match acquire {
                Ok(mut conn) => {
                    match handle.block_on(async {
                        sqlx::query::<crate::db::pool::Db>("BEGIN")
                            .execute(&mut *conn)
                            .await
                    }) {
                        Ok(_r) => {
                            let _: crate::db::pool::DbQueryResult = _r;
                            tracing::info!("[plugin:{}] transaction begun", self.plugin_id);
                            *tx_guard = Some(TxState { conn });
                            r#"{"ok":true}"#.to_string()
                        }
                        Err(e) => format!(r#"{{"error":"BEGIN failed: {e}"}}"#),
                    }
                }
                Err(e) => format!(r#"{{"error":"cannot acquire connection: {e}"}}"#),
            }
        })
    }

    /// Commit the current transaction and release the connection.
    #[must_use]
    pub fn db_commit(&self) -> String {
        let mut tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut tx_state) = tx_guard.take() else {
            return r#"{"error":"no active transaction"}"#.to_string();
        };
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                sqlx::query::<crate::db::pool::Db>("COMMIT")
                    .execute(&mut *tx_state.conn)
                    .await
            }) {
                Ok(_r) => {
                    let _: crate::db::pool::DbQueryResult = _r;
                    tracing::info!("[plugin:{}] transaction committed", self.plugin_id);
                    r#"{"ok":true}"#.to_string()
                }
                Err(e) => {
                    let _: Result<crate::db::pool::DbQueryResult, sqlx::Error> =
                        handle.block_on(async {
                            sqlx::query::<crate::db::pool::Db>("ROLLBACK")
                                .execute(&mut *tx_state.conn)
                                .await
                        });
                    format!(r#"{{"error":"COMMIT failed, rolled back: {e}"}}"#)
                }
            }
        })
    }

    /// Roll back the current transaction and release the connection.
    #[must_use]
    pub fn db_rollback(&self) -> String {
        let mut tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut tx_state) = tx_guard.take() else {
            return r#"{"error":"no active transaction"}"#.to_string();
        };
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                sqlx::query::<crate::db::pool::Db>("ROLLBACK")
                    .execute(&mut *tx_state.conn)
                    .await
            }) {
                Ok(_r) => {
                    let _: crate::db::pool::DbQueryResult = _r;
                    tracing::info!("[plugin:{}] transaction rolled back", self.plugin_id);
                    r#"{"ok":true}"#.to_string()
                }
                Err(e) => format!(r#"{{"error":"ROLLBACK failed: {e}"}}"#),
            }
        })
    }

    /// Clean up an uncommitted transaction (called on plugin timeout/crash).
    pub fn cleanup_tx(&self) {
        let mut tx_guard = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut tx_state) = tx_guard.take() {
            let handle = tokio::runtime::Handle::current();
            let plugin_id = self.plugin_id.clone();
            tokio::task::block_in_place(|| {
                let _ = handle.block_on(async {
                    sqlx::query::<crate::db::pool::Db>("ROLLBACK")
                        .execute(&mut *tx_state.conn)
                        .await
                });
                tracing::warn!(
                    "[plugin:{plugin_id}] cleaned up dangling transaction (rolled back)"
                );
            });
        }
    }

    // ── High-level CRUD API ─────────────────────────────────────

    /// Check if a table is a content type table with tenantable protocol
    fn is_tenantable_table(&self, table: &str) -> bool {
        self.content_type_registry
            .as_ref()
            .and_then(|reg| reg.get_by_table(table))
            .is_some_and(|ct| ct.implements_protocol("tenantable"))
    }

    fn check_table_readable(&self, table: &str) -> Result<(), String> {
        if !crate::db::driver::is_safe_identifier(table) {
            return Err(format!("invalid table name: {table}"));
        }
        if !PermissionChecker::is_table_readable(&self.permissions, table) {
            if PermissionChecker::is_protected_table(
                table,
                &self.config.builtins.protected_tables(),
            ) {
                return Err(format!("table '{table}' is protected"));
            }
            return Err(format!("no read permission for table: {table}"));
        }
        Ok(())
    }

    fn check_table_writable(&self, table: &str) -> Result<(), String> {
        if !crate::db::driver::is_safe_identifier(table) {
            return Err(format!("invalid table name: {table}"));
        }
        if !PermissionChecker::is_table_writable(&self.permissions, table) {
            if PermissionChecker::is_protected_table(
                table,
                &self.config.builtins.protected_tables(),
            ) {
                return Err(format!("table '{table}' is protected"));
            }
            return Err(format!("no write permission for table: {table}"));
        }
        Ok(())
    }

    fn require_pool(&self) -> Result<&Pool, String> {
        self.pool
            .as_ref()
            .ok_or_else(|| "no database access".to_string())
    }

    /// Insert a row into a table.
    ///
    /// `data_json` is a JSON object of column-value pairs.
    /// `options_json` is optional: `{ "tenant": "tenant_id" }` or `{ "tenant": false }`.
    ///
    /// Returns `{"data":{...},"rows_affected":1}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_insert(&self, table: &str, data_json: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_writable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let mut data: serde_json::Map<String, serde_json::Value> =
            match serde_json::from_str(data_json) {
                Ok(d) => d,
                Err(e) => return format!(r#"{{"error":"invalid data JSON: {e}"}}"#),
            };

        let opts = CrudOptions::parse(options_json);
        if self.is_tenantable_table(table) {
            match &opts.tenant {
                CrudTenant::Auto => {}
                CrudTenant::Explicit(tid) => {
                    data.insert("tenant_id".into(), serde_json::Value::String(tid.clone()));
                }
                CrudTenant::Disabled => {}
            }
        }

        let mut cols = Vec::new();
        let mut vals = Vec::new();
        let mut args = DbArguments::default();
        for (k, v) in &data {
            if !crate::db::driver::is_safe_identifier(k) {
                return format!(r#"{{"error":"invalid column name: {k}"}}"#);
            }
            cols.push(k.clone());
            Self::add_param(&mut args, v);
            vals.push(crate::db::Driver::ph(vals.len() + 1));
        }

        let sql = format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            cols.join(", "),
            vals.join(", ")
        );
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<DbQueryResult, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .execute(pool)
                    .await
            });
            match result {
                Ok(r) => {
                    let affected: u64 = r.rows_affected();
                    format!(r#"{{"rows_affected":{affected}}}"#)
                }
                Err(e) => format!(r#"{{"error":"insert failed: {e}"}}"#),
            }
        })
    }

    /// Fetch a single row from a table.
    ///
    /// `where_json` can be:
    /// - A JSON object: `{"id": 1}` → WHERE id = ?
    /// - A string: `"id = 1 AND status = 'active'"` → raw WHERE clause
    /// - An array: `["status = ? AND total > ?", "active", 100]` → parameterized
    ///
    /// Returns `{"data":{...}}` or `{"data":null}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_fetch_one(&self, table: &str, where_json: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_readable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);
        let tenantable = self.is_tenantable_table(table);
        let (sql, args) = Self::build_query_args(tenantable, table, &where_result, &opts);

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .fetch_optional(pool)
                    .await
            }) {
                Ok(Some(row)) => {
                    let json = crate::plugins::rows_to_json(std::slice::from_ref(&row));
                    format!(r#"{{"data":{json}}}"#)
                }
                Ok(None) => r#"{"data":null}"#.to_string(),
                Err(e) => format!(r#"{{"error":"query failed: {e}"}}"#),
            }
        })
    }

    /// Fetch multiple rows from a table.
    ///
    /// `where_json` same as `db_fetch_one`.
    /// `options_json` can include `order_by`, `limit`, `offset`, `tenant`.
    ///
    /// Returns `{"data":[...],"total":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_fetch_all(&self, table: &str, where_json: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_readable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);
        let tenantable = self.is_tenantable_table(table);
        let (sql, args) = Self::build_query_args(tenantable, table, &where_result, &opts);

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<Vec<DbRow>, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .fetch_all(pool)
                    .await
            });
            match result {
                Ok(rows) => {
                    let count: usize = rows.len();
                    let json = crate::plugins::rows_to_json(&rows);
                    format!(r#"{{"data":{json},"total":{count}}}"#)
                }
                Err(e) => format!(r#"{{"error":"query failed: {e}"}}"#),
            }
        })
    }

    /// Update rows in a table.
    ///
    /// `data_json` is a JSON object of columns to set.
    /// `where_json` same as `db_fetch_one`.
    ///
    /// Returns `{"rows_affected":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_update(
        &self,
        table: &str,
        data_json: &str,
        where_json: &str,
        options_json: &str,
    ) -> String {
        if let Err(e) = self.check_table_writable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let data: serde_json::Map<String, serde_json::Value> = match serde_json::from_str(data_json)
        {
            Ok(d) => d,
            Err(e) => return format!(r#"{{"error":"invalid data JSON: {e}"}}"#),
        };
        if data.is_empty() {
            return r#"{"error":"no columns to update"}"#.to_string();
        }
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);

        let mut set_parts = Vec::new();
        let mut args = DbArguments::default();
        let mut idx = 1;
        for (k, v) in &data {
            if !crate::db::driver::is_safe_identifier(k) {
                return format!(r#"{{"error":"invalid column name: {k}"}}"#);
            }
            set_parts.push(format!("{k} = {}", crate::db::Driver::ph(idx)));
            idx += 1;
            Self::add_param(&mut args, v);
        }

        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let ph = crate::db::Driver::ph(idx);
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let sql = format!("UPDATE {table} SET {}{where_sql}", set_parts.join(", "));
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<DbQueryResult, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .execute(pool)
                    .await
            });
            match result {
                Ok(r) => {
                    let affected: u64 = r.rows_affected();
                    format!(r#"{{"rows_affected":{affected}}}"#)
                }
                Err(e) => format!(r#"{{"error":"update failed: {e}"}}"#),
            }
        })
    }

    /// Delete rows from a table.
    ///
    /// `where_json` same as `db_fetch_one`.
    ///
    /// Returns `{"rows_affected":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_delete(&self, table: &str, where_json: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_writable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);

        let mut args = DbArguments::default();
        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let idx = where_result.params.len() + 1;
            let ph = crate::db::Driver::ph(idx);
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let sql = format!("DELETE FROM {table}{where_sql}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<DbQueryResult, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .execute(pool)
                    .await
            });
            match result {
                Ok(r) => {
                    let affected: u64 = r.rows_affected();
                    format!(r#"{{"rows_affected":{affected}}}"#)
                }
                Err(e) => format!(r#"{{"error":"delete failed: {e}"}}"#),
            }
        })
    }

    /// Count rows in a table.
    ///
    /// `where_json` same as `db_fetch_one`.
    ///
    /// Returns `{"count":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_count(&self, table: &str, where_json: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_readable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);

        let mut args = DbArguments::default();
        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let idx = where_result.params.len() + 1;
            let ph = crate::db::Driver::ph(idx);
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let sql = format!("SELECT COUNT(*) as cnt FROM {table}{where_sql}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let row: (i64,) = sqlx::query_as_with(&sql, args).fetch_one(pool).await?;
                Ok::<_, sqlx::Error>(row.0)
            }) {
                Ok(count) => format!(r#"{{"count":{count}}}"#),
                Err(e) => format!(r#"{{"error":"count failed: {e}"}}"#),
            }
        })
    }

    /// Atomically increment (or decrement) numeric columns.
    ///
    /// `columns_json` is a JSON object mapping column names to delta values, e.g. `{"view_count": 1}`.
    /// `where_json` same as other CRUD functions.
    /// `options_json` can include:
    /// - `set`: JSON object of regular column-value pairs to SET alongside the increments.
    /// - `min`: integer (default no clamp). When set, generates `CASE WHEN col > min - delta THEN col + delta ELSE min END`.
    /// - `tenant`: tenant control.
    ///
    /// Returns `{"rows_affected":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_increment(
        &self,
        table: &str,
        columns_json: &str,
        where_json: &str,
        options_json: &str,
    ) -> String {
        if let Err(e) = self.check_table_writable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let columns: serde_json::Map<String, serde_json::Value> =
            match serde_json::from_str(columns_json) {
                Ok(c) => c,
                Err(e) => return format!(r#"{{"error":"invalid columns JSON: {e}"}}"#),
            };
        if columns.is_empty() {
            return r#"{"error":"no columns to increment"}"#.to_string();
        }

        let opts = CrudOptions::parse(options_json);
        let set_data: Option<serde_json::Map<String, serde_json::Value>> =
            serde_json::from_str(options_json)
                .ok()
                .and_then(|obj: serde_json::Map<String, serde_json::Value>| obj.get("set").cloned())
                .and_then(|v| {
                    if v.is_object() {
                        v.as_object().cloned()
                    } else {
                        None
                    }
                });

        let min_value: Option<i64> = serde_json::from_str(options_json)
            .ok()
            .and_then(|obj: serde_json::Map<String, serde_json::Value>| obj.get("min").cloned())
            .and_then(|v| v.as_i64());

        let mut set_parts = Vec::new();
        let mut args = DbArguments::default();
        let mut idx = 1;

        for (col, delta) in &columns {
            let delta_i64 = match delta.as_i64() {
                Some(d) => d,
                None => return format!(r#"{{"error":"delta for '{col}' must be an integer"}}"#),
            };
            if let Some(min) = min_value {
                let min_ph = crate::db::Driver::ph(idx);
                idx += 1;
                let delta_ph = crate::db::Driver::ph(idx);
                idx += 1;
                set_parts.push(format!("{col} = MAX({min_ph}, {col} + {delta_ph})"));
                args.add(min).ok();
                args.add(delta_i64).ok();
            } else {
                let ph = crate::db::Driver::ph(idx);
                idx += 1;
                set_parts.push(format!("{col} = {col} + {ph}"));
                args.add(delta_i64).ok();
            }
        }

        if let Some(ref set) = set_data {
            for (k, v) in set {
                let ph = crate::db::Driver::ph(idx);
                idx += 1;
                set_parts.push(format!("{k} = {ph}"));
                Self::add_param(&mut args, v);
            }
        }

        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let ph = crate::db::Driver::ph(idx);
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let sql = format!("UPDATE {table} SET {}{where_sql}", set_parts.join(", "));
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<DbQueryResult, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .execute(pool)
                    .await
            });
            match result {
                Ok(r) => {
                    let affected: u64 = r.rows_affected();
                    format!(r#"{{"rows_affected":{affected}}}"#)
                }
                Err(e) => format!(r#"{{"error":"increment failed: {e}"}}"#),
            }
        })
    }

    /// Compute SUM of a column.
    ///
    /// Returns `{"sum":<number>}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_sum(
        &self,
        table: &str,
        column: &str,
        where_json: &str,
        options_json: &str,
    ) -> String {
        if let Err(e) = self.check_table_readable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let where_result = match Self::build_where_clause(where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };
        let opts = CrudOptions::parse(options_json);

        let mut args = DbArguments::default();
        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let idx = where_result.params.len() + 1;
            let ph = crate::db::Driver::ph(idx);
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let sql = format!("SELECT COALESCE(SUM({column}), 0) as total FROM {table}{where_sql}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let row: DbRow = sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .fetch_one(pool)
                    .await?;
                Ok::<_, sqlx::Error>(row)
            }) {
                Ok(row) => {
                    use sqlx::Row;
                    let total: f64 = if let Ok(v) = row.try_get::<i64, _>(0) {
                        v as f64
                    } else {
                        row.try_get::<f64, _>(0).unwrap_or(0.0)
                    };
                    if total.fract() == 0.0 {
                        format!(r#"{{"sum":{}}}"#, total as i64)
                    } else {
                        format!(r#"{{"sum":{total}}}"#)
                    }
                }
                Err(e) => format!(r#"{{"error":"sum failed: {e}"}}"#),
            }
        })
    }

    /// GROUP BY with count and optional sum aggregates.
    ///
    /// `options_json` must include:
    /// - `group_by`: string or array of strings — columns to GROUP BY.
    /// - `count`: boolean (default false) — include COUNT(*).
    /// - `sum`: string or array of strings — columns to SUM.
    /// - `where`, `order_by`, `limit`, `tenant` — same as other CRUD functions.
    ///
    /// Returns `{"data":[...],"total":N}` or `{"error":"..."}`.
    #[must_use]
    pub fn db_group_by(&self, table: &str, options_json: &str) -> String {
        if let Err(e) = self.check_table_readable(table) {
            return format!(r#"{{"error":"{e}"}}"#);
        }
        let Ok(pool) = self.require_pool() else {
            return r#"{"error":"no database access"}"#.to_string();
        };
        let obj: serde_json::Map<String, serde_json::Value> =
            match serde_json::from_str(options_json) {
                Ok(o) => o,
                Err(e) => return format!(r#"{{"error":"invalid options JSON: {e}"}}"#),
            };

        let group_by: Vec<String> = match obj.get("group_by") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => return r#"{"error":"group_by is required"}"#.to_string(),
        };
        if group_by.is_empty() {
            return r#"{"error":"group_by cannot be empty"}"#.to_string();
        }
        if !group_by
            .iter()
            .all(|c| crate::db::driver::is_safe_identifier(c))
        {
            return r#"{"error":"invalid column name in group_by"}"#.to_string();
        }

        let do_count = obj.get("count").and_then(|v| v.as_bool()).unwrap_or(false);

        let sum_cols: Vec<String> = match obj.get("sum") {
            Some(serde_json::Value::String(s)) => vec![s.clone()],
            Some(serde_json::Value::Array(arr)) => arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        };
        if !sum_cols
            .iter()
            .all(|c| crate::db::driver::is_safe_identifier(c))
        {
            return r#"{"error":"invalid column name in sum"}"#.to_string();
        }

        let where_json = obj
            .get("where")
            .and_then(|v| serde_json::to_string(v).ok())
            .unwrap_or_default();
        let where_result = match Self::build_where_clause(&where_json) {
            Ok(w) => w,
            Err(e) => return format!(r#"{{"error":"{e}"}}"#),
        };

        let opts = CrudOptions::parse(options_json);

        let mut select_parts: Vec<String> = group_by.clone();
        if do_count {
            select_parts.push("COUNT(*) as cnt".to_string());
        }
        for col in &sum_cols {
            select_parts.push(format!("COALESCE(SUM({col}), 0) as sum_{col}"));
        }

        let mut args = DbArguments::default();
        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }

        let mut idx = where_result.params.len() + 1;
        if self.is_tenantable_table(table) && !opts.tenant_is_disabled() {
            let ph = crate::db::Driver::ph(idx);
            idx += 1;
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }

        let group_clause = group_by.join(", ");
        let mut sql = format!(
            "SELECT {} FROM {table}{where_sql} GROUP BY {group_clause}",
            select_parts.join(", ")
        );

        if let Some(ref order_by) = opts.order_by
            && Self::is_safe_order_by(order_by)
        {
            sql.push_str(&format!(" ORDER BY {order_by}"));
        }
        if let Some(lim) = opts.limit {
            let ph = crate::db::Driver::ph(idx);
            sql.push_str(&format!(" LIMIT {ph}"));
            args.add(lim as i64).ok();
        }

        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            let result: Result<Vec<DbRow>, sqlx::Error> = handle.block_on(async {
                sqlx::query_with::<crate::db::pool::Db, _>(&sql, args)
                    .fetch_all(pool)
                    .await
            });
            match result {
                Ok(rows) => {
                    let count: usize = rows.len();
                    let json = crate::plugins::rows_to_json(&rows);
                    format!(r#"{{"data":{json},"total":{count}}}"#)
                }
                Err(e) => format!(r#"{{"error":"group_by failed: {e}"}}"#),
            }
        })
    }

    // ── Where clause parsing ─────────────────────────────────────

    fn is_safe_order_by(order_by: &str) -> bool {
        order_by.split(',').all(|part| {
            let part = part.trim();
            let core = part
                .strip_suffix(" DESC")
                .or_else(|| part.strip_suffix(" ASC"))
                .unwrap_or(part)
                .trim();
            core.split('.')
                .all(|seg| crate::db::driver::is_safe_identifier(seg.trim()))
        })
    }

    fn build_where_clause(where_json: &str) -> Result<WhereResult, String> {
        let trimmed = where_json.trim();
        if trimmed.is_empty() || trimmed == "null" || trimmed == "{}" {
            return Ok(WhereResult {
                clause: String::new(),
                params: Vec::new(),
            });
        }

        // Try array form: ["col = ? AND col2 = ?", val1, val2]
        if trimmed.starts_with('[') {
            let arr: Vec<serde_json::Value> =
                serde_json::from_str(trimmed).map_err(|e| format!("invalid where array: {e}"))?;
            if arr.is_empty() {
                return Ok(WhereResult {
                    clause: String::new(),
                    params: Vec::new(),
                });
            }
            let clause = arr[0]
                .as_str()
                .ok_or_else(|| "where array first element must be a SQL string".to_string())?;
            let params = arr[1..].to_vec();
            return Ok(WhereResult {
                clause: clause.to_string(),
                params,
            });
        }

        // Try object form: {"col1": val1, "col2": val2}
        if trimmed.starts_with('{') {
            let obj: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(trimmed).map_err(|e| format!("invalid where object: {e}"))?;
            if obj.is_empty() {
                return Ok(WhereResult {
                    clause: String::new(),
                    params: Vec::new(),
                });
            }
            let mut parts = Vec::new();
            let mut params = Vec::new();
            for (i, (k, _v)) in obj.iter().enumerate() {
                if !crate::db::driver::is_safe_identifier(k) {
                    return Err(format!("invalid column name in where: {k}"));
                }
                parts.push(format!("{k} = {}", crate::db::Driver::ph(i + 1)));
            }
            for (_, v) in obj.iter() {
                params.push(v.clone());
            }
            return Ok(WhereResult {
                clause: parts.join(" AND "),
                params,
            });
        }

        // Unknown format — reject raw SQL strings
        Err(
            "where_json must be a JSON object or array, raw SQL strings are not allowed"
                .to_string(),
        )
    }

    fn build_query_args(
        tenantable: bool,
        table: &str,
        where_result: &WhereResult,
        opts: &CrudOptions,
    ) -> (String, DbArguments<'static>) {
        let mut args = DbArguments::default();
        let mut where_sql = String::new();
        if !where_result.clause.is_empty() {
            where_sql = format!(" WHERE {}", where_result.clause);
        }
        for p in &where_result.params {
            Self::add_param(&mut args, p);
        }
        let mut idx = where_result.params.len() + 1;
        if tenantable && !opts.tenant_is_disabled() {
            let ph = crate::db::Driver::ph(idx);
            idx += 1;
            let connector = if where_sql.is_empty() {
                " WHERE"
            } else {
                " AND"
            };
            where_sql.push_str(&format!("{connector} tenant_id = {ph}"));
            let tid = opts
                .tenant_value_owned()
                .unwrap_or_else(|| crate::constants::DEFAULT_TENANT.to_string());
            args.add(tid).ok();
        }
        if let Some(ref order_by) = opts.order_by
            && Self::is_safe_order_by(order_by)
        {
            where_sql.push_str(&format!(" ORDER BY {order_by}"));
        }
        if let Some(lim) = opts.limit {
            let ph = crate::db::Driver::ph(idx);
            idx += 1;
            where_sql.push_str(&format!(" LIMIT {ph}"));
            args.add(lim as i64).ok();
        }
        if let Some(off) = opts.offset {
            let ph = crate::db::Driver::ph(idx);
            where_sql.push_str(&format!(" OFFSET {ph}"));
            args.add(off as i64).ok();
        }
        (format!("SELECT * FROM {table}{where_sql}"), args)
    }

    /// Read a file from the virtual file system
    pub fn vfs_read(&self, path: &str) -> Result<String, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.read_file(path).map_err(|e| e.to_string())
    }

    /// Write a file to the virtual file system
    pub fn vfs_write(&self, path: &str, content: &str) -> Result<(), String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.write_file(path, content).map_err(|e| e.to_string())
    }

    /// Delete a file from the virtual file system
    pub fn vfs_delete(&self, path: &str) -> Result<(), String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.delete_file(path).map_err(|e| e.to_string())
    }

    /// Check if a file exists in the virtual file system
    pub fn vfs_exists(&self, path: &str) -> Result<bool, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.exists(path).map_err(|e| e.to_string())
    }

    /// List directory contents in the virtual file system
    pub fn vfs_list(&self, path: &str) -> Result<Vec<String>, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        vfs.list_dir(path).map_err(|e| e.to_string())
    }

    /// Get file metadata from the virtual file system (returns JSON)
    pub fn vfs_stat(&self, path: &str) -> Result<String, String> {
        let vfs = VirtualFs::new(&self.config, &self.plugin_id, &self.permissions);
        let info = vfs.stat(path).map_err(|e| e.to_string())?;
        serde_json::to_string(&info).map_err(|e| format!("error: {e}"))
    }

    /// Emit a custom event from the plugin, broadcast via EventBus.
    ///
    /// Other plugins can listen via Hooks, and WebSocket clients will also receive pushes.
    /// Returns `{"ok":true}` or `{"error":"..."}`.
    #[must_use]
    pub fn emit_event(&self, event_type: &str, data: &str) -> String {
        match &self.event_bus {
            Some(bus) => {
                let data_value: serde_json::Value = serde_json::from_str(data)
                    .unwrap_or(serde_json::Value::String(data.to_string()));
                bus.emit(Event::Custom {
                    source: self.plugin_id.clone(),
                    event_type: event_type.to_string(),
                    data: data_value,
                });
                tracing::info!(
                    "[plugin:{}] emitted custom event: {}",
                    self.plugin_id,
                    event_type
                );
                r#"{"ok":true}"#.to_string()
            }
            None => r#"{"error":"event bus not available"}"#.to_string(),
        }
    }

    /// Parse params JSON into Vec<serde_json::Value>
    fn parse_params(params_json: &str) -> Result<Option<Vec<serde_json::Value>>, String> {
        if params_json.is_empty() {
            return Ok(None);
        }
        let params: Vec<serde_json::Value> =
            serde_json::from_str(params_json).map_err(|e| format!("invalid params JSON: {e}"))?;
        for p in &params {
            if matches!(
                p,
                serde_json::Value::Array(_) | serde_json::Value::Object(_)
            ) {
                return Err(format!("unsupported param type: {p}"));
            }
        }
        Ok(Some(params))
    }

    /// Add a single JSON Value to the sqlx parameter list
    fn add_param(args: &mut DbArguments<'_>, p: &serde_json::Value) {
        match p {
            serde_json::Value::String(s) => {
                args.add(s.clone()).ok();
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    args.add(i).ok();
                } else {
                    args.add(n.as_f64().unwrap_or(0.0)).ok();
                }
            }
            serde_json::Value::Bool(b) => {
                args.add(*b).ok();
            }
            serde_json::Value::Null => {
                args.add(Option::<String>::None).ok();
            }
            _ => {}
        }
    }
}

/// Execute a parameterized or raw write operation on a connection
fn build_and_exec(
    conn: &mut DbConnection,
    sql: &str,
    params: &[serde_json::Value],
    handle: &tokio::runtime::Handle,
) -> Result<DbQueryResult, sqlx::Error> {
    let mut args = DbArguments::default();
    for p in params {
        add_param_value(&mut args, p);
    }
    handle.block_on(async {
        sqlx::query_with::<crate::db::pool::Db, _>(sql, args)
            .execute(conn)
            .await
    })
}

fn add_param_value(args: &mut DbArguments, p: &serde_json::Value) {
    match p {
        serde_json::Value::String(s) => {
            args.add(s.clone()).ok();
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                args.add(i).ok();
            } else {
                args.add(n.as_f64().unwrap_or(0.0)).ok();
            }
        }
        serde_json::Value::Bool(b) => {
            args.add(*b).ok();
        }
        serde_json::Value::Null => {
            args.add(Option::<String>::None).ok();
        }
        _ => {}
    }
}

/// Read a config value from `AppConfig` by key path
#[must_use]
pub fn get_config_value(config: &AppConfig, key: &str) -> Option<String> {
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
        let mut config = AppConfig::test_defaults();
        config.host = "127.0.0.1".into();
        config.builtins.blog = false;
        Arc::new(config)
    }

    #[test]
    fn get_config_value_returns_known_keys() {
        let config = make_test_config();
        assert_eq!(
            get_config_value(&config, "app.host"),
            Some("127.0.0.1".into())
        );
        assert_eq!(get_config_value(&config, "app.port"), Some("9898".into()));
        assert_eq!(get_config_value(&config, "app.env"), Some("test".into()));
        assert_eq!(
            get_config_value(&config, "app.base_url"),
            Some("http://localhost:3000".into())
        );
        assert!(get_config_value(&config, "jwt.secret").is_none());
        assert!(get_config_value(&config, "database_url").is_none());
    }

    #[test]
    fn host_context_get_config_checks_permissions() {
        let config = make_test_config();
        let perms = Permissions {
            config: vec!["app.*".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert!(ctx.get_config("app.env").is_some());
        assert!(ctx.get_config("unknown.key").is_none());
    }

    #[test]
    fn host_context_http_get_blocked_without_permission() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.http_get("https://evil.com");
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_context_db_query_rejects_non_select() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_query("DELETE FROM posts", "[]");
        assert!(result.contains("error"));
        assert!(!result.contains("status"));
    }

    #[test]
    fn host_context_get_data_returns_none_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(ctx.get_data("key").is_none());
    }

    #[test]
    fn host_context_set_data_returns_false_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(!ctx.set_data("key", "val"));
    }

    #[test]
    fn host_context_get_post_returns_none_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(ctx.get_post("slug").is_none());
    }

    #[test]
    fn host_context_db_query_returns_error_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_query("SELECT 1", "[]");
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_context_log_does_not_panic() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        ctx.log("info", "hello");
        ctx.log("warn", "warning");
        ctx.log("error", "error");
    }

    #[test]
    fn host_context_http_post_blocked_without_permission() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.http_post("https://evil.com", "{}");
        assert!(result.contains("not allowed"));
    }

    #[test]
    fn host_context_get_config_with_restricted_permissions() {
        let config = make_test_config();
        let perms = Permissions {
            config: vec!["seo.*".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert!(ctx.get_config("seo.title").is_none()); // seo.title doesn't exist
        assert!(ctx.get_config("app.env").is_none()); // blocked by permission
    }

    #[test]
    fn host_context_db_query_rejects_write() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert!(
            ctx.db_query("INSERT INTO posts VALUES(1)", "[]")
                .contains("error")
        );
        assert!(
            ctx.db_query("UPDATE posts SET title='x'", "[]")
                .contains("error")
        );
        assert!(ctx.db_query("DELETE FROM posts", "[]").contains("error"));
    }

    #[test]
    fn host_context_db_query_table_permission_blocked() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        // No pool → first check is "no database access", but even with pool it should fail
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_query("SELECT * FROM posts", "[]");
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_context_get_post_blocked_without_db_permission() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert!(ctx.get_post("any-slug").is_none());
    }

    #[test]
    fn host_context_plugin_id_accessor() {
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "my-plugin".into(),
            Permissions::default(),
            None,
        );
        assert_eq!(ctx.plugin_id(), "my-plugin");
    }

    #[test]
    fn host_context_max_memory_bytes_default() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        assert_eq!(ctx.max_memory_bytes(), 32 * 1024 * 1024);
    }

    #[test]
    fn host_context_max_memory_bytes_custom() {
        let config = make_test_config();
        let perms = Permissions {
            max_memory_mb: Some(64),
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        assert_eq!(ctx.max_memory_bytes(), 64 * 1024 * 1024);
    }

    #[test]
    fn host_context_ph_returns_placeholder() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        #[cfg(not(feature = "db-postgres"))]
        {
            assert_eq!(ctx.db_ph(1), "?");
            assert_eq!(ctx.db_ph(5), "?");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_set_data_with_real_db() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "plugin-a".into(),
            Permissions::default(),
            Some(pool.clone()),
        );

        assert!(ctx.get_data("greeting").is_none());
        assert!(ctx.set_data("greeting", "hello world"));
        assert_eq!(ctx.get_data("greeting"), Some("hello world".into()));

        assert!(ctx.set_data("greeting", "updated"));
        assert_eq!(ctx.get_data("greeting"), Some("updated".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_isolation_between_plugins() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let config1 = make_test_config();
        let config2 = make_test_config();
        let ctx_a = HostContext::new(
            "test",
            config1,
            "plugin-a".into(),
            Permissions::default(),
            Some(pool.clone()),
        );
        let ctx_b = HostContext::new(
            "test",
            config2,
            "plugin-b".into(),
            Permissions::default(),
            Some(pool.clone()),
        );

        ctx_a.set_data("key", "value-a");
        ctx_b.set_data("key", "value-b");

        assert_eq!(ctx_a.get_data("key"), Some("value-a".into()));
        assert_eq!(ctx_b.get_data("key"), Some("value-b".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_with_real_db() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["posts".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT COUNT(*) as cnt FROM posts", "[]");
        assert!(!result.contains("error"));
        assert!(result.contains("cnt"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_table_not_permitted() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["read:comments".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT * FROM posts", "[]");
        assert!(result.contains("error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_query_wildcard_permission() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["*".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_query("SELECT COUNT(*) as cnt FROM posts", "[]");
        assert!(!result.contains("error"));
    }

    #[test]
    fn host_context_db_execute_rejects_select() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_execute("SELECT * FROM posts", "[]");
        assert!(result.contains("only INSERT/UPDATE/DELETE"));
    }

    #[test]
    fn host_context_db_execute_rejects_ddl() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_execute("CREATE TABLE evil (id TEXT)", "[]");
        assert!(result.contains("DDL operations") || result.contains("only INSERT/UPDATE/DELETE"));
    }

    #[test]
    fn host_context_db_execute_rejects_protected_table() {
        let config = make_test_config();
        let perms = Permissions::default();
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_execute("DELETE FROM users WHERE 1=1", "[]");
        assert!(result.contains("error"));
    }

    #[test]
    fn host_context_db_execute_rejects_no_write_permission() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["read:orders".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_execute("INSERT INTO orders (id) VALUES ('1')", "[]");
        assert!(result.contains("error"));
    }

    #[test]
    fn host_context_db_execute_no_database_access() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["orders".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_execute("INSERT INTO orders (id) VALUES ('1')", "[]");
        assert!(result.contains("no database access"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_execute_with_real_db() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES ('Test', 'test')",
            "[]",
        );
        assert!(result.contains("rows_affected"));
        assert!(!result.contains("error"));

        let update = ctx.db_execute("UPDATE tags SET name = 'Updated' WHERE slug = 'test'", "[]");
        assert!(update.contains("rows_affected"));

        let delete = ctx.db_execute("DELETE FROM tags WHERE slug = 'test'", "[]");
        assert!(delete.contains("rows_affected"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_execute_parameterized() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let result = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES (?, ?)",
            r#"["Param Tag","param-tag"]"#,
        );
        assert!(result.contains("rows_affected"));
        assert!(!result.contains("error"));

        let update = ctx.db_execute(
            "UPDATE tags SET name = ? WHERE slug = ?",
            r#"["Renamed","param-tag"]"#,
        );
        assert!(update.contains("rows_affected"));

        let delete = ctx.db_execute("DELETE FROM tags WHERE slug = ?", r#"["param-tag"]"#);
        assert!(delete.contains("rows_affected"));
    }

    #[test]
    fn host_context_db_execute_invalid_params_json() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_execute("INSERT INTO tags (name, slug) VALUES (?)", "not valid json");
        assert!(result.contains("invalid params JSON"));
    }

    #[test]
    fn host_context_db_execute_unsupported_param_type() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let result = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES (?, ?)",
            r#"[{"nested":"object"}]"#,
        );
        assert!(result.contains("unsupported param type"));
    }

    #[test]
    fn host_context_db_begin_returns_error_without_pool() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_begin();
        assert!(result.contains("no database access"));
    }

    #[test]
    fn host_context_db_commit_returns_error_without_tx() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_commit();
        assert!(result.contains("no active transaction"));
    }

    #[test]
    fn host_context_db_rollback_returns_error_without_tx() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.db_rollback();
        assert!(result.contains("no active transaction"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_transaction_commit_roundtrip() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#), "begin failed: {begin}");

        let insert = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES ('TxTest', 'tx-test')",
            "[]",
        );
        assert!(insert.contains("rows_affected"), "insert failed: {insert}");

        let commit = ctx.db_commit();
        assert!(commit.contains(r#""ok":true"#), "commit failed: {commit}");

        let rows: Vec<(String,)> = sqlx::query_as("SELECT name FROM tags WHERE slug = 'tx-test'")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "row should be committed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_transaction_rollback_discards() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#));

        let insert = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES ('RbTest', 'rb-test')",
            "[]",
        );
        assert!(insert.contains("rows_affected"));

        let rollback = ctx.db_rollback();
        assert!(rollback.contains(r#""ok":true"#));

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tags WHERE slug = 'rb-test'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "row should be rolled back");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_transaction_double_begin_error() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();

        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "p1".into(),
            Permissions::default(),
            Some(pool),
        );

        let begin1 = ctx.db_begin();
        assert!(begin1.contains(r#""ok":true"#));

        let begin2 = ctx.db_begin();
        assert!(begin2.contains("already active"));

        ctx.cleanup_tx();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_cleanup_tx_rolls_back() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#));

        let insert = ctx.db_execute(
            "INSERT INTO tags (name, slug) VALUES ('CleanTest', 'cl-test')",
            "[]",
        );
        assert!(insert.contains("rows_affected"));

        ctx.cleanup_tx();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tags WHERE slug = 'cl-test'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "cleanup_tx should rollback");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_cleanup_tx_noop_without_active() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "p1".into(),
            Permissions::default(),
            Some(pool),
        );
        ctx.cleanup_tx();
    }

    // ── High-level CRUD tests ──────────────────────────────────────

    fn make_crud_ctx(pool: &Pool) -> HostContext {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["tags".into(), "categories".into(), "posts".into()],
            ..Permissions::default()
        };
        HostContext::new(
            "test",
            config,
            "crud-test".into(),
            perms,
            Some(pool.clone()),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_insert_and_fetch_one() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let result = ctx.db_insert("tags", r#"{"name":"Rust","slug":"rust"}"#, "{}");
        assert!(result.contains(r#""rows_affected":1"#), "insert: {result}");

        let found = ctx.db_fetch_one("tags", r#"{"slug":"rust"}"#, "{}");
        assert!(found.contains(r#""name":"Rust""#), "fetch_one: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_not_found() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let found = ctx.db_fetch_one("tags", r#"{"slug":"nonexistent"}"#, "{}");
        assert!(found.contains(r#""data":null"#), "not found: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_object_where() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Go","slug":"go"}"#, "{}");

        let found = ctx.db_fetch_one("tags", r#"{"name":"Go"}"#, "{}");
        assert!(found.contains(r#""slug":"go""#), "string where: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_array_where() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Python","slug":"python"}"#, "{}");

        let found = ctx.db_fetch_one("tags", r#"["name = ?", "Python"]"#, "{}");
        assert!(found.contains(r#""slug":"python""#), "array where: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_all_with_order_and_limit() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"A","slug":"a"}"#, "{}");
        let _ = ctx.db_insert("tags", r#"{"name":"B","slug":"b"}"#, "{}");
        let _ = ctx.db_insert("tags", r#"{"name":"C","slug":"c"}"#, "{}");

        let result = ctx.db_fetch_all("tags", "{}", r#"{"order_by":"name DESC","limit":2}"#);
        assert!(result.contains(r#""total":2"#), "fetch_all limit: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_update() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Old","slug":"old"}"#, "{}");

        let result = ctx.db_update("tags", r#"{"name":"New"}"#, r#"{"slug":"old"}"#, "{}");
        assert!(result.contains(r#""rows_affected":1"#), "update: {result}");

        let found = ctx.db_fetch_one("tags", r#"{"slug":"old"}"#, "{}");
        assert!(found.contains(r#""name":"New""#), "after update: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_update_empty_data() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let result = ctx.db_update("tags", "{}", "{}", "{}");
        assert!(result.contains("no columns"), "empty data: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_delete() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Delete","slug":"del"}"#, "{}");

        let result = ctx.db_delete("tags", r#"{"slug":"del"}"#, "{}");
        assert!(result.contains(r#""rows_affected":1"#), "delete: {result}");

        let found = ctx.db_fetch_one("tags", r#"{"slug":"del"}"#, "{}");
        assert!(found.contains(r#""data":null"#), "after delete: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_count() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Count1","slug":"c1"}"#, "{}");
        let _ = ctx.db_insert("tags", r#"{"name":"Count2","slug":"c2"}"#, "{}");

        let result = ctx.db_count("tags", "{}", "{}");
        assert!(result.contains(r#""count":2"#), "count: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_count_with_where() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("tags", r#"{"name":"Go","slug":"go"}"#, "{}");
        let _ = ctx.db_insert("tags", r#"{"name":"Rust","slug":"rust"}"#, "{}");

        let result = ctx.db_count("tags", r#"["name = ?", "Rust"]"#, "{}");
        assert!(
            result.contains(r#""count":1"#),
            "count with where: {result}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_crud_no_pool() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);

        assert!(ctx.db_insert("tags", "{}", "{}").contains("no database"));
        assert!(ctx.db_fetch_one("tags", "{}", "{}").contains("no database"));
        assert!(ctx.db_fetch_all("tags", "{}", "{}").contains("no database"));
        assert!(
            ctx.db_update("tags", "{}", "{}", "{}")
                .contains("no database")
        );
        assert!(ctx.db_delete("tags", "{}", "{}").contains("no database"));
        assert!(ctx.db_count("tags", "{}", "{}").contains("no database"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_crud_no_permission() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "p1".into(),
            Permissions::default(),
            Some(pool),
        );

        assert!(ctx.db_insert("tags", "{}", "{}").contains("error"));
        assert!(ctx.db_fetch_one("tags", "{}", "{}").contains("error"));
        assert!(ctx.db_fetch_all("tags", "{}", "{}").contains("error"));
        assert!(ctx.db_update("tags", "{}", "{}", "{}").contains("error"));
        assert!(ctx.db_delete("tags", "{}", "{}").contains("error"));
        assert!(ctx.db_count("tags", "{}", "{}").contains("error"));
    }

    // ── db_increment / db_sum / db_group_by tests ────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_simple() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Test","slug":"test","sort_order":"0"}"#,
            "{}",
        );

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            r#"{"slug":"test"}"#,
            "{}",
        );
        assert!(r.contains(r#""rows_affected":1"#), "increment: {r}");

        let s = ctx.db_sum("categories", "sort_order", r#"{"slug":"test"}"#, "{}");
        assert!(s.contains(r#""sum":1"#), "after increment sum: {s}");

        let r2 = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            r#"{"slug":"test"}"#,
            "{}",
        );
        assert!(r2.contains(r#""rows_affected":1"#));

        let s2 = ctx.db_sum("categories", "sort_order", r#"{"slug":"test"}"#, "{}");
        assert!(s2.contains(r#""sum":2"#), "after 2nd sum: {s2}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_negative_delta() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Dec","slug":"dec","sort_order":"5"}"#,
            "{}",
        );

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-1}"#,
            r#"{"slug":"dec"}"#,
            "{}",
        );
        assert!(r.contains(r#""rows_affected":1"#), "decrement: {r}");

        let s = ctx.db_sum("categories", "sort_order", r#"{"slug":"dec"}"#, "{}");
        assert!(s.contains(r#""sum":4"#), "after decrement sum: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_with_min_clamp() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Clamp","slug":"clamp","sort_order":"1"}"#,
            "{}",
        );

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-5}"#,
            r#"{"slug":"clamp"}"#,
            r#"{"min":0}"#,
        );
        assert!(r.contains(r#""rows_affected":1"#), "clamp: {r}");

        let s = ctx.db_sum("categories", "sort_order", r#"{"slug":"clamp"}"#, "{}");
        assert!(s.contains(r#""sum":0"#), "clamped to 0 sum: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_with_set() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Set","slug":"set","sort_order":"0"}"#,
            "{}",
        );

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            r#"{"slug":"set"}"#,
            r#"{"set":{"name":"Updated"}}"#,
        );
        assert!(r.contains(r#""rows_affected":1"#), "increment+set: {r}");

        let s = ctx.db_sum("categories", "sort_order", r#"{"slug":"set"}"#, "{}");
        assert!(s.contains(r#""sum":1"#), "incremented sum: {s}");

        let found = ctx.db_fetch_one("categories", r#"{"slug":"set"}"#, "{}");
        assert!(found.contains(r#""name":"Updated""#), "set col: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_no_pool() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["categories".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let r = ctx.db_increment("categories", r#"{"sort_order":1}"#, "{}", "{}");
        assert!(r.contains("no database access"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_no_permission() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "p1".into(),
            Permissions::default(),
            Some(pool),
        );
        let r = ctx.db_increment("categories", r#"{"sort_order":1}"#, "{}", "{}");
        assert!(r.contains("error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_sum_basic() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"A","slug":"sa","sort_order":"3"}"#,
            "{}",
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"B","slug":"sb","sort_order":"7"}"#,
            "{}",
        );

        let r = ctx.db_sum(
            "categories",
            "sort_order",
            r#"{"tenant_id":"default"}"#,
            "{}",
        );
        assert!(r.contains(r#""sum":10"#), "sum: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_sum_empty() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let r = ctx.db_sum("categories", "sort_order", "{}", "{}");
        assert!(r.contains(r#""sum":0"#), "sum empty: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_sum_no_pool() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["categories".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let r = ctx.db_sum("categories", "sort_order", "{}", "{}");
        assert!(r.contains("no database access"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_count() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Rust1","slug":"rust1","tenant_id":"t1"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Go","slug":"go","tenant_id":"t2"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Rust2","slug":"rust2","tenant_id":"t1"}"#,
            r#"{"tenant":"disabled"}"#,
        );

        let r = ctx.db_group_by(
            "categories",
            r#"{"group_by":"tenant_id","count":true,"order_by":"cnt DESC","tenant":false}"#,
        );
        assert!(r.contains(r#""total":2"#), "group_by count: {r}");
        assert!(r.contains(r#""tenant_id":"t1""#), "group_by t1: {r}");
        assert!(r.contains(r#""cnt":2"#), "group_by cnt: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_sum() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"A1","slug":"gsa","sort_order":"3","tenant_id":"grpA"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"A2","slug":"gsb","sort_order":"7","tenant_id":"grpA"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"B1","slug":"gsc","sort_order":"2","tenant_id":"grpB"}"#,
            r#"{"tenant":"disabled"}"#,
        );

        let r = ctx.db_group_by(
            "categories",
            r#"{"group_by":"tenant_id","count":true,"sum":"sort_order","order_by":"sum_sort_order DESC","tenant":false}"#,
        );
        assert!(r.contains(r#""total":2"#), "group_by sum total: {r}");
        assert!(
            r.contains(r#""sum_sort_order":10"#),
            "group_by sum grpA: {r}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_where() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"W1","slug":"wa","tenant_id":"tA"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"W2","slug":"wb","tenant_id":"tA"}"#,
            r#"{"tenant":"disabled"}"#,
        );
        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"W3","slug":"wc","tenant_id":"tB"}"#,
            r#"{"tenant":"disabled"}"#,
        );

        let r = ctx.db_group_by(
            "categories",
            r#"{"group_by":"tenant_id","count":true,"where":{"tenant_id":"tA"},"tenant":false}"#,
        );
        assert!(r.contains(r#""total":1"#), "group_by where: {r}");
        assert!(r.contains(r#""cnt":2"#), "group_by where cnt: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_limit() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert("categories", r#"{"name":"X","slug":"lx"}"#, "{}");
        let _ = ctx.db_insert("categories", r#"{"name":"Y","slug":"ly"}"#, "{}");
        let _ = ctx.db_insert("categories", r#"{"name":"Z","slug":"lz"}"#, "{}");

        let r = ctx.db_group_by(
            "categories",
            r#"{"group_by":"name","count":true,"limit":2,"order_by":"name ASC"}"#,
        );
        assert!(r.contains(r#""total":2"#), "group_by limit: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_missing_field() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let r = ctx.db_group_by("categories", r#"{"count":true}"#);
        assert!(r.contains("group_by is required"), "missing: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_no_pool() {
        let config = make_test_config();
        let perms = Permissions {
            database: vec!["categories".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        let r = ctx.db_group_by("categories", r#"{"group_by":"name","count":true}"#);
        assert!(r.contains("no database access"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_no_permission() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "p1".into(),
            Permissions::default(),
            Some(pool),
        );
        let r = ctx.db_group_by("categories", r#"{"group_by":"name","count":true}"#);
        assert!(r.contains("error"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_multi_column_with_min() {
        let pool = crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query::<crate::db::pool::Db>(crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        let ctx = make_crud_ctx(&pool);

        let _ = ctx.db_insert(
            "categories",
            r#"{"name":"Multi","slug":"multi","sort_order":"2"}"#,
            "{}",
        );

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-10}"#,
            r#"{"slug":"multi"}"#,
            r#"{"min":0}"#,
        );
        assert!(r.contains(r#""rows_affected":1"#), "multi clamp: {r}");

        let s = ctx.db_sum("categories", "sort_order", r#"{"slug":"multi"}"#, "{}");
        assert!(s.contains(r#""sum":0"#), "clamped sum: {s}");
    }
}
