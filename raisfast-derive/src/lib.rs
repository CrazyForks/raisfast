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
//! These replace the old `macro_rules!` SQL macros with proc-macro equivalents. At macro
//! expansion time they:
//! 1. Parse the caller's token stream into structured input.
//! 2. Validate table/column names against the SQL schema parsed from migration files.
//! 3. Emit code containing `sqlx::query!()` / `sqlx::query_as!()` / `sqlx::query_scalar!()`
//!    calls for compile-time SQL verification (for non-tenant variants), or runtime
//!    `sqlx::query()` / `sqlx::query_as()` for tenant-aware variants (because the SQL string
//!    depends on whether `tenant_id` is `Some` or `None` at runtime).
//!
//! ### Tenant-aware macros (with `tid` parameter)
//!
//! | Macro | SQL operation |
//! |-------|---------------|
//! | `tenant_delete!` | `DELETE FROM ... WHERE col = ? AND tenant_id = ?` |
//! | `tenant_insert!` | `INSERT INTO ... (..., tenant_id) VALUES (..., ?)` |
//! | `tenant_scalar!` | `SELECT scalar ... [AND tenant_id = ?]` |
//! | `tenant_query!` | `SELECT ... [AND tenant_id = ?]` via `query_as` |
//! | `tenant_find!` | `SELECT cols FROM ... WHERE col = ? [AND tenant_id = ?]` → `fetch_optional` |
//! | `tenant_find_one!` | same → `fetch_one` |
//! | `tenant_find_all!` | same → `fetch_all` |
//! | `tenant_update!` | `UPDATE ... SET ... WHERE pk = ? [AND tenant_id = ?]` |
//!
//! ### CRUD macros (no tenant)
//!
//! | Macro | SQL operation |
//! |-------|---------------|
//! | `crud_delete!` | `DELETE FROM ... WHERE col = ?` |
//! | `crud_insert!` | `INSERT INTO ... (...) VALUES (...)` |
//! | `crud_scalar!` | `SELECT scalar ...` |
//! | `crud_query!` | `SELECT ...` via `query_as` |
//! | `crud_find!` | `SELECT cols FROM ... WHERE col = ?` → `fetch_optional` |
//! | `crud_find_one!` | same → `fetch_one` |
//! | `crud_find_all!` | same → `fetch_all` |
//! | `crud_list!` | `SELECT cols FROM ...` → `fetch_all` (no WHERE) |
//! | `crud_update!` | `UPDATE ... SET ... WHERE pk = ?` |
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

use proc_macro::TokenStream;

/// Derive macro for event enums.
///
/// Generates `name()`, `display_name()`, `table()` methods.
/// Supports per-variant attributes: `#[event(table = "...", name = "...", dynamic)]`.
#[proc_macro_derive(EventMeta, attributes(event))]
pub fn derive_event_meta(input: TokenStream) -> TokenStream {
    event_meta::derive_event_meta(input)
}

/// `tenant_delete!(pool, "table", "col" => val, tenant_id)`
///
/// Generates a match on `tenant_id`:
/// - `Some(tid)` → `DELETE FROM table WHERE col = ?1 AND tenant_id = ?2`
/// - `None` → `DELETE FROM table WHERE col = ?1`
///
/// Uses `sqlx::query!()` for compile-time SQL validation.
#[proc_macro]
pub fn tenant_delete(input: TokenStream) -> TokenStream {
    crud::tenant_delete(input)
}

/// `crud_delete!(pool, "table", "col" => val)`
///
/// Generates `DELETE FROM table WHERE col = ?1` via `sqlx::query!()`.
#[proc_macro]
pub fn crud_delete(input: TokenStream) -> TokenStream {
    crud::crud_delete(input)
}

/// `tenant_insert!(pool, "table", ["col1" => val1, "col2" => val2], tenant_id)`
///
/// Generates a match on `tenant_id`:
/// - `Some(tid)` → INSERT with extra `tenant_id` column and bind
/// - `None` → INSERT without `tenant_id`
///
/// Uses `sqlx::query!()` for compile-time SQL validation.
/// Values are pre-bound to `let` variables to avoid E0716 temporary lifetime issues.
#[proc_macro]
pub fn tenant_insert(input: TokenStream) -> TokenStream {
    crud::tenant_insert(input)
}

/// `crud_insert!(pool, "table", ["col1" => val1, "col2" => val2])`
///
/// Generates `INSERT INTO table (cols) VALUES (placeholders)` via `sqlx::query!()`.
/// Values are pre-bound to `let` variables to avoid E0716 temporary lifetime issues.
#[proc_macro]
pub fn crud_insert(input: TokenStream) -> TokenStream {
    crud::crud_insert(input)
}

/// `tenant_scalar!(pool, Type, sql, [vals], tenant_id, method)`
///
/// Generates a runtime `sqlx::query_scalar::<_, Type>(sql)` with optional tenant bind.
/// Uses runtime query (not `query!`) because the SQL string is caller-provided.
#[proc_macro]
pub fn tenant_scalar(input: TokenStream) -> TokenStream {
    crud::tenant_scalar(input)
}

/// `crud_scalar!(pool, Type, sql, [vals], method)`
///
/// Generates `sqlx::query_scalar::<_, Type>(sql)` with binds. Runtime query.
#[proc_macro]
pub fn crud_scalar(input: TokenStream) -> TokenStream {
    crud::crud_scalar(input)
}

/// `tenant_select!(pool, "table", ["col1", "col2"], "where_col" => val, tenant_id)`
///
/// Generates `SELECT col1, col2 FROM table WHERE where_col = ? AND tenant_id = ?` via `sqlx::query_as`.
/// Returns `Option<(col1_type, col2_type, ...)>`. Supports optional `and:` parameter.
#[proc_macro]
pub fn tenant_select(input: TokenStream) -> TokenStream {
    crud::tenant_select(input)
}

/// `crud_select!(pool, "table", ["col1", "col2"], "where_col" => val)`
///
/// Same as `tenant_select!` but without tenant filtering.
#[proc_macro]
pub fn crud_select(input: TokenStream) -> TokenStream {
    crud::crud_select(input)
}

/// `tenant_join!(pool, Type, select: [...], from: "...", joins: [LEFT "table" ON "..."], where: "col" => val, tenant_alias: "...", tenant: tid, method: fetch_one)`
///
/// Generates a JOIN query with tenant filtering.
/// `joins:` entries use keywords `INNER`/`LEFT`/`RIGHT` before the table name.
#[proc_macro]
pub fn tenant_join(input: TokenStream) -> TokenStream {
    crud::tenant_join(input)
}

/// `crud_join!(pool, Type, select: [...], from: "...", joins: [...], where: "col" => val, method: fetch_all)`
///
/// Same as `tenant_join!` but without tenant filtering.
#[proc_macro]
pub fn crud_join(input: TokenStream) -> TokenStream {
    crud::crud_join(input)
}

/// `tenant_join_paged!(pool, Type, select: [...], from: "...", joins: [...], and: [...], tenant_alias: "...", tenant: tid, order_by: "...", page: page, page_size: page_size)`
///
/// Generates a paginated JOIN query with COUNT. Returns `(Vec<T>, i64)`.
#[proc_macro]
pub fn tenant_join_paged(input: TokenStream) -> TokenStream {
    crud::tenant_join_paged(input)
}

/// `tenant_count!(pool, "table", "col" => val, tenant_id [, and: ["c" => v, ...]])`
///
/// `SELECT COUNT(*) FROM table WHERE col = ? [AND c = ? ...] [AND tenant_id = ?]` → `i64`.
#[proc_macro]
pub fn tenant_count(input: TokenStream) -> TokenStream {
    crud::tenant_count(input)
}

/// `crud_count!(pool, "table", "col" => val [, and: ["c" => v, ...]])`
///
/// `SELECT COUNT(*) FROM table WHERE col = ? [AND c = ? ...]` → `i64`.
#[proc_macro]
pub fn crud_count(input: TokenStream) -> TokenStream {
    crud::crud_count(input)
}

/// `tenant_query!(pool, Type, sql, [vals], tenant_id, method)`
///
/// Generates `sqlx::query_as::<_, Type>(sql)` with optional tenant bind. Runtime query.
#[proc_macro]
pub fn tenant_query(input: TokenStream) -> TokenStream {
    crud::tenant_query(input)
}

/// `crud_query!(pool, Type, sql, [vals], method)`
///
/// Generates `sqlx::query_as::<_, Type>(sql)` with binds. Runtime query.
#[proc_macro]
pub fn crud_query(input: TokenStream) -> TokenStream {
    crud::crud_query(input)
}

/// `tenant_find!(pool, "table", Type, "col" => val, tenant_id)`
///
/// `SELECT {all_columns} FROM table WHERE col = ? [AND tenant_id = ?]` → `fetch_optional`.
/// Uses runtime `query_as` because the SQL depends on tenant_id at runtime.
/// Column list is generated from schema (replaces `SELECT *`).
#[proc_macro]
pub fn tenant_find(input: TokenStream) -> TokenStream {
    crud::tenant_find(input)
}

/// `tenant_find_one!(...)` — same as `tenant_find!` but uses `fetch_one`.
#[proc_macro]
pub fn tenant_find_one(input: TokenStream) -> TokenStream {
    crud::tenant_find_one(input)
}

/// `tenant_find_all!(...)` — same as `tenant_find!` but uses `fetch_all`.
#[proc_macro]
pub fn tenant_find_all(input: TokenStream) -> TokenStream {
    crud::tenant_find_all(input)
}

/// `crud_find!(pool, "table", Type, "col" => val)`
///
/// `SELECT {all_columns} FROM table WHERE col = ?` → `fetch_optional`.
/// Uses runtime `query_as` (consistency with tenant variants).
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

/// `tenant_update!(pool, "table", bind: [...], raw: [...], where: "pk" => val, and: [...], tenant: tid)`
///
/// Generates a runtime `sqlx::query()` UPDATE with optional tenant filter.
/// Uses runtime query because the SQL WHERE clause depends on tenant_id at runtime.
/// Values are pre-bound to `let` variables to avoid E0716 temporary lifetime issues.
#[proc_macro]
pub fn tenant_update(input: TokenStream) -> TokenStream {
    crud::tenant_update(input)
}

/// `crud_update!(pool, "table", bind: [...], raw: [...], where: "pk" => val, and: [...])`
///
/// Generates a runtime `sqlx::query()` UPDATE without tenant filtering.
/// Values are pre-bound to `let` variables to avoid E0716 temporary lifetime issues.
#[proc_macro]
pub fn crud_update(input: TokenStream) -> TokenStream {
    crud::crud_update(input)
}

/// `tenant_query_paged!(pool, Type, data_sql: "...", count_sql: "...", binds: [...], where: ["col" => opt_val, ...], tenant: tid, page: page, page_size: page_size)`
///
/// Generates a paginated query pair (data + COUNT) with optional tenant filtering
/// and optional dynamic WHERE conditions.
///
/// Both `data_sql` and `count_sql` are string literals. Use `{tenant}` as a placeholder
/// where the `AND tenant_id = ?` clause should be inserted when tenant_id is Some.
/// Use `?` for parameter placeholders — the macro replaces them with `?N` automatically.
///
/// The `where:` section accepts optional values — `Some(val)` appends `AND col = ?`
/// and binds, `None` skips. Values must be `Option` types.
///
/// Returns `(Vec<T>, i64)` — the data rows and total count.
///
/// # Example
///
/// ```ignore
/// // With optional status filter:
/// let (products, total) = tenant_query_paged!(
///     pool, Product,
///     data_sql: "SELECT * FROM products WHERE 1=1{tenant} ORDER BY created_at DESC",
///     count_sql: "SELECT COUNT(*) FROM products WHERE 1=1{tenant}",
///     binds: [],
///     where: ["status" => status],
///     tenant: tenant_id,
///     page: page,
///     page_size: page_size
/// )?;
/// ```
#[proc_macro]
pub fn tenant_query_paged(input: TokenStream) -> TokenStream {
    crud::tenant_query_paged(input)
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
