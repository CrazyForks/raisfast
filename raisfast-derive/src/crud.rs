//! CRUD proc-macro implementations.
//!
//! This module implements all the `tenant_*!` and `crud_*!` bang macros, plus
//! `check_schema!`. Each macro follows the same pattern:
//!
//! 1. **Parse** the input tokens into a structured input struct (e.g., `TenantDeleteInput`).
//! 2. **Validate** table and column names against the compile-time schema.
//! 3. **Expand** into Rust code that calls `sqlx::query!()` (compile-time verified) or
//!    `sqlx::query()` / `sqlx::query_as()` (runtime, for tenant-dependent SQL).
//!
//! # Design decisions
//!
//! ## Tenant-aware macros use runtime SQL
//!
//! Tenant-aware macros cannot use `sqlx::query!()` because the SQL string must be a
//! literal known at compile time, but the tenant filter (`AND tenant_id = ?`) depends
//! on whether `tenant_id` is `Some` or `None` at runtime. The solution is a `match`:
//!
//! ```ignore
//! match tid {
//!     Some(_tid) => /* SQL with tenant_id filter */,
//!     None => /* SQL without tenant_id filter */,
//! }
//! ```
//!
//! For `tenant_delete!` and `tenant_insert!`, both branches use `sqlx::query!()` with
//! separate string literals, achieving full compile-time verification for each path.
//!
//! For `tenant_find!` and `tenant_update!`, the SQL is built at runtime via `format!()`
//! because the column lists and WHERE clauses are dynamically constructed.
//!
//! ## E0716 temporary lifetime fix
//!
//! When `sqlx::query!()` is used inside a `match` expression, Rust's temporary lifetime
//! rules can cause E0716 errors. The fix is to pre-bind all values to `let` variables
//! before the `match`:
//!
//! ```ignore
//! {
//!     let __vi_0 = val0;
//!     let __vi_1 = val1;
//!     match tid {
//!         Some(_tid) => sqlx::query!(sql_with, __vi_0, __vi_1, _tid)...
//!         None => sqlx::query!(sql_without, __vi_0, __vi_1)...
//!     }
//! }
//! ```
//!
//! ## SELECT * replacement
//!
//! `sqlx::query!()` does not support `SELECT *` — all columns must be explicitly listed.
//! The `get_select_columns()` function uses `Schema::column_names()` to generate the
//! column list at macro expansion time from the parsed migration files.

use proc_macro::TokenStream;
use quote::quote;
use syn::ext::IdentExt;
use syn::parse_macro_input;
use syn::spanned::Spanned;

use crate::schema::{Dialect, Schema};

static SCHEMA: std::sync::OnceLock<Schema> = std::sync::OnceLock::new();

fn get_schema() -> &'static Schema {
    SCHEMA.get_or_init(Schema::load)
}

fn dialect() -> Dialect {
    get_schema().dialect
}

/// Validate that a table exists in the schema. Returns `None` if valid, or a compile error.
fn validate_table(table: &syn::LitStr) -> Option<TokenStream> {
    if get_schema().tables.contains_key(&table.value()) {
        return None;
    }
    Some(
        syn::Error::new(
            table.span(),
            format!("table \"{}\" not found in schema", table.value()),
        )
        .to_compile_error()
        .into(),
    )
}

/// Validate that a column exists in a table. Returns `None` if valid, or a compile error.
fn validate_column(table: &syn::LitStr, col: &syn::LitStr) -> Option<TokenStream> {
    let table_str = table.value();
    let col_str = col.value();
    if let Some(ts) = get_schema().tables.get(&table_str)
        && ts.columns.iter().any(|c| c.name == col_str)
    {
        return None;
    }
    Some(
        syn::Error::new(
            col.span(),
            format!(
                "column \"{}\" not found in table \"{}\"",
                col_str, table_str
            ),
        )
        .to_compile_error()
        .into(),
    )
}

/// Validate multiple columns. Returns the first error, or `None` if all valid.
fn validate_columns(table: &syn::LitStr, cols: &[syn::LitStr]) -> Option<TokenStream> {
    for col in cols {
        if let Some(err) = validate_column(table, col) {
            return Some(err);
        }
    }
    None
}

// ── tenant_delete! / crud_delete! ────────────────────────────────────────

pub fn tenant_delete(input: TokenStream) -> TokenStream {
    expand_delete(input, true)
}

pub fn crud_delete(input: TokenStream) -> TokenStream {
    expand_delete(input, false)
}

/// Expand a DELETE macro.
///
/// - **tenant** variant: emits a `match` on `tid` with two `sqlx::query!()` branches
///   (with/without `AND tenant_id = ?`). Both SQL strings are literals → compile-time verified.
/// - **crud** variant: single `sqlx::query!()` call.
fn expand_delete(input: TokenStream, with_tenant: bool) -> TokenStream {
    if with_tenant {
        let parsed = parse_macro_input!(input as TenantDeleteInput);
        let table = &parsed.table;
        let col = &parsed.col;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_column(table, col) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_str = col.value();

        // Two separate SQL literals — sqlx::query!() validates each at compile time
        let d = dialect();
        let sql_with_tenant = syn::LitStr::new(
            &format!(
                "DELETE FROM {} WHERE {} = {} AND tenant_id = {}",
                table_str, col_str, d.ph(1), d.ph(2)
            ),
            table.span(),
        );
        let sql_without_tenant = syn::LitStr::new(
            &format!("DELETE FROM {} WHERE {} = {}", table_str, col_str, d.ph(1)),
            table.span(),
        );
        let expanded = quote! {
            match #tid {
                Some(_tid) => sqlx::query!(#sql_with_tenant, #val, _tid).execute(#pool).await,
                None => sqlx::query!(#sql_without_tenant, #val).execute(#pool).await,
            }
        };
        TokenStream::from(expanded)
    } else {
        let parsed = parse_macro_input!(input as CrudDeleteInput);
        let table = &parsed.table;
        let col = &parsed.col;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_column(table, col) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let table_str = table.value();
        let col_str = col.value();

        let sql_lit = syn::LitStr::new(
            &format!("DELETE FROM {} WHERE {} = {}", table_str, col_str, dialect().ph(1)),
            table.span(),
        );
        let expanded = quote! {
            sqlx::query!(#sql_lit, #val).execute(#pool).await
        };
        TokenStream::from(expanded)
    }
}

// ── tenant_insert! / crud_insert! ────────────────────────────────────────

pub fn tenant_insert(input: TokenStream) -> TokenStream {
    expand_insert(input, true)
}

pub fn crud_insert(input: TokenStream) -> TokenStream {
    expand_insert(input, false)
}

/// Expand an INSERT macro.
///
/// - **tenant** variant: two SQL literals (with/without `tenant_id` column), match on `tid`.
///   Uses `sqlx::query!()` for both branches — compile-time verified.
/// - **crud** variant: single `sqlx::query!()` call.
///
/// Both variants use `let` pre-binding to avoid E0716 temporary lifetime errors
/// when values are used inside `match` arms.
fn expand_insert(input: TokenStream, with_tenant: bool) -> TokenStream {
    if with_tenant {
        let parsed = parse_macro_input!(input as TenantInsertInput);
        let table = &parsed.table;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_columns(table, &parsed.cols) {
            return err;
        }

        let pool = &parsed.pool;
        let vals = &parsed.vals;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_strs: Vec<String> = parsed.cols.iter().map(|l| l.value()).collect();

        let col_list = col_strs.join(", ");
        let n = col_strs.len();
        let d = dialect();
        let col_list_with = format!("{}, tenant_id", col_list);
        let ph_with: Vec<String> = (1..=n + 1).map(|i| d.ph(i)).collect();
        let ph: Vec<String> = (1..=n).map(|i| d.ph(i)).collect();
        let sql_with = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                table_str,
                col_list_with,
                ph_with.join(", ")
            ),
            table.span(),
        );
        let sql_without = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                table_str,
                col_list,
                ph.join(", ")
            ),
            table.span(),
        );
        // E0716 fix: pre-bind values to named locals before the match
        let val_idents: Vec<syn::Ident> = (0..vals.len())
            .map(|i| syn::Ident::new(&format!("__vi_{}", i), proc_macro2::Span::call_site()))
            .collect();
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                match #tid {
                    Some(_tid) => sqlx::query!(#sql_with, #(#val_idents),*, _tid).execute(#pool).await,
                    None => sqlx::query!(#sql_without, #(#val_idents),*).execute(#pool).await,
                }
            }
        };
        TokenStream::from(expanded)
    } else {
        let parsed = parse_macro_input!(input as CrudInsertInput);
        let table = &parsed.table;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_columns(table, &parsed.cols) {
            return err;
        }

        let pool = &parsed.pool;
        let vals = &parsed.vals;
        let table_str = table.value();
        let col_strs: Vec<String> = parsed.cols.iter().map(|l| l.value()).collect();

        let col_list = col_strs.join(", ");
        let n = col_strs.len();
        let d = dialect();
        let ph: Vec<String> = (1..=n).map(|i| d.ph(i)).collect();
        let sql = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                table_str,
                col_list,
                ph.join(", ")
            ),
            table.span(),
        );
        // E0716 fix: pre-bind values even for single-branch (consistency + future-proofing)
        let val_idents: Vec<syn::Ident> = (0..vals.len())
            .map(|i| syn::Ident::new(&format!("__vi_{}", i), proc_macro2::Span::call_site()))
            .collect();
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                sqlx::query!(#sql, #(#val_idents),*).execute(#pool).await
            }
        };
        TokenStream::from(expanded)
    }
}

// ── tenant_scalar! / crud_scalar! ────────────────────────────────────────

pub fn tenant_scalar(input: TokenStream) -> TokenStream {
    expand_scalar(input, true)
}

pub fn crud_scalar(input: TokenStream) -> TokenStream {
    expand_scalar(input, false)
}

/// Expand a scalar query macro (`query_scalar`).
///
/// Uses runtime `sqlx::query_scalar::<_, Type>()` because the SQL string is
/// caller-provided (not a literal we control). Tenant variant conditionally binds `tid`.
fn expand_scalar(input: TokenStream, with_tenant: bool) -> TokenStream {
    let input = parse_macro_input!(input as ScalarInput);
    let pool = &input.pool;
    let ty = &input.ty;
    let sql = &input.sql;
    let vals = &input.vals;
    let method = &input.method;

    if with_tenant {
        let tid = &input.tid;
        let expanded = quote! {
            {
                let mut _q = sqlx::query_scalar::<_, #ty>(#sql)#(.bind(#vals))*;
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.#method(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            sqlx::query_scalar::<_, #ty>(#sql)#(.bind(#vals))*.#method(#pool).await
        };
        TokenStream::from(expanded)
    }
}

// ── tenant_query! / crud_query! ──────────────────────────────────────────

pub fn tenant_query(input: TokenStream) -> TokenStream {
    expand_query(input, true)
}

pub fn crud_query(input: TokenStream) -> TokenStream {
    expand_query(input, false)
}

/// Expand a query_as macro.
///
/// Uses runtime `sqlx::query_as::<_, Type>()` because the SQL string is caller-provided.
/// Tenant variant conditionally binds `tid`.
fn expand_query(input: TokenStream, with_tenant: bool) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let pool = &input.pool;
    let ty = &input.ty;
    let sql = &input.sql;
    let vals = &input.vals;
    let method = &input.method;

    if with_tenant {
        let tid = &input.tid;
        let expanded = quote! {
            {
                let mut _q = sqlx::query_as::<_, #ty>(#sql)#(.bind(#vals))*;
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.#method(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            sqlx::query_as::<_, #ty>(#sql)#(.bind(#vals))*.#method(#pool).await
        };
        TokenStream::from(expanded)
    }
}

/// Get the explicit column list for a table (replaces `SELECT *`).
///
/// Column names are read from the compile-time schema. Used by find/list macros.
fn get_select_columns(table: &syn::LitStr) -> String {
    get_schema().column_names(&table.value()).join(", ")
}

// ── tenant_find! / crud_find! ─────────────────────────────────────────

pub fn tenant_find(input: TokenStream) -> TokenStream {
    expand_find(input, true, FindMethod::FetchOptional)
}

pub fn tenant_find_one(input: TokenStream) -> TokenStream {
    expand_find(input, true, FindMethod::FetchOne)
}

pub fn tenant_find_all(input: TokenStream) -> TokenStream {
    expand_find(input, true, FindMethod::FetchAll)
}

pub fn crud_find(input: TokenStream) -> TokenStream {
    expand_find(input, false, FindMethod::FetchOptional)
}

pub fn crud_find_one(input: TokenStream) -> TokenStream {
    expand_find(input, false, FindMethod::FetchOne)
}

pub fn crud_find_all(input: TokenStream) -> TokenStream {
    expand_find(input, false, FindMethod::FetchAll)
}

/// Which sqlx fetch method to use.
#[allow(clippy::enum_variant_names)]
enum FindMethod {
    FetchOptional,
    FetchOne,
    FetchAll,
}

/// Expand a find-by-column macro.
///
/// Generates `SELECT {all_columns} FROM table WHERE col = ?` with explicit column list
/// from the schema (no `SELECT *`).
///
/// - **tenant** variant: runtime SQL via `format!()` + `sqlx::query_as()` because the
///   WHERE clause depends on whether `tenant_id` is Some/None at runtime.
/// - **crud** variant: runtime `sqlx::query_as()` with a static SQL literal.
fn expand_find(input: TokenStream, with_tenant: bool, method: FindMethod) -> TokenStream {
    if with_tenant {
        let parsed = parse_macro_input!(input as TenantFindInput);
        let table = &parsed.table;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_column(table, &parsed.col) {
            return err;
        }

        let pool = &parsed.pool;
        let ty = &parsed.ty;
        let val = &parsed.val;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_str = parsed.col.value();
        let cols = get_select_columns(table);

        let method_call = match &method {
            FindMethod::FetchOptional => quote! { fetch_optional(#pool).await },
            FindMethod::FetchOne => quote! { fetch_one(#pool).await },
            FindMethod::FetchAll => quote! { fetch_all(#pool).await },
        };

        // Store as string literals so the generated format!() uses them directly
        let col_lit = syn::LitStr::new(&col_str, table.span());
        let table_lit = syn::LitStr::new(&table_str, table.span());
        let cols_lit = syn::LitStr::new(&cols, table.span());

        let order_by_fragment = match &parsed.order_by {
            Some(ob) => {
                let ob_str = ob.value();
                let ob_lit = syn::LitStr::new(&format!(" ORDER BY {}", ob_str), table.span());
                quote! { #ob_lit }
            }
            None => quote! { "" },
        };

        let expanded = quote! {
            {
                let __fv = #val;
                let __ob: &str = #order_by_fragment;
                let __sql = match #tid {
                    Some(_tid) => format!("SELECT {} FROM {} WHERE {} = ? AND tenant_id = ?{}", #cols_lit, #table_lit, #col_lit, __ob),
                    None => format!("SELECT {} FROM {} WHERE {} = ?{}", #cols_lit, #table_lit, #col_lit, __ob),
                };
                let mut _q = sqlx::query_as::<_, #ty>(&__sql).bind(__fv);
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.#method_call
            }
        };
        TokenStream::from(expanded)
    } else {
        let parsed = parse_macro_input!(input as CrudFindInput);
        let table = &parsed.table;

        if let Some(err) = validate_table(table) {
            return err;
        }
        if let Some(err) = validate_column(table, &parsed.col) {
            return err;
        }

        let pool = &parsed.pool;
        let ty = &parsed.ty;
        let val = &parsed.val;
        let table_str = table.value();
        let col_str = parsed.col.value();
        let cols = get_select_columns(table);

        let method_call = match &method {
            FindMethod::FetchOptional => quote! { fetch_optional(#pool).await },
            FindMethod::FetchOne => quote! { fetch_one(#pool).await },
            FindMethod::FetchAll => quote! { fetch_all(#pool).await },
        };

        let mut sql_str = format!("SELECT {} FROM {} WHERE {} = ?", cols, table_str, col_str);
        if let Some(ref ob) = parsed.order_by {
            sql_str.push_str(&format!(" ORDER BY {}", ob.value()));
        }
        let sql = syn::LitStr::new(&sql_str, table.span());
        let expanded = quote! {
            sqlx::query_as::<_, #ty>(#sql).bind(#val).#method_call
        };
        TokenStream::from(expanded)
    }
}

// ── crud_list! ────────────────────────────────────────────────────────

pub fn crud_list(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ListInput);
    let table = &input.table;

    if let Some(err) = validate_table(table) {
        return err;
    }

    let pool = &input.pool;
    let ty = &input.ty;
    let table_str = table.value();
    let cols = get_select_columns(table);

    let order_by_str = match &input.order_by {
        Some(ob) => format!(" ORDER BY {}", ob.value()),
        None => String::new(),
    };

    if let Some(ref tid) = input.tid {
        let cols_lit = syn::LitStr::new(&cols, table.span());
        let table_lit = syn::LitStr::new(&table_str, table.span());
        let ob_lit = syn::LitStr::new(&order_by_str, table.span());
        let expanded = quote! {
            {
                let __sql = match #tid {
                    Some(_tid) => format!("SELECT {} FROM {} WHERE 1=1 AND tenant_id = ?{}", #cols_lit, #table_lit, #ob_lit),
                    None => format!("SELECT {} FROM {} WHERE 1=1{}", #cols_lit, #table_lit, #ob_lit),
                };
                let mut _q = sqlx::query_as::<_, #ty>(&__sql);
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.fetch_all(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let mut sql_str = format!("SELECT {} FROM {}", cols, table_str);
        sql_str.push_str(&order_by_str);
        let sql = syn::LitStr::new(&sql_str, table.span());
        let expanded = quote! {
            sqlx::query_as::<_, #ty>(#sql).fetch_all(#pool).await
        };
        TokenStream::from(expanded)
    }
}

// ── tenant_update! / crud_update! ────────────────────────────────────

pub fn tenant_update(input: TokenStream) -> TokenStream {
    expand_update(input, true)
}

pub fn crud_update(input: TokenStream) -> TokenStream {
    expand_update(input, false)
}

/// Expand an UPDATE macro.
///
/// Supports flexible sections: `bind:`, `raw:`, `where:`, `and:`, `tenant:`.
///
/// - **bind** — columns set to `?` placeholders with runtime-bound values.
/// - **raw** — columns set to literal SQL expressions (e.g., `datetime('now')`, `version + 1`).
/// - **where** — primary key column and value (required).
/// - **and** — extra `AND col = ?` conditions (e.g., optimistic locking on version).
/// - **tenant** — optional `Option<&str>` for `AND tenant_id = ?` filter.
///
/// Both variants use runtime `sqlx::query()` because:
/// - The tenant variant's SQL depends on `tid` at runtime.
/// - The `raw` expressions contain non-trivial SQL that's easier to construct dynamically.
///
/// Values are pre-bound to `let` variables (`__uv_*`, `__ua_*`, `__pkv`) to avoid E0716
/// temporary lifetime issues.
fn expand_update(input: TokenStream, with_tenant: bool) -> TokenStream {
    let parsed = parse_macro_input!(input as UpdateInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = validate_column(table, &parsed.pk_col) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.bind_cols) {
        return err;
    }
    for (rc, _) in &parsed.raw_pairs {
        if let Some(err) = validate_column(table, rc) {
            return err;
        }
    }
    if let Some(err) = validate_columns(table, &parsed.opt_cols) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.and_cols) {
        return err;
    }

    let pool = &parsed.pool;
    let table_str = table.value();
    let bind_cols = &parsed.bind_cols;
    let bind_vals = &parsed.bind_vals;
    let opt_cols = &parsed.opt_cols;
    let opt_vals = &parsed.opt_vals;
    let raw_pairs = &parsed.raw_pairs;
    let pk_col = &parsed.pk_col;
    let pk_val = &parsed.pk_val;
    let and_cols = &parsed.and_cols;
    let and_vals = &parsed.and_vals;

    let has_optional = !opt_cols.is_empty();

    let bind_col_strs: Vec<String> = bind_cols.iter().map(|l| l.value()).collect();

    let val_idents: Vec<syn::Ident> = (0..bind_vals.len())
        .map(|i| syn::Ident::new(&format!("__uv_{}", i), proc_macro2::Span::call_site()))
        .collect();
    let opt_idents: Vec<syn::Ident> = (0..opt_vals.len())
        .map(|i| syn::Ident::new(&format!("__ov_{}", i), proc_macro2::Span::call_site()))
        .collect();
    let and_idents: Vec<syn::Ident> = (0..and_vals.len())
        .map(|i| syn::Ident::new(&format!("__ua_{}", i), proc_macro2::Span::call_site()))
        .collect();

    // ── Dynamic path (has optional columns) ──
    //
    // Generates runtime code that builds SET clause conditionally:
    //   1. Always add bind: columns → "col = ?"
    //   2. Always add raw: columns → "col = <expr>"
    //   3. Conditionally add optional: columns → if is_some() { push "col = ?" }
    //   4. Bind values in order: bind → optional(condition) → pk → and → tenant
    //
    // Uses positional `?` placeholders (not numbered ?N) because the SET list
    // is built dynamically and the total count varies at runtime.
    if has_optional {
        let table_lit = syn::LitStr::new(&table_str, table.span());
        let pk_col_lit = syn::LitStr::new(&pk_col.value(), table.span());

        // Static SET fragments for bind: columns (always present)
        let bind_set_lits: Vec<syn::LitStr> = bind_col_strs
            .iter()
            .map(|c| syn::LitStr::new(&format!("{} = ?", c), table.span()))
            .collect();

        // Static SET fragments for raw: columns (always present)
        let raw_set_lits: Vec<syn::LitStr> = raw_pairs
            .iter()
            .map(|(rc, rv)| {
                syn::LitStr::new(&format!("{} = {}", rc.value(), rv.value()), table.span())
            })
            .collect();

        // Optional column name literals
        let opt_col_lits: Vec<syn::LitStr> = opt_cols
            .iter()
            .map(|c| syn::LitStr::new(&c.value(), c.span()))
            .collect();

        // AND column fragments
        let and_col_lits: Vec<syn::LitStr> = and_cols
            .iter()
            .map(|ac| syn::LitStr::new(&format!("AND {} = ?", ac.value()), table.span()))
            .collect();

        let tid = &parsed.tid;

        let opt_bind_idents: Vec<syn::Ident> = (0..opt_vals.len())
            .map(|i| {
                syn::Ident::new(
                    &format!("__obv_{}", i),
                    proc_macro2::Span::call_site(),
                )
            })
            .collect();

        let expanded = if with_tenant {
            quote! {
                {
                    #(let #val_idents = #bind_vals;)*
                    #(let #opt_idents = &(#opt_vals);)*
                    #(let #and_idents = #and_vals;)*
                    let __pkv = #pk_val;
                    let mut __sets: Vec<String> = Vec::new();
                    #(__sets.push(#bind_set_lits.to_string());)*
                    #(__sets.push(#raw_set_lits.to_string());)*
                    #(
                        if #opt_idents.is_some() {
                            __sets.push(format!("{} = ?", #opt_col_lits));
                        }
                    )*
                    let mut __and_sql = String::new();
                    #(__and_sql.push_str(#and_col_lits);)*
                    let _sql = match #tid {
                        Some(_tid) => format!(
                            "UPDATE {} SET {} WHERE {} = ?{} AND tenant_id = ?",
                            #table_lit, __sets.join(", "), #pk_col_lit, __and_sql
                        ),
                        None => format!(
                            "UPDATE {} SET {} WHERE {} = ?{}",
                            #table_lit, __sets.join(", "), #pk_col_lit, __and_sql
                        ),
                    };
                    let mut _q = sqlx::query(&_sql);
                    #(_q = _q.bind(#val_idents);)*
                    #(
                        if let Some(#opt_bind_idents) = #opt_idents {
                            _q = _q.bind(#opt_bind_idents);
                        }
                    )*
                    _q = _q.bind(__pkv);
                    #(_q = _q.bind(#and_idents);)*
                    if let Some(_tid) = #tid {
                        _q = _q.bind(_tid);
                    }
                    _q.execute(#pool).await
                }
            }
        } else {
            quote! {
                {
                    #(let #val_idents = #bind_vals;)*
                    #(let #opt_idents = &(#opt_vals);)*
                    #(let #and_idents = #and_vals;)*
                    let __pkv = #pk_val;
                    let mut __sets: Vec<String> = Vec::new();
                    #(__sets.push(#bind_set_lits.to_string());)*
                    #(__sets.push(#raw_set_lits.to_string());)*
                    #(
                        if #opt_idents.is_some() {
                            __sets.push(format!("{} = ?", #opt_col_lits));
                        }
                    )*
                    let mut __and_sql = String::new();
                    #(__and_sql.push_str(#and_col_lits);)*
                    let _sql = format!(
                        "UPDATE {} SET {} WHERE {} = ?{}",
                        #table_lit, __sets.join(", "), #pk_col_lit, __and_sql
                    );
                    let mut _q = sqlx::query(&_sql);
                    #(_q = _q.bind(#val_idents);)*
                    #(
                        if let Some(#opt_bind_idents) = #opt_idents {
                            _q = _q.bind(#opt_bind_idents);
                        }
                    )*
                    _q = _q.bind(__pkv);
                    #(_q = _q.bind(#and_idents);)*
                    _q.execute(#pool).await
                }
            }
        };
        return TokenStream::from(expanded);
    }

    // ── Static path (no optional columns — original logic) ──
    let d = dialect();
    let mut ph_idx = 1usize;
    let mut set_parts: Vec<String> = Vec::new();
    for _ in &bind_col_strs {
        set_parts.push(d.ph(ph_idx));
        ph_idx += 1;
    }
    let raw_set: Vec<String> = raw_pairs
        .iter()
        .map(|(rc, rv)| format!("{} = {}", rc.value(), rv.value()))
        .collect();

    let pk_ph = d.ph(ph_idx);
    ph_idx += 1;

    let and_parts: Vec<String> = and_cols
        .iter()
        .map(|ac| {
            let ph = d.ph(ph_idx);
            ph_idx += 1;
            format!("AND {} = {}", ac.value(), ph)
        })
        .collect();

    let set_static: Vec<String> = bind_col_strs
        .iter()
        .zip(set_parts.iter())
        .map(|(c, p)| format!("{} = {}", c, p))
        .chain(raw_set)
        .collect();
    let set_str = set_static.join(", ");
    let and_str = and_parts.join("");

    if with_tenant {
        let tid = &parsed.tid;
        let tenant_ph = d.ph(ph_idx);

        // Store SQL fragments as literals for the generated format!() call
        let table_lit = syn::LitStr::new(&table_str, table.span());
        let set_lit = syn::LitStr::new(&set_str, table.span());
        let pk_col_lit = syn::LitStr::new(&pk_col.value(), table.span());
        let pk_ph_lit = syn::LitStr::new(&pk_ph, table.span());
        let and_lit = syn::LitStr::new(&and_str, table.span());
        let tenant_ph_lit = syn::LitStr::new(&tenant_ph, table.span());

        // E0716 fix: pre-bind all values to named locals
        let val_idents: Vec<syn::Ident> = (0..bind_vals.len())
            .map(|i| syn::Ident::new(&format!("__uv_{}", i), proc_macro2::Span::call_site()))
            .collect();
        let and_idents: Vec<syn::Ident> = (0..and_vals.len())
            .map(|i| syn::Ident::new(&format!("__ua_{}", i), proc_macro2::Span::call_site()))
            .collect();

        let expanded = quote! {
            {
                #(let #val_idents = #bind_vals;)*
                #(let #and_idents = #and_vals;)*
                let __pkv = #pk_val;
                let _sql = match #tid {
                    Some(_tid) => format!(
                        "UPDATE {} SET {} WHERE {} = {}{} AND tenant_id = {}",
                        #table_lit, #set_lit, #pk_col_lit, #pk_ph_lit, #and_lit, #tenant_ph_lit
                    ),
                    None => format!(
                        "UPDATE {} SET {} WHERE {} = {}{}",
                        #table_lit, #set_lit, #pk_col_lit, #pk_ph_lit, #and_lit
                    ),
                };
                let mut _q = sqlx::query(&_sql)#(.bind(#val_idents))*;
                _q = _q.bind(__pkv);
                #(_q = _q.bind(#and_idents);)*
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.execute(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let sql = syn::LitStr::new(
            &format!(
                "UPDATE {} SET {} WHERE {} = {}{}",
                table_str,
                set_str,
                pk_col.value(),
                pk_ph,
                and_str
            ),
            table.span(),
        );
        // E0716 fix: pre-bind all values to named locals
        let val_idents: Vec<syn::Ident> = (0..bind_vals.len())
            .map(|i| syn::Ident::new(&format!("__uv_{}", i), proc_macro2::Span::call_site()))
            .collect();
        let and_idents: Vec<syn::Ident> = (0..and_vals.len())
            .map(|i| syn::Ident::new(&format!("__ua_{}", i), proc_macro2::Span::call_site()))
            .collect();

        let expanded = quote! {
            {
                #(let #val_idents = #bind_vals;)*
                #(let #and_idents = #and_vals;)*
                let __pkv = #pk_val;
                sqlx::query(#sql)#(.bind(#val_idents))*.bind(__pkv)#(.bind(#and_idents))*.execute(#pool).await
            }
        };
        TokenStream::from(expanded)
    }
}

// ── tenant_query_paged! ──────────────────────────────────────────────

pub fn tenant_query_paged(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as QueryPagedInput);
    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let data_sql = &parsed.data_sql;
    let count_sql = &parsed.count_sql;
    let binds = &parsed.binds;
    let tid = &parsed.tid;
    let page = &parsed.page;
    let page_size = &parsed.page_size;
    let where_cols = &parsed.where_cols;
    let where_vals = &parsed.where_vals;

    let bind_idents: Vec<syn::Ident> = (0..binds.len())
        .map(|i| syn::Ident::new(&format!("__pb_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let _bind_count = binds.len() + 1;

    let where_bind_idents: Vec<syn::Ident> = (0..where_vals.len())
        .map(|i| syn::Ident::new(&format!("__wp_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let where_col_lits: Vec<syn::LitStr> = where_cols
        .iter()
        .map(|c| syn::LitStr::new(&format!(" AND {} = ?", c.value()), c.span()))
        .collect();

    let expanded = quote! {
        {
            let __page_size = #page_size;
            let __offset = (#page - 1).max(0) * __page_size;
            #(let #bind_idents = #binds;)*
            #(let #where_bind_idents = #where_vals;)*
            let mut __where_sql = String::new();
            #(
                if #where_bind_idents.is_some() {
                    __where_sql.push_str(#where_col_lits);
                }
            )*
            let __tenant_sql = match #tid {
                Some(_tid) => " AND tenant_id = ?".to_string(),
                None => String::new(),
            };
            let __data_sql_raw = #data_sql.replace("{tenant}", &__tenant_sql);
            let mut __data_sql = if let Some(__pos) = __data_sql_raw.find("ORDER BY") {
                let mut __s = String::with_capacity(__data_sql_raw.len() + __where_sql.len() + 20);
                __s.push_str(&__data_sql_raw[..__pos]);
                __s.push_str(&__where_sql);
                __s.push_str(&__data_sql_raw[__pos..]);
                __s
            } else {
                let mut __s = __data_sql_raw;
                __s.push_str(&__where_sql);
                __s
            };
            __data_sql.push_str(" LIMIT ? OFFSET ?");
            let mut __count_sql = #count_sql.replace("{tenant}", &__tenant_sql);
            __count_sql.push_str(&__where_sql);
            let mut __dq = sqlx::query_as::<_, #ty>(&__data_sql)#(.bind(#bind_idents))*;
            if let Some(_tid) = #tid {
                __dq = __dq.bind(_tid);
            }
            #(
                if let Some(ref __wv) = #where_bind_idents {
                    __dq = __dq.bind(__wv);
                }
            )*
            __dq = __dq.bind(__page_size).bind(__offset);
            let __data = __dq.fetch_all(#pool).await?;
            let mut __cq = sqlx::query_scalar::<_, i64>(&__count_sql)#(.bind(#bind_idents))*;
            if let Some(_tid) = #tid {
                __cq = __cq.bind(_tid);
            }
            #(
                if let Some(ref __wv) = #where_bind_idents {
                    __cq = __cq.bind(__wv);
                }
            )*
            let __total = __cq.fetch_one(#pool).await?;
            (__data, __total)
        }
    };
    TokenStream::from(expanded)
}

// ── check_schema! ────────────────────────────────────────────────────

/// Expand `check_schema!("table", "col1", "col2", ...)`.
///
/// Validates that the table and all named columns exist in the schema.
/// On success, expands to nothing (empty token stream).
/// On failure, emits a compile error.
pub fn check_schema(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as CheckSchemaInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    for col in &parsed.cols {
        if let Some(err) = validate_column(table, col) {
            return err;
        }
    }

    TokenStream::new()
}

// ══════════════════════════════════════════════════════════════════════════
// Input parsing structs
// ══════════════════════════════════════════════════════════════════════════
//
// Each macro has a corresponding input struct that implements `syn::parse::Parse`.
// The `Parse` trait defines how to consume tokens from the macro's input stream.
//
// Naming convention:
//   Tenant*Input — has a `tid` (tenant_id) field
//   Crud*Input   — no `tid` field

// ── Delete inputs ──

/// `tenant_delete!(pool, "table", "col" => val, tenant_id)`
struct TenantDeleteInput {
    pool: syn::Expr,
    table: syn::LitStr,
    col: syn::LitStr,
    val: syn::Expr,
    tid: syn::Expr,
}

impl syn::parse::Parse for TenantDeleteInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let col: syn::LitStr = input.parse()?;
        let _: syn::Token![=>] = input.parse()?;
        let val: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let tid: syn::Expr = input.parse()?;
        Ok(Self {
            pool,
            table,
            col,
            val,
            tid,
        })
    }
}

/// `crud_delete!(pool, "table", "col" => val)`
struct CrudDeleteInput {
    pool: syn::Expr,
    table: syn::LitStr,
    col: syn::LitStr,
    val: syn::Expr,
}

impl syn::parse::Parse for CrudDeleteInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let col: syn::LitStr = input.parse()?;
        let _: syn::Token![=>] = input.parse()?;
        let val: syn::Expr = input.parse()?;
        Ok(Self {
            pool,
            table,
            col,
            val,
        })
    }
}

// ── Insert inputs ──

/// Shared parser for the common insert body: `pool, "table", ["col" => val, ...]`
fn parse_insert_body(
    input: syn::parse::ParseStream,
) -> syn::Result<(syn::Expr, syn::LitStr, Vec<syn::LitStr>, Vec<syn::Expr>)> {
    let pool: syn::Expr = input.parse()?;
    let _: syn::Token![,] = input.parse()?;
    let table: syn::LitStr = input.parse()?;
    let _: syn::Token![,] = input.parse()?;
    let content;
    syn::bracketed!(content in input);
    let mut cols: Vec<syn::LitStr> = Vec::new();
    let mut vals: Vec<syn::Expr> = Vec::new();
    while !content.is_empty() {
        let col: syn::LitStr = content.parse()?;
        let _: syn::Token![=>] = content.parse()?;
        let val: syn::Expr = content.parse()?;
        cols.push(col);
        vals.push(val);
        let _ = content.parse::<syn::Token![,]>(); // optional trailing comma
    }
    Ok((pool, table, cols, vals))
}

/// `tenant_insert!(pool, "table", ["col" => val, ...], tenant_id)`
struct TenantInsertInput {
    pool: syn::Expr,
    table: syn::LitStr,
    cols: Vec<syn::LitStr>,
    vals: Vec<syn::Expr>,
    tid: syn::Expr,
}

impl syn::parse::Parse for TenantInsertInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let (pool, table, cols, vals) = parse_insert_body(input)?;
        let _: syn::Token![,] = input.parse()?;
        let tid: syn::Expr = input.parse()?;
        Ok(Self {
            pool,
            table,
            cols,
            vals,
            tid,
        })
    }
}

/// `crud_insert!(pool, "table", ["col" => val, ...])`
struct CrudInsertInput {
    pool: syn::Expr,
    table: syn::LitStr,
    cols: Vec<syn::LitStr>,
    vals: Vec<syn::Expr>,
}

impl syn::parse::Parse for CrudInsertInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let (pool, table, cols, vals) = parse_insert_body(input)?;
        Ok(Self {
            pool,
            table,
            cols,
            vals,
        })
    }
}

// ── Scalar / Query inputs ──

/// `tenant_scalar!(pool, Type, sql, [val1, val2], tenant_id, method)`
///
/// - `sql`: a string expression (not necessarily a literal — can be a `format!()` result).
/// - `method`: one of `fetch_one`, `fetch_optional`, etc.
struct ScalarInput {
    pool: syn::Expr,
    ty: syn::Type,
    sql: syn::Expr,
    vals: Vec<syn::Expr>,
    tid: syn::Expr,
    method: syn::Ident,
}

impl syn::parse::Parse for ScalarInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let sql: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let content;
        syn::bracketed!(content in input);
        let mut vals: Vec<syn::Expr> = Vec::new();
        while !content.is_empty() {
            vals.push(content.parse()?);
            let _ = content.parse::<syn::Token![,]>();
        }
        let _: syn::Token![,] = input.parse()?;
        let tid: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let method: syn::Ident = input.parse()?;
        Ok(Self {
            pool,
            ty,
            sql,
            vals,
            tid,
            method,
        })
    }
}

/// `tenant_query!(pool, Type, sql, [val1, val2], tenant_id, method)`
struct QueryInput {
    pool: syn::Expr,
    ty: syn::Type,
    sql: syn::Expr,
    vals: Vec<syn::Expr>,
    tid: syn::Expr,
    method: syn::Ident,
}

impl syn::parse::Parse for QueryInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let sql: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let content;
        syn::bracketed!(content in input);
        let mut vals: Vec<syn::Expr> = Vec::new();
        while !content.is_empty() {
            vals.push(content.parse()?);
            let _ = content.parse::<syn::Token![,]>();
        }
        let _: syn::Token![,] = input.parse()?;
        let tid: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let method: syn::Ident = input.parse()?;
        Ok(Self {
            pool,
            ty,
            sql,
            vals,
            tid,
            method,
        })
    }
}

// ── Find inputs ──

/// `tenant_find!(pool, "table", Type, "col" => val, tenant_id [, order_by: "expr"])`
struct TenantFindInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    col: syn::LitStr,
    val: syn::Expr,
    tid: syn::Expr,
    order_by: Option<syn::LitStr>,
}

impl syn::parse::Parse for TenantFindInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let col: syn::LitStr = input.parse()?;
        let _: syn::Token![=>] = input.parse()?;
        let val: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let tid: syn::Expr = input.parse()?;

        let mut order_by = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "order_by" {
                order_by = Some(input.parse()?);
            }
        }

        Ok(Self {
            pool,
            table,
            ty,
            col,
            val,
            tid,
            order_by,
        })
    }
}

/// `crud_find!(pool, "table", Type, "col" => val [, order_by: "expr"])`
struct CrudFindInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    col: syn::LitStr,
    val: syn::Expr,
    order_by: Option<syn::LitStr>,
}

impl syn::parse::Parse for CrudFindInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let col: syn::LitStr = input.parse()?;
        let _: syn::Token![=>] = input.parse()?;
        let val: syn::Expr = input.parse()?;

        let mut order_by = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "order_by" {
                order_by = Some(input.parse()?);
            }
        }

        Ok(Self {
            pool,
            table,
            ty,
            col,
            val,
            order_by,
        })
    }
}

// ── List input ──

/// `crud_list!(pool, "table", Type [, order_by: "expr", tenant: tid])`
struct ListInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    order_by: Option<syn::LitStr>,
    tid: Option<syn::Expr>,
}

impl syn::parse::Parse for ListInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;

        let mut order_by = None;
        let mut tid = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            match section.to_string().as_str() {
                "order_by" => {
                    order_by = Some(input.parse()?);
                }
                "tenant" => {
                    tid = Some(input.parse()?);
                }
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section: {}", other),
                    ));
                }
            }
        }

        Ok(Self {
            pool,
            table,
            ty,
            order_by,
            tid,
        })
    }
}

// ── Check schema input ──

/// `check_schema!("table", "col1", "col2", ...)`
struct CheckSchemaInput {
    table: syn::LitStr,
    cols: Vec<syn::LitStr>,
}

impl syn::parse::Parse for CheckSchemaInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let table: syn::LitStr = input.parse()?;
        let mut cols: Vec<syn::LitStr> = Vec::new();
        while input.parse::<syn::Token![,]>().is_ok() {
            cols.push(input.parse()?);
        }
        Ok(Self { table, cols })
    }
}

// ── Update input ──

/// Flexible update input with named sections.
///
/// Syntax:
/// ```ignore
/// tenant_update!(pool, "table",
///     bind:  ["col1" => val1, "col2" => val2],
///     raw:   ["col3" => "datetime('now')"],
///     where: "pk_col" => pk_val,
///     and:   ["version" => expected_version],
///     tenant: tenant_id
/// )
/// ```
///
/// Sections can appear in any order. `bind:`, `raw:`, `and:`, `tenant:` are optional.
/// `where:` is required.
struct UpdateInput {
    pool: syn::Expr,
    table: syn::LitStr,
    bind_cols: Vec<syn::LitStr>,
    bind_vals: Vec<syn::Expr>,
    opt_cols: Vec<syn::LitStr>,
    opt_vals: Vec<syn::Expr>,
    raw_pairs: Vec<(syn::LitStr, syn::LitStr)>,
    pk_col: syn::LitStr,
    pk_val: syn::Expr,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    tid: Option<syn::Expr>,
}

/// Parse `["col" => expr, ...]` bracket content into separate col/val lists.
fn parse_kv_bracket(
    content: &syn::parse::ParseBuffer,
) -> syn::Result<(Vec<syn::LitStr>, Vec<syn::Expr>)> {
    let mut cols = Vec::new();
    let mut vals = Vec::new();
    while !content.is_empty() {
        let col: syn::LitStr = content.parse()?;
        let _: syn::Token![=>] = content.parse()?;
        let val: syn::Expr = content.parse()?;
        cols.push(col);
        vals.push(val);
        let _ = content.parse::<syn::Token![,]>(); // optional trailing comma
    }
    Ok((cols, vals))
}

/// Parse `["col" => "raw_sql", ...]` bracket content into (col, raw_value) pairs.
/// Both sides are string literals — the right side is a raw SQL expression.
fn parse_raw_bracket(
    content: &syn::parse::ParseBuffer,
) -> syn::Result<Vec<(syn::LitStr, syn::LitStr)>> {
    let mut pairs = Vec::new();
    while !content.is_empty() {
        let col: syn::LitStr = content.parse()?;
        let _: syn::Token![=>] = content.parse()?;
        let val: syn::LitStr = content.parse()?;
        pairs.push((col, val));
        let _ = content.parse::<syn::Token![,]>();
    }
    Ok(pairs)
}

impl syn::parse::Parse for UpdateInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;

        let mut bind_cols = Vec::new();
        let mut bind_vals = Vec::new();
        let mut opt_cols = Vec::new();
        let mut opt_vals = Vec::new();
        let mut raw_pairs: Vec<(syn::LitStr, syn::LitStr)> = Vec::new();
        let mut pk_col = None;
        let mut pk_val = None;
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut tid = None;

        // Parse sections in any order, each identified by a keyword label followed by `:`
        while !input.is_empty() {
            // Use `parse_any` because `where` is a Rust keyword and won't parse as a normal Ident
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            match section.to_string().as_str() {
                "bind" => {
                    let content;
                    syn::bracketed!(content in input);
                    let (c, v) = parse_kv_bracket(&content)?;
                    bind_cols = c;
                    bind_vals = v;
                }
                "optional" => {
                    let content;
                    syn::bracketed!(content in input);
                    let (c, v) = parse_kv_bracket(&content)?;
                    opt_cols = c;
                    opt_vals = v;
                }
                "raw" => {
                    let content;
                    syn::bracketed!(content in input);
                    raw_pairs = parse_raw_bracket(&content)?;
                }
                "where" => {
                    let col: syn::LitStr = input.parse()?;
                    let _: syn::Token![=>] = input.parse()?;
                    let val: syn::Expr = input.parse()?;
                    pk_col = Some(col);
                    pk_val = Some(val);
                }
                "and" => {
                    let content;
                    syn::bracketed!(content in input);
                    let (c, v) = parse_kv_bracket(&content)?;
                    and_cols = c;
                    and_vals = v;
                }
                "tenant" => {
                    tid = Some(input.parse()?);
                }
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section: {}", other),
                    ));
                }
            }
            let _ = input.parse::<syn::Token![,]>(); // optional trailing comma between sections
        }

        let pk_col =
            pk_col.ok_or_else(|| syn::Error::new(table.span(), "missing `where:` section"))?;
        let pk_val =
            pk_val.ok_or_else(|| syn::Error::new(table.span(), "missing `where:` value"))?;

        Ok(Self {
            pool,
            table,
            bind_cols,
            bind_vals,
            opt_cols,
            opt_vals,
            raw_pairs,
            pk_col,
            pk_val,
            and_cols,
            and_vals,
            tid,
        })
    }
}

// ── QueryPaged input ──

/// `tenant_query_paged!(pool, Type, data_sql: "...", count_sql: "...", binds: [...], tenant: tid, page: page, page_size: page_size)`
struct QueryPagedInput {
    pool: syn::Expr,
    ty: syn::Type,
    data_sql: syn::LitStr,
    count_sql: syn::LitStr,
    binds: Vec<syn::Expr>,
    tid: syn::Expr,
    page: syn::Expr,
    page_size: syn::Expr,
    where_cols: Vec<syn::LitStr>,
    where_vals: Vec<syn::Expr>,
}

impl syn::parse::Parse for QueryPagedInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;
        let _: syn::Token![,] = input.parse()?;

        let mut data_sql = None;
        let mut count_sql = None;
        let mut binds = Vec::new();
        let mut tid = None;
        let mut page = None;
        let mut page_size = None;
        let mut where_cols = Vec::new();
        let mut where_vals = Vec::new();

        while !input.is_empty() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            match section.to_string().as_str() {
                "data_sql" => {
                    data_sql = Some(input.parse()?);
                }
                "count_sql" => {
                    count_sql = Some(input.parse()?);
                }
                "binds" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        binds.push(content.parse()?);
                        let _ = content.parse::<syn::Token![,]>();
                    }
                }
                "tenant" => {
                    tid = Some(input.parse()?);
                }
                "page" => {
                    page = Some(input.parse()?);
                }
                "page_size" => {
                    page_size = Some(input.parse()?);
                }
                "where" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        let col: syn::LitStr = content.parse()?;
                        let _: syn::Token![=>] = content.parse()?;
                        let val: syn::Expr = content.parse()?;
                        where_cols.push(col);
                        where_vals.push(val);
                        let _ = content.parse::<syn::Token![,]>();
                    }
                }
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section: {}", other),
                    ));
                }
            }
            let _ = input.parse::<syn::Token![,]>();
        }

        let data_sql =
            data_sql.ok_or_else(|| syn::Error::new(ty.span(), "missing `data_sql:` section"))?;
        let count_sql =
            count_sql.ok_or_else(|| syn::Error::new(ty.span(), "missing `count_sql:` section"))?;
        let tid = tid.ok_or_else(|| syn::Error::new(ty.span(), "missing `tenant:` section"))?;
        let page = page.ok_or_else(|| syn::Error::new(ty.span(), "missing `page:` section"))?;
        let page_size =
            page_size.ok_or_else(|| syn::Error::new(ty.span(), "missing `page_size:` section"))?;

        Ok(Self {
            pool,
            ty,
            data_sql,
            count_sql,
            binds,
            tid,
            page,
            page_size,
            where_cols,
            where_vals,
        })
    }
}
