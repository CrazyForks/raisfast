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

    /// Outbound api-client call (`host.callApi`, mvp-plan D3).
    ///
    /// Routes through the integration plane so auth injection, rate limiting
    /// and `itg_egress_log` tracing apply uniformly; the ambient `TRACE_CTX`
    /// (when the plugin runs inside a traced job) attaches automatically.
    /// Returns `{"status":..,"output":..,"tokens_in":..,"tokens_out":..,"model":..}`
    /// on success or `{"error": ".."}` on failure — JSON string form for all engines.
    #[must_use]
    pub fn api_call(&self, client_key: &str, op: &str, input: &str) -> String {
        let fail = |msg: String| serde_json::json!({ "error": msg }).to_string();
        if !PermissionChecker::is_egress_allowed(&self.permissions, client_key) {
            return fail(format!("egress client not allowed: {client_key}"));
        }
        let Some(integration) = crate::integration::shared() else {
            return fail("integration plane disabled".into());
        };
        let parsed: serde_json::Value = serde_json::from_str(input)
            .unwrap_or_else(|_| serde_json::Value::String(input.to_string()));
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(integration.call_api(client_key, op, parsed)) {
                Ok(receipt) => serde_json::json!({
                    "status": receipt.status,
                    "output": receipt.output,
                    "tokens_in": receipt.tokens_in,
                    "tokens_out": receipt.tokens_out,
                    "model": receipt.model,
                })
                .to_string(),
                Err(e) => fail(e.to_string()),
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
                let rows =
                    sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
                    match handle
                        .block_on(async { sqlx::raw_sql("BEGIN").execute(&mut *conn).await })
                    {
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
            match handle
                .block_on(async { sqlx::raw_sql("COMMIT").execute(&mut *tx_state.conn).await })
            {
                Ok(_r) => {
                    let _: crate::db::pool::DbQueryResult = _r;
                    tracing::info!("[plugin:{}] transaction committed", self.plugin_id);
                    r#"{"ok":true}"#.to_string()
                }
                Err(e) => {
                    let _: Result<crate::db::pool::DbQueryResult, sqlx::Error> =
                        handle.block_on(async {
                            sqlx::raw_sql("ROLLBACK").execute(&mut *tx_state.conn).await
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
            match handle
                .block_on(async { sqlx::raw_sql("ROLLBACK").execute(&mut *tx_state.conn).await })
            {
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
                    sqlx::raw_sql("ROLLBACK").execute(&mut *tx_state.conn).await
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
    /// - A JSON object: `{"id": 1}` → auto-generated parameterized WHERE (safe)
    /// - An array: `["status = ${host.ph(1)} AND total > ${host.ph(2)}", "active", 100]`
    ///   → the SQL template must use `host.ph(N)` for placeholders; no
    ///   automatic `?` replacement is performed.
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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

        // Build WHERE clause with offset = number of SET params (placeholders
        // in WHERE must start after SET placeholders).
        let where_result = match Self::build_where_clause_with_offset(where_json, data.len()) {
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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

        let cnt_expr = crate::db::Driver::cast_int("COUNT(*)");
        let sql = format!("SELECT {cnt_expr} as cnt FROM {table}{where_sql}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let row: (i64,) = sqlx::query_as_with(crate::db::safe_sql(&sql), args)
                    .fetch_one(pool)
                    .await?;
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
                set_parts.push(format!(
                    "{col} = {}",
                    crate::db::Driver::greatest(&min_ph, &format!("{col} + {delta_ph}"))
                ));
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

        let where_result = match Self::build_where_clause_with_offset(where_json, idx - 1) {
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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

        let sum_expr = crate::db::Driver::cast_int(&format!("COALESCE(SUM({column}), 0)"));
        let sql = format!("SELECT {sum_expr} as total FROM {table}{where_sql}");
        let handle = tokio::runtime::Handle::current();
        tokio::task::block_in_place(|| {
            match handle.block_on(async {
                let row: DbRow =
                    sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
            select_parts.push(format!(
                "{} as cnt",
                crate::db::Driver::cast_int("COUNT(*)")
            ));
        }
        for col in &sum_cols {
            select_parts.push(format!(
                "{} as sum_{col}",
                crate::db::Driver::cast_int(&format!("COALESCE(SUM({col}), 0)"))
            ));
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
                sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(&sql), args)
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
        Self::build_where_clause_with_offset(where_json, 0)
    }

    fn build_where_clause_with_offset(
        where_json: &str,
        offset: usize,
    ) -> Result<WhereResult, String> {
        let trimmed = where_json.trim();
        if trimmed.is_empty() || trimmed == "null" || trimmed == "{}" {
            return Ok(WhereResult {
                clause: String::new(),
                params: Vec::new(),
            });
        }

        // Try array form: ["col = $1 AND col2 = $2", val1, val2]
        //
        // The SQL template must use `host.ph(N)` placeholders (e.g. `$1`, `$2`
        // on PostgreSQL, `?` on SQLite/MySQL). The clause is passed through
        // as-is — no automatic `?` replacement is performed, because naive
        // string replacement cannot distinguish placeholders from `?`
        // characters inside string literals or comments.
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
                parts.push(format!("{k} = {}", crate::db::Driver::ph(i + offset + 1)));
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
        sqlx::query_with::<crate::db::pool::Db, _>(crate::db::safe_sql(sql), args)
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

    // On PostgreSQL, `id BIGINT PRIMARY KEY` does not auto-increment and the
    // `tags`/`categories` tables have `UNIQUE(tenant_id, name)` / `UNIQUE(tenant_id, slug)`
    // constraints. These builders mint a fresh snowflake id and derive a unique
    // name/slug so the generated `db_insert` payloads are safe across parallel
    // test runs. They return `(json, name, slug)`.

    fn tag_json(name: &str) -> (String, String, String) {
        let id = crate::utils::id::new_id();
        let full = format!("{name}{id}");
        let slug = format!("{}-{id}", name.to_lowercase());
        (
            format!(r#"{{"id":{id},"name":"{full}","slug":"{slug}"}}"#),
            full,
            slug,
        )
    }

    fn cat_json(
        name: &str,
        sort_order: Option<i64>,
        tenant_id: Option<&str>,
    ) -> (String, String, String) {
        let id = crate::utils::id::new_id();
        let full = format!("{name}{id}");
        let slug = format!("{}-{id}", name.to_lowercase());
        let mut json = format!(r#"{{"id":{id},"name":"{full}","slug":"{slug}""#);
        if let Some(s) = sort_order {
            json.push_str(&format!(",\"sort_order\":{s}"));
        }
        if let Some(t) = tenant_id {
            json.push_str(&format!(",\"tenant_id\":\"{t}\""));
        }
        json.push('}');
        (json, full, slug)
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
    fn host_context_api_call_blocked_without_permission() {
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), Permissions::default(), None);
        let result = ctx.api_call("dify", "chat", "{}");
        assert!(result.contains("egress client not allowed"));
    }

    #[test]
    fn host_context_api_call_allowed_client_only() {
        let config = make_test_config();
        let perms = Permissions {
            egress: vec!["dify".into()],
            ..Permissions::default()
        };
        let ctx = HostContext::new("test", config, "p1".into(), perms, None);
        // Permission passes but no shared integration plane in unit tests —
        // distinct error proves the gate itself let the call through.
        let ok_client = ctx.api_call("dify", "chat", "{}");
        assert!(ok_client.contains("integration plane disabled"));
        let denied = ctx.api_call("other", "chat", "{}");
        assert!(denied.contains("egress client not allowed"));
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
        #[cfg(feature = "db-postgres")]
        {
            assert_eq!(ctx.db_ph(1), "$1");
            assert_eq!(ctx.db_ph(5), "$5");
        }
        #[cfg(not(feature = "db-postgres"))]
        {
            assert_eq!(ctx.db_ph(1), "?");
            assert_eq!(ctx.db_ph(5), "?");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_set_data_with_real_db() {
        let pool = crate::test_pool!();

        let config = make_test_config();
        let ctx = HostContext::new(
            "test",
            config,
            "plugin-a".into(),
            Permissions::default(),
            Some(pool.clone()),
        );

        let key = format!("greeting_{}", crate::utils::id::new_id());
        assert!(ctx.get_data(&key).is_none());
        assert!(ctx.set_data(&key, "hello world"));
        assert_eq!(ctx.get_data(&key), Some("hello world".into()));

        assert!(ctx.set_data(&key, "updated"));
        assert_eq!(ctx.get_data(&key), Some("updated".into()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_get_data_isolation_between_plugins() {
        let pool = crate::test_pool!();

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
        let pool = crate::test_pool!();

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
        let pool = crate::test_pool!();

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
        let pool = crate::test_pool!();

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
        let pool = crate::test_pool!();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let id = crate::utils::id::new_id();
        let slug = format!("test_{id}");
        let result = ctx.db_execute(
            &format!("INSERT INTO tags (id, name, slug) VALUES ({id}, 'Test', '{slug}')"),
            "[]",
        );
        assert!(result.contains("rows_affected"));
        assert!(!result.contains("error"));

        let update = ctx.db_execute(
            &format!("UPDATE tags SET name = 'Updated' WHERE slug = '{slug}'"),
            "[]",
        );
        assert!(update.contains("rows_affected"));

        let delete = ctx.db_execute(&format!("DELETE FROM tags WHERE slug = '{slug}'"), "[]");
        assert!(delete.contains("rows_affected"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_db_execute_parameterized() {
        let pool = crate::test_pool!();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool));

        let nid = crate::utils::id::new_id();
        let slug = format!("param-tag_{nid}");
        let insert_params = format!(r#"[{nid},"Param Tag","{slug}"]"#);
        let result = ctx.db_execute(
            &format!(
                "INSERT INTO tags (id, name, slug) VALUES ({}, {}, {})",
                ctx.db_ph(1),
                ctx.db_ph(2),
                ctx.db_ph(3)
            ),
            &insert_params,
        );
        assert!(result.contains("rows_affected"));
        assert!(!result.contains("error"));

        let update_params = format!(r#"["Renamed","{slug}"]"#);
        let update = ctx.db_execute(
            &format!(
                "UPDATE tags SET name = {} WHERE slug = {}",
                ctx.db_ph(1),
                ctx.db_ph(2)
            ),
            &update_params,
        );
        assert!(update.contains("rows_affected"));

        let delete_params = format!(r#"["{slug}"]"#);
        let delete = ctx.db_execute(
            &format!("DELETE FROM tags WHERE slug = {}", ctx.db_ph(1)),
            &delete_params,
        );
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
        let result = ctx.db_execute(
            &format!("INSERT INTO tags (name, slug) VALUES ({})", ctx.db_ph(1)),
            "not valid json",
        );
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
            &format!(
                "INSERT INTO tags (name, slug) VALUES ({}, {})",
                ctx.db_ph(1),
                ctx.db_ph(2)
            ),
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
        let pool = crate::test_pool!();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#), "begin failed: {begin}");

        let tx_id = crate::utils::id::new_id();
        let tx_slug = format!("tx-test-{tx_id}");
        let insert = ctx.db_execute(
            &format!("INSERT INTO tags (id, name, slug) VALUES ({tx_id}, 'TxTest', '{tx_slug}')"),
            "[]",
        );
        assert!(insert.contains("rows_affected"), "insert failed: {insert}");

        let commit = ctx.db_commit();
        assert!(commit.contains(r#""ok":true"#), "commit failed: {commit}");

        let rows: Vec<(String,)> = sqlx::query_as(crate::db::safe_sql(&format!(
            "SELECT name FROM tags WHERE slug = '{tx_slug}'"
        )))
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1, "row should be committed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_transaction_rollback_discards() {
        let pool = crate::test_pool!();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#));

        let rb_id = crate::utils::id::new_id();
        let rb_slug = format!("rb-test-{rb_id}");
        let insert = ctx.db_execute(
            &format!("INSERT INTO tags (id, name, slug) VALUES ({rb_id}, 'RbTest', '{rb_slug}')"),
            "[]",
        );
        assert!(insert.contains("rows_affected"));

        let rollback = ctx.db_rollback();
        assert!(rollback.contains(r#""ok":true"#));

        let count: (i64,) = sqlx::query_as(crate::db::safe_sql(&format!(
            "SELECT COUNT(*) FROM tags WHERE slug = '{rb_slug}'"
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count.0, 0, "row should be rolled back");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_transaction_double_begin_error() {
        let pool = crate::test_pool!();

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
        let pool = crate::test_pool!();

        let perms = Permissions {
            database: vec!["tags".into()],
            ..Permissions::default()
        };
        let config = make_test_config();
        let ctx = HostContext::new("test", config, "p1".into(), perms, Some(pool.clone()));

        let begin = ctx.db_begin();
        assert!(begin.contains(r#""ok":true"#));

        let cl_id = crate::utils::id::new_id();
        let cl_slug = format!("cl-test-{cl_id}");
        let insert = ctx.db_execute(
            &format!(
                "INSERT INTO tags (id, name, slug) VALUES ({cl_id}, 'CleanTest', '{cl_slug}')"
            ),
            "[]",
        );
        assert!(insert.contains("rows_affected"));

        ctx.cleanup_tx();

        let count: (i64,) = sqlx::query_as(crate::db::safe_sql(&format!(
            "SELECT COUNT(*) FROM tags WHERE slug = '{cl_slug}'"
        )))
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count.0, 0, "cleanup_tx should rollback");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_context_cleanup_tx_noop_without_active() {
        let pool = crate::test_pool!();
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
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, name, slug) = tag_json("Rust");
        let result = ctx.db_insert("tags", &json, "{}");
        assert!(result.contains(r#""rows_affected":1"#), "insert: {result}");

        let found = ctx.db_fetch_one("tags", &format!(r#"{{"slug":"{slug}"}}"#), "{}");
        assert!(
            found.contains(&format!(r#""name":"{name}""#)),
            "fetch_one: {found}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_not_found() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let found = ctx.db_fetch_one("tags", r#"{"slug":"nonexistent"}"#, "{}");
        assert!(found.contains(r#""data":null"#), "not found: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_object_where() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, name, slug) = tag_json("Go");
        let _ = ctx.db_insert("tags", &json, "{}");

        let found = ctx.db_fetch_one("tags", &format!(r#"{{"name":"{name}"}}"#), "{}");
        assert!(
            found.contains(&format!(r#""slug":"{slug}""#)),
            "string where: {found}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_one_array_where() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, name, slug) = tag_json("Python");
        let _ = ctx.db_insert("tags", &json, "{}");

        let found = ctx.db_fetch_one(
            "tags",
            &format!(r#"["name = {}", "{}"]"#, ctx.db_ph(1), name),
            "{}",
        );
        assert!(
            found.contains(&format!(r#""slug":"{slug}""#)),
            "array where: {found}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_fetch_all_with_order_and_limit() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (ja, _, _) = tag_json("A");
        let (jb, _, _) = tag_json("B");
        let (jc, _, _) = tag_json("C");
        let _ = ctx.db_insert("tags", &ja, "{}");
        let _ = ctx.db_insert("tags", &jb, "{}");
        let _ = ctx.db_insert("tags", &jc, "{}");

        let result = ctx.db_fetch_all("tags", "{}", r#"{"order_by":"name DESC","limit":2}"#);
        assert!(result.contains(r#""total":2"#), "fetch_all limit: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_update() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = tag_json("Old");
        let _ = ctx.db_insert("tags", &json, "{}");

        let new_name = format!("New{}", crate::utils::id::new_id());
        let result = ctx.db_update(
            "tags",
            &format!(r#"{{"name":"{new_name}"}}"#),
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(result.contains(r#""rows_affected":1"#), "update: {result}");

        let found = ctx.db_fetch_one("tags", &format!(r#"{{"slug":"{slug}"}}"#), "{}");
        assert!(found.contains(&new_name), "after update: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_update_empty_data() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let result = ctx.db_update("tags", "{}", "{}", "{}");
        assert!(result.contains("no columns"), "empty data: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_delete() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = tag_json("Delete");
        let _ = ctx.db_insert("tags", &json, "{}");

        let result = ctx.db_delete("tags", &format!(r#"{{"slug":"{slug}"}}"#), "{}");
        assert!(result.contains(r#""rows_affected":1"#), "delete: {result}");

        let found = ctx.db_fetch_one("tags", &format!(r#"{{"slug":"{slug}"}}"#), "{}");
        assert!(found.contains(r#""data":null"#), "after delete: {found}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_count() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (j1, _, s1) = tag_json("Count1");
        let (j2, _, s2) = tag_json("Count2");
        let _ = ctx.db_insert("tags", &j1, "{}");
        let _ = ctx.db_insert("tags", &j2, "{}");

        let result = ctx.db_count(
            "tags",
            &format!(
                r#"["slug IN ({}, {})", "{s1}", "{s2}"]"#,
                ctx.db_ph(1),
                ctx.db_ph(2)
            ),
            "{}",
        );
        assert!(result.contains(r#""count":2"#), "count: {result}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_count_with_where() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (j_go, _, _) = tag_json("Go");
        let _ = ctx.db_insert("tags", &j_go, "{}");
        let (j_rust, rust_name, _) = tag_json("Rust");
        let _ = ctx.db_insert("tags", &j_rust, "{}");

        let result = ctx.db_count(
            "tags",
            &format!(r#"["name = {}", "{}"]"#, ctx.db_ph(1), rust_name),
            "{}",
        );
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
        let pool = crate::test_pool!();
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
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = cat_json("Test", Some(0), None);
        let _ = ctx.db_insert("categories", &json, "{}");

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(r.contains(r#""rows_affected":1"#), "increment: {r}");

        let s = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s.contains(r#""sum":1"#), "after increment sum: {s}");

        let r2 = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(r2.contains(r#""rows_affected":1"#));

        let s2 = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s2.contains(r#""sum":2"#), "after 2nd sum: {s2}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_negative_delta() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = cat_json("Dec", Some(5), None);
        let _ = ctx.db_insert("categories", &json, "{}");

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-1}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(r.contains(r#""rows_affected":1"#), "decrement: {r}");

        let s = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s.contains(r#""sum":4"#), "after decrement sum: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_with_min_clamp() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = cat_json("Clamp", Some(1), None);
        let _ = ctx.db_insert("categories", &json, "{}");

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-5}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            r#"{"min":0}"#,
        );
        assert!(r.contains(r#""rows_affected":1"#), "clamp: {r}");

        let s = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s.contains(r#""sum":0"#), "clamped to 0 sum: {s}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_increment_with_set() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = cat_json("Set", Some(0), None);
        let _ = ctx.db_insert("categories", &json, "{}");

        let set_name = format!("Updated{}", crate::utils::id::new_id());
        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":1}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            &format!(r#"{{"set":{{"name":"{set_name}"}}}}"#),
        );
        assert!(r.contains(r#""rows_affected":1"#), "increment+set: {r}");

        let s = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s.contains(r#""sum":1"#), "incremented sum: {s}");

        let found = ctx.db_fetch_one("categories", &format!(r#"{{"slug":"{slug}"}}"#), "{}");
        assert!(
            found.contains(&format!(r#""name":"{set_name}""#)),
            "set col: {found}"
        );
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
        let pool = crate::test_pool!();
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
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (ja, _, s1) = cat_json("A", Some(3), None);
        let (jb, _, s2) = cat_json("B", Some(7), None);
        let _ = ctx.db_insert("categories", &ja, "{}");
        let _ = ctx.db_insert("categories", &jb, "{}");

        let r = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(
                r#"["slug IN ({}, {})", "{s1}", "{s2}"]"#,
                ctx.db_ph(1),
                ctx.db_ph(2)
            ),
            "{}",
        );
        assert!(r.contains(r#""sum":10"#), "sum: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_sum_empty() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let empty_slug = format!("__never_{}__", crate::utils::id::new_id());
        let r = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{empty_slug}"}}"#),
            "{}",
        );
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
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (j1, _, s1) = cat_json("Rust1", None, Some("t1"));
        let (j2, _, s2) = cat_json("Go", None, Some("t2"));
        let (j3, _, s3) = cat_json("Rust2", None, Some("t1"));
        let _ = ctx.db_insert("categories", &j1, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j2, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j3, r#"{"tenant":"disabled"}"#);

        let r = ctx.db_group_by(
            "categories",
            &format!(
                r#"{{"group_by":"tenant_id","count":true,"order_by":"cnt DESC","tenant":false,"where":["slug IN ({}, {}, {})", "{s1}", "{s2}", "{s3}"]}}"#,
                ctx.db_ph(1), ctx.db_ph(2), ctx.db_ph(3)
            ),
        );
        assert!(r.contains(r#""total":2"#), "group_by count: {r}");
        assert!(r.contains(r#""tenant_id":"t1""#), "group_by t1: {r}");
        assert!(r.contains(r#""cnt":2"#), "group_by cnt: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_sum() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (j1, _, s1) = cat_json("A1", Some(3), Some("grpA"));
        let (j2, _, s2) = cat_json("A2", Some(7), Some("grpA"));
        let (j3, _, s3) = cat_json("B1", Some(2), Some("grpB"));
        let _ = ctx.db_insert("categories", &j1, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j2, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j3, r#"{"tenant":"disabled"}"#);

        let r = ctx.db_group_by(
            "categories",
            &format!(
                r#"{{"group_by":"tenant_id","count":true,"sum":"sort_order","order_by":"sum_sort_order DESC","tenant":false,"where":["slug IN ({}, {}, {})", "{s1}", "{s2}", "{s3}"]}}"#,
                ctx.db_ph(1), ctx.db_ph(2), ctx.db_ph(3)
            ),
        );
        assert!(r.contains(r#""total":2"#), "group_by sum total: {r}");
        assert!(
            r.contains(r#""sum_sort_order":10"#),
            "group_by sum grpA: {r}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_where() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (j1, _, s1) = cat_json("W1", None, Some("tA"));
        let (j2, _, s2) = cat_json("W2", None, Some("tA"));
        let (j3, _, _) = cat_json("W3", None, Some("tB"));
        let _ = ctx.db_insert("categories", &j1, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j2, r#"{"tenant":"disabled"}"#);
        let _ = ctx.db_insert("categories", &j3, r#"{"tenant":"disabled"}"#);

        let r = ctx.db_group_by(
            "categories",
            &format!(
                r#"{{"group_by":"tenant_id","count":true,"where":["slug IN ({}, {})", "{s1}", "{s2}"],"tenant":false}}"#,
                ctx.db_ph(1), ctx.db_ph(2)
            ),
        );
        assert!(r.contains(r#""total":1"#), "group_by where: {r}");
        assert!(r.contains(r#""cnt":2"#), "group_by where cnt: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_with_limit() {
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (jx, _, _) = cat_json("X", None, None);
        let (jy, _, _) = cat_json("Y", None, None);
        let (jz, _, _) = cat_json("Z", None, None);
        let _ = ctx.db_insert("categories", &jx, "{}");
        let _ = ctx.db_insert("categories", &jy, "{}");
        let _ = ctx.db_insert("categories", &jz, "{}");

        let r = ctx.db_group_by(
            "categories",
            r#"{"group_by":"name","count":true,"limit":2,"order_by":"name ASC"}"#,
        );
        assert!(r.contains(r#""total":2"#), "group_by limit: {r}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_group_by_missing_field() {
        let pool = crate::test_pool!();
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
        let pool = crate::test_pool!();
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
        let pool = crate::test_pool!();
        let ctx = make_crud_ctx(&pool);

        let (json, _, slug) = cat_json("Multi", Some(2), None);
        let _ = ctx.db_insert("categories", &json, "{}");

        let r = ctx.db_increment(
            "categories",
            r#"{"sort_order":-10}"#,
            &format!(r#"{{"slug":"{slug}"}}"#),
            r#"{"min":0}"#,
        );
        assert!(r.contains(r#""rows_affected":1"#), "multi clamp: {r}");

        let s = ctx.db_sum(
            "categories",
            "sort_order",
            &format!(r#"{{"slug":"{slug}"}}"#),
            "{}",
        );
        assert!(s.contains(r#""sum":0"#), "clamped sum: {s}");
    }
}
