//! # raisfast-derive
//!
//! Proc-macro crate for the `raisfast` blog/CMS system.
//!
//! Provides two categories of macros:
//!
//! ## 1. Derive macros
//!
//! - **`#[derive(EventMeta)]`** — auto-generates `name()`, `display_name()`, `table()` methods
//!   on event enums, with per-variant `#[event(table = "...", name = "...", dynamic)]` attributes.
//!
//! ## 2. Attribute macros
//!
//! - **`#[aspect_service(entity = "...", model = Type)]`** — generates a service struct with
//!   before/after hooks that delegate to the aspect engine, and auto-emits domain events.
//!
//! ## 3. Bang macros (SQL CRUD helpers)
//!
//! All macros use optional `tenant: expr` named section for tenant filtering.
//! When provided, the SQL includes `AND tenant_id = ?` at runtime.
//!
//! | Macro | SQL operation |
//! |-------|---------------|
//! | `crud_delete!` | `DELETE FROM ... WHERE WhereExpr` |
//! | `crud_insert!` | `INSERT INTO ... (...) VALUES (...)` |
//! | `crud_scalar!` | `SELECT scalar ...` |
//! | `crud_query!` | `SELECT ...` via `query_as` |
//! | `crud_find!` | `SELECT cols FROM ... WHERE WhereExpr` → `fetch_optional` |
//! | `crud_find_one!` | same → `fetch_one` |
//! | `crud_find_all!` | same → `fetch_all` |
//! | `crud_list!` | `SELECT cols FROM ...` → `fetch_all` (no WHERE) |
//! | `crud_update!` | `UPDATE ... SET ... WHERE WhereExpr` |
//! | `crud_count!` | `SELECT COUNT(*) FROM ... WHERE WhereExpr` |
//! | `crud_exists!` | `SELECT EXISTS(SELECT 1 ... WHERE WhereExpr)` |
//! | `crud_query_paged!` | paginated data + COUNT |
//! | `crud_join_paged!` | paginated JOIN + COUNT |
//! | `crud_resolve_id!` | `SELECT id FROM table WHERE id = ?` → `Option<i64>` |
//! | `crud_resolve_ids!` | `SELECT id FROM table WHERE id IN (...)` → `Vec<i64>` (validates all exist) |
//!
//! ### Schema validation
//!
//! - **`check_schema!("table", "col1", "col2", ...)`** — compile-time validation only;
//!   expands to nothing. Emits a compile error if the table or any column is missing.
//!
//! ## Architecture notes
//!
//! - `schema.rs` — parses `schema.sqlite.sql` + `tenantable.sqlite.sql` at compile time
//!   into a `Schema` struct. Used for table/column validation and for generating explicit
//!   column lists (replacing `SELECT *`).
//! - `crud.rs` — all CRUD macro implementations + input parsing structs.
//! - `event_meta.rs` — `#[derive(EventMeta)]`.
//! - `aspect_service.rs` — `#[aspect_service]`.

mod aspect_service;
mod crud;
mod event_meta;
mod schema;
mod where_dsl;

use proc_macro::TokenStream;

/// Derive macro for event enums.
///
/// Generates `name()`, `display_name()`, `table()` methods.
/// Supports per-variant attributes: `#[event(table = "...", name = "...", dynamic)]`.
#[proc_macro_derive(EventMeta, attributes(event))]
pub fn derive_event_meta(input: TokenStream) -> TokenStream {
    event_meta::derive_event_meta(input)
}

/// `crud_delete!(pool, "table", where: WhereExpr [, tenant: expr])`
///
/// Generates `DELETE FROM table WHERE ...` via `sqlx::query()`.
/// Uses Where DSL for conditions. When `tenant:` is provided, adds `AND tenant_id = ?` filter.
#[proc_macro]
pub fn crud_delete(input: TokenStream) -> TokenStream {
    crud::crud_delete(input)
}

/// `crud_insert!(pool, "table", ["col1" => val1, "col2" => val2] [, tenant: expr])`
///
/// Generates `INSERT INTO table (cols) VALUES (placeholders)` via `sqlx::query!()`.
/// Values are pre-bound to `let` variables to avoid E0716 temporary lifetime issues.
#[proc_macro]
pub fn crud_insert(input: TokenStream) -> TokenStream {
    crud::crud_insert(input)
}

/// `crud_scalar!(pool, Type, sql, [vals], method [, tenant: expr])`
///
/// Generates `sqlx::query_scalar::<_, Type>(sql)` with binds. Runtime query.
#[proc_macro]
pub fn crud_scalar(input: TokenStream) -> TokenStream {
    crud::crud_scalar(input)
}

/// `crud_select!(pool, "table", ["col1", "col2"], where: WhereExpr [, tenant: expr])`
///
/// Generates `SELECT col1, col2 FROM table WHERE ...` via `sqlx::query_as`.
#[proc_macro]
pub fn crud_select(input: TokenStream) -> TokenStream {
    crud::crud_select(input)
}

/// `crud_join!(pool, Type, select: [...], from: "...", joins: [...], where: WhereExpr [, tenant: expr, method: fetch_all, order_by: "...", limit: expr, offset: expr])`
///
/// Generates a JOIN query with optional Where DSL conditions and tenant filtering.
#[proc_macro]
pub fn crud_join(input: TokenStream) -> TokenStream {
    crud::crud_join(input)
}

/// `crud_join_paged!(pool, Type, select: [...], from: "...", joins: [...], tenant_alias: "...", tenant: tid, order_by: "...", page: page, page_size: page_size)`
///
/// Generates a paginated JOIN query with COUNT. Returns `(Vec<T>, i64)`.
#[proc_macro]
pub fn crud_join_paged(input: TokenStream) -> TokenStream {
    crud::crud_join_paged(input)
}

/// `crud_count!(pool, "table", where: WhereExpr [, tenant: expr])`
///
/// `SELECT COUNT(*) FROM table WHERE ...` → `i64`.
#[proc_macro]
pub fn crud_count(input: TokenStream) -> TokenStream {
    crud::crud_count(input)
}

/// `crud_query!(pool, Type, sql, [vals], method [, tenant: expr])`
///
/// Generates `sqlx::query_as::<_, Type>(sql)` with binds. Runtime query.
#[proc_macro]
pub fn crud_query(input: TokenStream) -> TokenStream {
    crud::crud_query(input)
}

/// `crud_find!(pool, "table", Type, where: WhereExpr [, tenant: expr, order_by: "expr"])`
///
/// `SELECT {all_columns} FROM table WHERE ...` → `fetch_optional`.
/// Column list is generated from schema (replaces `SELECT *`).
#[proc_macro]
pub fn crud_find(input: TokenStream) -> TokenStream {
    crud::crud_find(input)
}

/// `crud_find_one!(...)` — same as `crud_find!` but uses `fetch_one`.
#[proc_macro]
pub fn crud_find_one(input: TokenStream) -> TokenStream {
    crud::crud_find_one(input)
}

/// `crud_find_all!(...)` — same as `crud_find!` but uses `fetch_all`.
#[proc_macro]
pub fn crud_find_all(input: TokenStream) -> TokenStream {
    crud::crud_find_all(input)
}

/// `crud_list!(pool, "table", Type)`
///
/// `SELECT {all_columns} FROM table` → `fetch_all`. No WHERE clause.
/// Optional: `order_by: "expr"` — appends `ORDER BY expr`.
/// Optional: `tenant: tid` — adds `WHERE 1=1 AND tenant_id = ?` filter.
///
/// ```ignore
/// crud_list!(pool, "tags" => Tag)
/// crud_list!(pool, "tags" => Tag, order_by: "name")
/// crud_list!(pool, "tags" => Tag, order_by: "name", tenant: tenant_id)
/// ```
#[proc_macro]
pub fn crud_list(input: TokenStream) -> TokenStream {
    crud::crud_list(input)
}

/// `check_schema!("table", "col1", "col2", ...)`
///
/// Compile-time validation only — expands to nothing (empty token stream).
/// Emits a compile error if the table or any named column is missing from the schema.
#[proc_macro]
pub fn check_schema(input: TokenStream) -> TokenStream {
    crud::check_schema(input)
}

/// `crud_exists!(pool, "table", where: WhereExpr [, tenant: expr])`
///
/// `SELECT EXISTS(SELECT 1 FROM table WHERE ...)` → `bool`.
/// Uses `sqlx::query_scalar` with compile-time verified SQL.
#[proc_macro]
pub fn crud_exists(input: TokenStream) -> TokenStream {
    crud::crud_exists(input)
}

/// `crud_upsert!(pool, "table", key: ["conflict_col"], bind: ["col" => val, ...], update: ["col1", "col2"] [, tenant: expr])`
///
/// Generates `INSERT INTO table (...) VALUES (...) ON CONFLICT(...) DO UPDATE SET ...`
/// via `sqlx::query!()` (compile-time verified).
#[proc_macro]
pub fn crud_upsert(input: TokenStream) -> TokenStream {
    crud::crud_upsert(input)
}

/// `crud_update!(pool, "table", bind: [...], optional: [...], raw: [...], where: WhereExpr [, tenant: tid])`
///
/// Generates a runtime `sqlx::query()` UPDATE. Supports `bind:` (always-set),
/// `optional:` (set only when Some), `raw:` (SQL expressions).
#[proc_macro]
pub fn crud_update(input: TokenStream) -> TokenStream {
    crud::crud_update(input)
}

/// `crud_query_paged!(pool, Type, data_sql: "...", count_sql: "...", binds: [...], where: ["col" => opt_val, ...], tenant: tid, page: page, page_size: page_size)`
///
/// Generates a paginated query pair (data + COUNT) with optional tenant filtering
/// and optional dynamic WHERE conditions.
///
/// Both `data_sql` and `count_sql` are string literals. Use `{tenant}` as a placeholder
/// where the `AND tenant_id = ?` clause should be inserted when tenant_id is Some.
///
/// The `where:` section accepts optional values — `Some(val)` appends `AND col = ?`
/// and binds, `None` skips. Values must be `Option` types.
///
/// Returns `(Vec<T>, i64)` — the data rows and total count.
#[proc_macro]
pub fn crud_query_paged(input: TokenStream) -> TokenStream {
    crud::crud_query_paged(input)
}

/// `crud_resolve_id!(pool, table, id [, tenant: expr])`
///
/// Resolves a SnowflakeId to an `i64` by verifying it exists in the target table.
/// Returns `sqlx::Result<Option<i64>>` — `Some(id)` if found, `None` if not.
///
/// The `table` parameter is a runtime `&str` expression (not a literal).
/// Performs `is_safe_identifier()` check before querying.
/// When `tenant:` is provided, adds `AND tenant_id = ?` filter.
#[proc_macro]
pub fn crud_resolve_id(input: TokenStream) -> TokenStream {
    crud::crud_resolve_id(input)
}

/// `crud_resolve_ids!(pool, table, ids)`
///
/// Batch version of `crud_resolve_id!`. Resolves a slice of `i64` IDs by verifying
/// they ALL exist in the target table via a single `SELECT id FROM table WHERE id IN (...)`.
///
/// Returns `Result<Vec<i64>, sqlx::Error>` — the validated ID list on success.
/// Returns `sqlx::Error::RowNotFound` if any ID is missing.
/// Returns error on unsafe table name.
#[proc_macro]
pub fn crud_resolve_ids(input: TokenStream) -> TokenStream {
    crud::crud_resolve_ids(input)
}

/// `#[aspect_service(entity = "posts", model = Post)]`
///
/// Attribute macro applied to a service struct. Generates:
/// - `new(...)` constructor
/// - `before_create(...)` / `before_update(...)` / `before_delete(...)` — delegate to aspect engine
/// - `after_created(...)` / `after_updated(...)` / `after_deleted(...)` — emit domain events
///
/// The struct must have one field marked with `#[engine]` pointing to the aspect engine.
#[proc_macro_attribute]
pub fn aspect_service(attr: TokenStream, item: TokenStream) -> TokenStream {
    aspect_service::aspect_service(attr, item)
}
