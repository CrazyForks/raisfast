//! CRUD proc-macro implementations.
//!
//! This module implements all the `tenant_*!` and `crud_*!` bang macros, plus
//! `check_schema!`. Each macro follows the same pattern:
//!
//! 1. **Parse** the input tokens into a structured input struct (e.g., `DeleteInput`).
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
    validate_column_inner(&table.value(), col).map(Into::into)
}

pub fn validate_column_inner(
    table_str: &str,
    col: &syn::LitStr,
) -> Option<proc_macro2::TokenStream> {
    let col_str = col.value();
    if let Some(ts) = get_schema().tables.get(table_str)
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
        .to_compile_error(),
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

// ── Extra conditions (and_null / and_gt / and_lt / and_gte / and_lte) ────
//
// Shared across all macros that support `and:`. These extra parameters allow
// IS NULL checks and comparison operators alongside the existing `and:` (equality).

fn emit_runtime_binds(
    binds: &[crate::where_dsl::BindKind],
) -> (Vec<proc_macro2::TokenStream>, Vec<proc_macro2::TokenStream>) {
    let local_stmts: Vec<_> = binds
        .iter()
        .enumerate()
        .map(|(i, bk)| match bk {
            crate::where_dsl::BindKind::Static(expr) => {
                let ident = syn::Ident::new(&format!("__wb_{}", i), proc_macro2::Span::call_site());
                quote! { let #ident = #expr; }
            }
            crate::where_dsl::BindKind::InLoop(expr) => {
                let ident = syn::Ident::new(&format!("__in_{}", i), proc_macro2::Span::call_site());
                quote! { let #ident = #expr; }
            }
        })
        .collect();

    let bind_stmts: Vec<_> = binds
        .iter()
        .enumerate()
        .map(|(i, bk)| match bk {
            crate::where_dsl::BindKind::Static(_) => {
                let ident = syn::Ident::new(&format!("__wb_{}", i), proc_macro2::Span::call_site());
                quote! { __q = __q.bind(#ident); }
            }
            crate::where_dsl::BindKind::InLoop(_) => {
                let ident = syn::Ident::new(&format!("__in_{}", i), proc_macro2::Span::call_site());
                quote! { for __iv in #ident { __q = __q.bind(__iv.clone()); } }
            }
        })
        .collect();

    (local_stmts, bind_stmts)
}

fn emit_tenant_code(
    tid: &Option<syn::Expr>,
    d: Dialect,
    tenant_alias: Option<&syn::LitStr>,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
    let ph_prefix_lit = syn::LitStr::new(
        match d {
            Dialect::Postgres => "$",
            _ => "?",
        },
        proc_macro2::Span::call_site(),
    );
    let tenant_col = match tenant_alias {
        Some(a) => format!("{}.{}", d.qi(&a.value()), d.qi("tenant_id")),
        None => d.qi("tenant_id"),
    };
    let tenant_col_lit = syn::LitStr::new(&tenant_col, proc_macro2::Span::call_site());

    if tid.is_some() {
        let tid_expr = tid.as_ref().unwrap();
        let tenant_sql = quote! {
            let (__tenant_sql, __tid_val) = match #tid_expr {
                Some(_tid) => {
                    let __tph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    (format!(" AND {} = {}", #tenant_col_lit, __tph), Some(_tid))
                },
                None => (String::new(), None),
            };
        };
        let tenant_bind = quote! {
            if let Some(_tid) = __tid_val {
                __q = __q.bind(_tid);
            }
        };
        (tenant_sql, tenant_bind)
    } else {
        (quote! { let __tenant_sql = String::new(); }, quote! {})
    }
}

// ── crud_delete! ────────────────────────────────────────────────────────

pub fn crud_delete(input: TokenStream) -> TokenStream {
    expand_delete(input)
}

fn expand_delete(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as WhereOnlyInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let d = dialect();
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());

    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (local_stmts, bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let sql_code = wr.sql_code;

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #sql_code
            #tenant_sql
            let __sql = format!("DELETE FROM {} WHERE {}{}", #table_lit, __where_sql, __tenant_sql);
            let mut __q = sqlx::query::<crate::db::pool::Db>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            __q.execute(#pool).await
        }
    };
    TokenStream::from(expanded)
}

// ── crud_insert! ────────────────────────────────────────────────────────

pub fn crud_insert(input: TokenStream) -> TokenStream {
    expand_insert(input)
}

fn expand_insert(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as InsertInput);
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

    let d = dialect();
    let qi_table = d.qi(&table_str);
    let col_list = d.qi_list(&col_strs);
    let n = col_strs.len();

    let val_idents: Vec<syn::Ident> = (0..vals.len())
        .map(|i| syn::Ident::new(&format!("__vi_{}", i), proc_macro2::Span::call_site()))
        .collect();

    if parsed.tid.is_some() {
        let qi_tid = d.qi("tenant_id");
        let col_list_with = format!("{}, {}", col_list, qi_tid);
        let ph_with: Vec<String> = (1..=n + 1).map(|i| d.ph(i)).collect();
        let ph: Vec<String> = (1..=n).map(|i| d.ph(i)).collect();
        let sql_with = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                qi_table,
                col_list_with,
                ph_with.join(", ")
            ),
            table.span(),
        );
        let sql_without = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                qi_table,
                col_list,
                ph.join(", ")
            ),
            table.span(),
        );
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                let __r: sqlx::Result<crate::db::pool::DbQueryResult> = match #tid {
                    Some(_tid) => sqlx::query!(#sql_with, #(#val_idents),*, _tid).execute(#pool).await,
                    None => sqlx::query!(#sql_without, #(#val_idents),*).execute(#pool).await,
                };
                __r
            }
        };
        TokenStream::from(expanded)
    } else {
        let ph: Vec<String> = (1..=n).map(|i| d.ph(i)).collect();
        let sql = syn::LitStr::new(
            &format!(
                "INSERT INTO {} ({}) VALUES ({})",
                qi_table,
                col_list,
                ph.join(", ")
            ),
            table.span(),
        );
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                let __r: sqlx::Result<crate::db::pool::DbQueryResult> = sqlx::query!(#sql, #(#val_idents),*).execute(#pool).await;
                __r
            }
        };
        TokenStream::from(expanded)
    }
}

// ── crud_scalar! ────────────────────────────────────────

pub fn crud_scalar(input: TokenStream) -> TokenStream {
    expand_scalar(input)
}

fn expand_scalar(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ScalarInput);
    let pool = &input.pool;
    let ty = &input.ty;
    let sql = &input.sql;
    let vals = &input.vals;
    let method = &input.method;

    if input.tid.is_some() {
        let tid = &input.tid;
        let expanded = quote! {
            {
                let mut _q = sqlx::query_scalar::<crate::db::pool::Db, #ty>(#sql)#(.bind(#vals))*;
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.#method(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            sqlx::query_scalar::<crate::db::pool::Db, #ty>(#sql)#(.bind(#vals))*.#method(#pool).await
        };
        TokenStream::from(expanded)
    }
}

// ── crud_select! ─────────────────────────────────────

pub fn crud_select(input: TokenStream) -> TokenStream {
    expand_select(input)
}

fn expand_select(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as SelectInput);
    let d = dialect();
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.sel_cols) {
        return err;
    }
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .map(|c| d.qi(&c))
        .collect::<Vec<_>>()
        .join(", ");
    let sel_lit = syn::LitStr::new(&sel_str, table.span());

    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (local_stmts, bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let sql_code = wr.sql_code;

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #sql_code
            #tenant_sql
            let __sql = format!("SELECT {} FROM {} WHERE {}{}", #sel_lit, #table_lit, __where_sql, __tenant_sql);
            let mut __q = sqlx::query_as::<crate::db::pool::Db, _>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            __q.fetch_optional(#pool).await
        }
    };
    TokenStream::from(expanded)
}

// ── crud_query! ──────────────────────────────────────────

pub fn crud_query(input: TokenStream) -> TokenStream {
    expand_query(input)
}

fn expand_query(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as QueryInput);
    let pool = &input.pool;
    let ty = &input.ty;
    let sql = &input.sql;
    let vals = &input.vals;
    let method = &input.method;

    if input.tid.is_some() {
        let tid = &input.tid;
        let expanded = quote! {
            {
                let mut _q = sqlx::query_as::<crate::db::pool::Db, #ty>(#sql)#(.bind(#vals))*;
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.#method(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            sqlx::query_as::<crate::db::pool::Db, #ty>(#sql)#(.bind(#vals))*.#method(#pool).await
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

// ── crud_find! ─────────────────────────────────────────────────────────

pub fn crud_find(input: TokenStream) -> TokenStream {
    expand_find(input, FindMethod::FetchOptional)
}

pub fn crud_find_one(input: TokenStream) -> TokenStream {
    expand_find(input, FindMethod::FetchOne)
}

pub fn crud_find_all(input: TokenStream) -> TokenStream {
    expand_find(input, FindMethod::FetchAll)
}

#[allow(clippy::enum_variant_names)]
enum FindMethod {
    FetchOptional,
    FetchOne,
    FetchAll,
}

fn expand_find(input: TokenStream, method: FindMethod) -> TokenStream {
    let parsed = parse_macro_input!(input as FindInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let d = dialect();
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());
    let cols = get_select_columns(table);
    let cols_lit = syn::LitStr::new(&cols, table.span());

    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (local_stmts, bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let sql_code = wr.sql_code;

    let order_fragment = match &parsed.order_by {
        Some(ob) => {
            let ob_str = format!(" ORDER BY {}", ob.value());
            let ob_lit = syn::LitStr::new(&ob_str, table.span());
            quote! { #ob_lit }
        }
        None => quote! { "" },
    };

    let method_call = match &method {
        FindMethod::FetchOptional => quote! { fetch_optional(#pool).await },
        FindMethod::FetchOne => quote! { fetch_one(#pool).await },
        FindMethod::FetchAll => quote! { fetch_all(#pool).await },
    };

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #sql_code
            #tenant_sql
            let __sql = format!("SELECT {} FROM {} WHERE {}{}{}", #cols_lit, #table_lit, __where_sql, __tenant_sql, #order_fragment);
            let mut __q = sqlx::query_as::<crate::db::pool::Db, #ty>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            __q.#method_call
        }
    };
    TokenStream::from(expanded)
}

// ── crud_count! ─────────────────────────────────────────

pub fn crud_count(input: TokenStream) -> TokenStream {
    expand_count(input)
}

fn expand_count(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as WhereOnlyInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let d = dialect();
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());

    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (local_stmts, bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let sql_code = wr.sql_code;

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #sql_code
            #tenant_sql
            let __sql = format!("SELECT COUNT(*) FROM {} WHERE {}{}", #table_lit, __where_sql, __tenant_sql);
            let mut __q = sqlx::query_scalar::<crate::db::pool::Db, i64>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            __q.fetch_one(#pool).await
        }
    };
    TokenStream::from(expanded)
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

    let d = dialect();
    if let Some(ref tid) = input.tid {
        let cols_lit = syn::LitStr::new(&cols, table.span());
        let qi_table = d.qi(&table_str);
        let table_lit = syn::LitStr::new(&qi_table, table.span());
        let ob_lit = syn::LitStr::new(&order_by_str, table.span());
        let tid_ph = d.ph(1);
        let tid_ph_lit = syn::LitStr::new(&tid_ph, table.span());
        let qi_tid = d.qi("tenant_id");
        let qi_tid_lit = syn::LitStr::new(&qi_tid, table.span());
        let expanded = quote! {
            {
                let __sql = match #tid {
                    Some(_tid) => format!("SELECT {} FROM {} WHERE 1=1 AND {} = {}{}", #cols_lit, #table_lit, #qi_tid_lit, #tid_ph_lit, #ob_lit),
                    None => format!("SELECT {} FROM {} WHERE 1=1{}", #cols_lit, #table_lit, #ob_lit),
                };
                let mut _q = sqlx::query_as::<crate::db::pool::Db, #ty>(&__sql);
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.fetch_all(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let qi_table = d.qi(&table_str);
        let mut sql_str = format!("SELECT {} FROM {}", cols, qi_table);
        sql_str.push_str(&order_by_str);
        let sql = syn::LitStr::new(&sql_str, table.span());
        let expanded = quote! {
            sqlx::query_as::<crate::db::pool::Db, #ty>(#sql).fetch_all(#pool).await
        };
        TokenStream::from(expanded)
    }
}

// ── crud_update! ────────────────────────────────────

pub fn crud_update(input: TokenStream) -> TokenStream {
    expand_update(input)
}

fn expand_update(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as UpdateInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
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
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let d = dialect();
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());
    let bind_cols = &parsed.bind_cols;
    let bind_vals = &parsed.bind_vals;
    let opt_cols = &parsed.opt_cols;
    let opt_vals = &parsed.opt_vals;
    let raw_pairs = &parsed.raw_pairs;

    let has_optional = !opt_cols.is_empty();

    let bind_col_strs: Vec<String> = bind_cols.iter().map(|l| l.value()).collect();

    let val_idents: Vec<syn::Ident> = (0..bind_vals.len())
        .map(|i| syn::Ident::new(&format!("__uv_{}", i), proc_macro2::Span::call_site()))
        .collect();
    let opt_idents: Vec<syn::Ident> = (0..opt_vals.len())
        .map(|i| syn::Ident::new(&format!("__ov_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
    let ph_prefix_lit = syn::LitStr::new(
        match d {
            Dialect::Postgres => "$",
            _ => "?",
        },
        proc_macro2::Span::call_site(),
    );

    let bind_set_lits: Vec<syn::LitStr> = bind_col_strs
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let ph = d.ph(1 + i);
            syn::LitStr::new(&format!("{} = {}", d.qi(c), ph), table.span())
        })
        .collect();

    let raw_col_names: Vec<syn::LitStr> = raw_pairs
        .iter()
        .map(|(rc, _)| syn::LitStr::new(&d.qi(&rc.value()), rc.span()))
        .collect();
    let raw_exprs: Vec<&syn::Expr> = raw_pairs.iter().map(|(_, rv)| rv).collect();

    let opt_col_lits: Vec<syn::LitStr> = opt_cols
        .iter()
        .map(|c| syn::LitStr::new(&d.qi(&c.value()), c.span()))
        .collect();

    let opt_bind_idents: Vec<syn::Ident> = (0..opt_vals.len())
        .map(|i| syn::Ident::new(&format!("__obv_{}", i), proc_macro2::Span::call_site()))
        .collect();

    // SET clause building
    let dyn_start = bind_cols.len() + 1;
    let dyn_start_lit = syn::LitInt::new(&dyn_start.to_string(), proc_macro2::Span::call_site());

    // WHERE clause via runtime codegen
    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (where_local_stmts, where_bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let where_sql_code = wr.sql_code;

    let set_code = if has_optional {
        quote! {
            let mut __sets: Vec<String> = Vec::new();
            #(__sets.push(#bind_set_lits.to_string());)*
            #(__sets.push(format!("{} = {}", #raw_col_names, #raw_exprs));)*
            let mut __ph_idx: usize = #dyn_start_lit;
            #(
                if #opt_idents.is_some() {
                    let __ph: String = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    __sets.push(format!("{} = {}", #opt_col_lits, __ph));
                }
            )*
        }
    } else {
        let static_set: String = {
            let parts: Vec<String> = bind_col_strs
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{} = {}", d.qi(c), d.ph(1 + i)))
                .collect();
            parts.join(", ")
        };
        let has_raw = !raw_pairs.is_empty();
        if has_raw {
            let sep = if static_set.is_empty() { "" } else { ", " };
            let raw_parts: Vec<String> = raw_pairs
                .iter()
                .enumerate()
                .map(|(i, (c, _))| {
                    let qi_c = d.qi(&c.value());
                    if i == 0 {
                        format!("{} = {{}}", qi_c)
                    } else {
                        format!(", {} = {{}}", qi_c)
                    }
                })
                .collect();
            let raw_fmt = format!("{}{}", sep, raw_parts.join(""));
            let raw_fmt_lit = syn::LitStr::new(&raw_fmt, table.span());
            let static_set_lit = syn::LitStr::new(&static_set, table.span());
            quote! {
                let __raw_suffix = format!(#raw_fmt_lit #(, #raw_exprs)*);
                let __set_str = format!("{}{}", #static_set_lit, __raw_suffix);
            }
        } else {
            let static_set_lit = syn::LitStr::new(&static_set, table.span());
            quote! {
                let __set_str = #static_set_lit.to_string();
            }
        }
    };

    let set_join = if has_optional {
        quote! { __sets.join(", ") }
    } else {
        quote! { __set_str }
    };

    let bind_code = if has_optional {
        quote! {
            #(let #val_idents = #bind_vals;)*
            #(let #opt_idents = &(#opt_vals);)*
        }
    } else {
        quote! {
            #(let #val_idents = #bind_vals;)*
        }
    };

    let opt_bind_code = if has_optional {
        quote! {
            #(
                if let Some(#opt_bind_idents) = #opt_idents {
                    __q = __q.bind(#opt_bind_idents);
                }
            )*
        }
    } else {
        quote! {}
    };

    let where_ph_start_stmt = if has_optional {
        quote! {} // __ph_idx is set by set_code and carries over
    } else {
        let start = 1 + bind_vals.len();
        let start_lit = syn::LitInt::new(&start.to_string(), proc_macro2::Span::call_site());
        quote! { let mut __ph_idx: usize = #start_lit; }
    };

    let expanded = quote! {
        {
            #bind_code
            #(#where_local_stmts)*
            #set_code
            let mut __where_sql = String::new();
            #where_ph_start_stmt
            #where_sql_code
            #tenant_sql
            let __sql = format!("UPDATE {} SET {} WHERE {}{}", #table_lit, #set_join, __where_sql, __tenant_sql);
            let mut __q = sqlx::query::<crate::db::pool::Db>(&__sql);
            #(__q = __q.bind(#val_idents);)*
            #opt_bind_code
            #(#where_bind_stmts)*
            #tenant_bind
            __q.execute(#pool).await
        }
    };
    TokenStream::from(expanded)
}

// ── crud_query_paged! ──────────────────────────────────────────────

pub fn crud_query_paged(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as QueryPagedInput);
    expand_query_paged_dsl(&parsed)
}

fn expand_query_paged_dsl(parsed: &QueryPagedInput) -> TokenStream {
    use crate::where_dsl::WhereCodegen;

    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let table = &parsed.table;
    let tid = &parsed.tid;
    let page = &parsed.page;
    let page_size = &parsed.page_size;

    if let Some(err) = validate_table(table) {
        return err;
    }

    let d = dialect();
    let cols = get_select_columns(table);
    let table_str = table.value();

    let where_sql: String;
    let mut dsl_bind_count: usize = 0;
    let dsl_bind_exprs: Vec<syn::Expr>;

    if let Some(ref dsl_where) = parsed.dsl_where {
        if let Some(err) = crate::where_dsl::validate_where_columns(dsl_where, &table_str) {
            return err.into();
        }
        let mut cg = WhereCodegen::new(d, 1);
        cg.generate(dsl_where);
        where_sql = format!(" WHERE {}", cg.sql);
        dsl_bind_count = cg.next_idx - 1;
        dsl_bind_exprs = cg.binds;
    } else {
        where_sql = String::new();
        dsl_bind_exprs = Vec::new();
    }

    let order_str = parsed
        .order_by
        .as_ref()
        .map(|o| format!(" ORDER BY {}", o.value()))
        .unwrap_or_default();
    let order_str_lit = syn::LitStr::new(&order_str, table.span());

    let needs_where_base = where_sql.is_empty() && (tid.is_some() || !parsed.where_cols.is_empty());
    let where_base = if needs_where_base { " WHERE 1=1" } else { "" };

    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
    let ph_prefix_lit = syn::LitStr::new(
        match d {
            Dialect::Postgres => "$",
            _ => "?",
        },
        proc_macro2::Span::call_site(),
    );

    let dsl_bind_idents: Vec<syn::Ident> = (0..dsl_bind_exprs.len())
        .map(|i| syn::Ident::new(&format!("__db_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let where_col_strs: Vec<syn::LitStr> = parsed
        .where_cols
        .iter()
        .map(|c| syn::LitStr::new(&d.qi(&c.value()), c.span()))
        .collect();
    let where_bind_idents: Vec<syn::Ident> = (0..parsed.where_vals.len())
        .map(|i| syn::Ident::new(&format!("__wp_{}", i), proc_macro2::Span::call_site()))
        .collect();
    let where_vals = &parsed.where_vals;

    let data_sql_str = format!(
        "SELECT {} FROM {}{}{}",
        cols,
        d.qi(&table_str),
        where_sql,
        where_base
    );
    let count_sql_str = format!(
        "SELECT COUNT(*) FROM {}{}{}",
        d.qi(&table_str),
        where_sql,
        where_base
    );

    let data_sql_lit = syn::LitStr::new(&data_sql_str, table.span());
    let count_sql_lit = syn::LitStr::new(&count_sql_str, table.span());

    let (tenant_sql_block, tenant_bind_data, _tenant_bind_count) = if let Some(tid_expr) = tid {
        let base_after_dsl = dsl_bind_count + 1;
        let base_lit =
            syn::LitInt::new(&base_after_dsl.to_string(), proc_macro2::Span::call_site());
        let qi_tid_lit = syn::LitStr::new(&d.qi("tenant_id"), proc_macro2::Span::call_site());
        let block: proc_macro2::TokenStream = quote! {
            let mut __ph_idx: usize = #base_lit;
            let __tenant_val = #tid_expr;
            let __tenant_sql = match __tenant_val {
                Some(_) => {
                    let __tph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    format!(" AND {} = {}", #qi_tid_lit, __tph)
                }
                None => String::new(),
            };
        };
        (block, Some(tid_expr.clone()), 1)
    } else {
        let base_lit = syn::LitInt::new(
            &(dsl_bind_count + 1).to_string(),
            proc_macro2::Span::call_site(),
        );
        let block: proc_macro2::TokenStream = quote! {
            let mut __ph_idx: usize = #base_lit;
            let __tenant_sql = String::new();
        };
        (block, None, 0)
    };

    let where_bind_block: proc_macro2::TokenStream = if !parsed.where_cols.is_empty() {
        quote! {
            #(
                if let Some(ref __wv) = #where_bind_idents {
                    let __wph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    __where_sql.push_str(&format!(" AND {} = {}", #where_col_strs, __wph));
                }
            )*
        }
    } else {
        quote! {}
    };

    let limit_offset_block: proc_macro2::TokenStream = quote! {
        let __limit_ph = if #numbered_lit {
            format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
        } else {
            "?".to_string()
        };
        __ph_idx += 1;
        let __offset_ph = if #numbered_lit {
            format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
        } else {
            "?".to_string()
        };
    };

    let (tenant_bind_dq, tenant_bind_cq): (proc_macro2::TokenStream, proc_macro2::TokenStream) =
        if tenant_bind_data.is_some() {
            (
                quote! { if let Some(ref _tid) = __tenant_val { __dq = __dq.bind(_tid); } },
                quote! { if let Some(ref _tid) = __tenant_val { __cq = __cq.bind(_tid); } },
            )
        } else {
            (quote! {}, quote! {})
        };

    let expanded = quote! {
        {
            let __page_size = #page_size;
            let __offset = (#page - 1).max(0) * __page_size;
            #(let #dsl_bind_idents = #dsl_bind_exprs;)*
            #(let #where_bind_idents = #where_vals;)*
            #tenant_sql_block
            let mut __where_sql = String::new();
            #where_bind_block
            #limit_offset_block
            let mut __data_sql = format!("{}{}{}{}", #data_sql_lit, __tenant_sql, __where_sql, #order_str_lit);
            __data_sql.push_str(&format!(" LIMIT {} OFFSET {}", __limit_ph, __offset_ph));
            let mut __count_sql = format!("{}{}{}", #count_sql_lit, __tenant_sql, __where_sql);
            let mut __dq = sqlx::query_as::<crate::db::pool::Db, #ty>(&__data_sql)#(.bind(#dsl_bind_idents))*;
            #tenant_bind_dq
            #(
                if let Some(ref __wv) = #where_bind_idents {
                    __dq = __dq.bind(__wv);
                }
            )*
            __dq = __dq.bind(__page_size).bind(__offset);
            let __data = __dq.fetch_all(#pool).await?;
            let mut __cq = sqlx::query_scalar::<crate::db::pool::Db, i64>(&__count_sql)#(.bind(#dsl_bind_idents))*;
            #tenant_bind_cq
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

// ── crud_exists! ────────────────────────────────────────────────────

pub fn crud_exists(input: TokenStream) -> TokenStream {
    expand_exists(input)
}

fn expand_exists(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as WhereOnlyInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = crate::where_dsl::validate_where_columns(&parsed.dsl_where, &table.value()) {
        return err.into();
    }

    let pool = &parsed.pool;
    let d = dialect();
    let table_str = table.value();
    let table_lit = syn::LitStr::new(&d.qi(&table_str), table.span());

    let wr = crate::where_dsl::generate_where_runtime(&parsed.dsl_where, d);
    let (local_stmts, bind_stmts) = emit_runtime_binds(&wr.binds);
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, None);
    let sql_code = wr.sql_code;

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #sql_code
            #tenant_sql
            let __sql = format!("SELECT EXISTS(SELECT 1 FROM {} WHERE {}{}{}) as _e", #table_lit, __where_sql, __tenant_sql, "");
            let mut __q = sqlx::query_scalar::<crate::db::pool::Db, bool>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            __q.fetch_one(#pool).await
        }
    };
    TokenStream::from(expanded)
}

// ── crud_upsert! ────────────────────────────────────────────────────

pub fn crud_upsert(input: TokenStream) -> TokenStream {
    expand_upsert(input)
}

fn expand_upsert(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as UpsertInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.cols) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.update_cols) {
        return err;
    }

    let pool = &parsed.pool;
    let table_str = table.value();
    let col_strs: Vec<String> = parsed.cols.iter().map(|l| l.value()).collect();
    let update_col_strs: Vec<String> = parsed.update_cols.iter().map(|l| l.value()).collect();

    let d = dialect();
    let qi_table = d.qi(&table_str);
    let col_list = d.qi_list(&col_strs);
    let n = col_strs.len();

    let vals = &parsed.vals;
    let val_idents: Vec<syn::Ident> = (0..vals.len())
        .map(|i| syn::Ident::new(&format!("__uv_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let ph: Vec<String> = (1..=n).map(|i| d.ph(i)).collect();

    let update_assignments: Vec<String> = update_col_strs
        .iter()
        .map(|c| {
            let excluded = d.excluded_col(c);
            format!("{} = {}", d.qi(c), excluded)
        })
        .collect();
    let update_str = update_assignments.join(", ");

    let conflict_cols = &parsed.conflict_cols;
    let conflict_str: String = conflict_cols
        .iter()
        .map(|l| d.qi(&l.value()))
        .collect::<Vec<_>>()
        .join(", ");

    let upsert_sql = d.upsert_clause(&conflict_str, &update_str);

    let sql = syn::LitStr::new(
        &format!(
            "INSERT INTO {} ({}) VALUES ({}) {}",
            qi_table,
            col_list,
            ph.join(", "),
            upsert_sql
        ),
        table.span(),
    );

    if parsed.tid.is_some() {
        let tid = &parsed.tid;
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                match #tid {
                    Some(_tid) => sqlx::query!(#sql, #(#val_idents),*, _tid).execute(#pool).await,
                    None => sqlx::query!(#sql, #(#val_idents),*).execute(#pool).await,
                }
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            {
                #(let #val_idents = #vals;)*
                sqlx::query!(#sql, #(#val_idents),*).execute(#pool).await
            }
        };
        TokenStream::from(expanded)
    }
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

// ── crud_resolve_ids! ─────────────────────────────────────────────────

pub fn crud_resolve_ids(input: TokenStream) -> TokenStream {
    expand_resolve_ids(input)
}

fn expand_resolve_ids(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as ResolveIdsInput);
    let pool = &parsed.pool;
    let table = &parsed.table;
    let ids = &parsed.ids;
    let d = dialect();

    let qi_id = d.qi("id");
    let qi_id_lit = syn::LitStr::new(&qi_id, proc_macro2::Span::call_site());
    let qi_fmt: &str = match d {
        Dialect::Mysql => "`{}`",
        Dialect::Postgres => "\"{}\"",
        Dialect::Sqlite => "{}",
    };
    let qi_fmt_lit = syn::LitStr::new(qi_fmt, proc_macro2::Span::call_site());

    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
    let ph_prefix_lit = syn::LitStr::new(
        match d {
            Dialect::Postgres => "$",
            _ => "?",
        },
        proc_macro2::Span::call_site(),
    );

    let expanded = quote! {
        {
            let __table: &str = #table;
            let __ids: &[i64] = #ids;
            if __ids.is_empty() {
                Ok::<Vec<i64>, sqlx::Error>(Vec::new())
            } else if !crate::db::driver::is_safe_identifier(__table) {
                Err(sqlx::Error::Configuration(
                    format!("invalid table name: {__table}").into()
                ))
            } else {
                let __qi_table = format!(#qi_fmt_lit, __table);
                let __phs: Vec<String> = (1..=__ids.len())
                    .map(|__i| {
                        if #numbered_lit {
                            format!(concat!(#ph_prefix_lit, "{}"), __i)
                        } else {
                            "?".to_string()
                        }
                    })
                    .collect();
                let __sql = format!("SELECT {} FROM {} WHERE {} IN ({})", #qi_id_lit, __qi_table, #qi_id_lit, __phs.join(", "));
                let mut __q = sqlx::query::<crate::db::pool::Db>(&__sql);
                for &__id in __ids {
                    __q = __q.bind(__id);
                }
                let __rows = __q.fetch_all(#pool).await?;
                let mut __found = std::collections::HashSet::<i64>::new();
                for __row in &__rows {
                    let __v: i64 = __row.try_get("id").unwrap_or(0);
                    if __v > 0 {
                        __found.insert(__v);
                    }
                }
                let mut __result = Vec::with_capacity(__ids.len());
                for &__id in __ids {
                    if !__found.contains(&__id) {
                        return Err(sqlx::Error::RowNotFound.into());
                    }
                    __result.push(__id);
                }
                Ok(__result)
            }
        }
    };
    TokenStream::from(expanded)
}

// ── crud_resolve_id! ──────────────────────────────────────────────────

pub fn crud_resolve_id(input: TokenStream) -> TokenStream {
    expand_resolve_id(input)
}

fn expand_resolve_id(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as ResolveIdInput);
    let pool = &parsed.pool;
    let table = &parsed.table;
    let id = &parsed.id;
    let d = dialect();

    let qi_id = d.qi("id");
    let qi_id_lit = syn::LitStr::new(&qi_id, proc_macro2::Span::call_site());
    let qi_fmt: &str = match d {
        Dialect::Mysql => "`{}`",
        Dialect::Postgres => "\"{}\"",
        Dialect::Sqlite => "{}",
    };
    let qi_fmt_lit = syn::LitStr::new(qi_fmt, proc_macro2::Span::call_site());

    let ph1 = d.ph(1);
    let ph1_lit = syn::LitStr::new(&ph1, proc_macro2::Span::call_site());

    if let Some(tid_expr) = &parsed.tid {
        let (tenant_sql, tenant_bind) = emit_tenant_code(&Some(tid_expr.clone()), d, None);
        let expanded = quote! {
            {
                let __table: &str = #table;
                if !crate::db::driver::is_safe_identifier(__table) {
                    Ok::<Option<i64>, sqlx::Error>(None)
                } else {
                    let __qi_table = format!(#qi_fmt_lit, __table);
                    let __id: i64 = #id;
                    let __pool: &crate::db::Pool = #pool;
                    let mut __ph_idx: usize = 2;
                    #tenant_sql
                    let __sql = format!("SELECT {} FROM {} WHERE {} = {}{}", #qi_id_lit, __qi_table, #qi_id_lit, #ph1_lit, __tenant_sql);
                    let mut __q = sqlx::query_scalar::<crate::db::pool::Db, i64>(&__sql).bind(__id);
                    #tenant_bind
                    __q.fetch_optional(__pool).await
                }
            }
        };
        TokenStream::from(expanded)
    } else {
        let expanded = quote! {
            {
                let __table: &str = #table;
                if !crate::db::driver::is_safe_identifier(__table) {
                    Ok::<Option<i64>, sqlx::Error>(None)
                } else {
                    let __qi_table = format!(#qi_fmt_lit, __table);
                    let __id: i64 = #id;
                    let __pool: &crate::db::Pool = #pool;
                    let __sql = format!("SELECT {} FROM {} WHERE {} = {}", #qi_id_lit, __qi_table, #qi_id_lit, #ph1_lit);
                    sqlx::query_scalar::<crate::db::pool::Db, i64>(&__sql)
                        .bind(__id)
                        .fetch_optional(__pool)
                        .await
                }
            }
        };
        TokenStream::from(expanded)
    }
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

// ── Where-only inputs (used by crud_delete!, crud_count!, crud_exists!) ──

/// `crud_delete!(pool, "table", where: WhereExpr [, tenant: expr])`
/// `crud_count!(pool, "table", where: WhereExpr [, tenant: expr])`
/// `crud_exists!(pool, "table", where: WhereExpr [, tenant: expr])`
struct WhereOnlyInput {
    pool: syn::Expr,
    table: syn::LitStr,
    dsl_where: crate::where_dsl::WhereExpr,
    tid: Option<syn::Expr>,
}

impl syn::parse::Parse for WhereOnlyInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;

        let mut dsl_where = None;
        let mut tid = None;

        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "where" {
                dsl_where = Some(input.parse()?);
            } else if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        let dsl_where =
            dsl_where.ok_or_else(|| syn::Error::new(table.span(), "missing `where:` section"))?;

        Ok(Self {
            pool,
            table,
            dsl_where,
            tid,
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

/// `crud_select!(pool, "table", ["col1", "col2"], where: WhereExpr [, tenant: expr])`
struct SelectInput {
    pool: syn::Expr,
    table: syn::LitStr,
    sel_cols: Vec<syn::LitStr>,
    dsl_where: crate::where_dsl::WhereExpr,
    tid: Option<syn::Expr>,
}

impl syn::parse::Parse for SelectInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let content;
        syn::bracketed!(content in input);
        let mut sel_cols = Vec::new();
        while !content.is_empty() {
            sel_cols.push(content.parse()?);
            let _ = content.parse::<syn::Token![,]>();
        }

        let mut dsl_where = None;
        let mut tid = None;

        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "where" {
                dsl_where = Some(input.parse()?);
            } else if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        let dsl_where =
            dsl_where.ok_or_else(|| syn::Error::new(table.span(), "missing `where:` section"))?;

        Ok(Self {
            pool,
            table,
            sel_cols,
            dsl_where,
            tid,
        })
    }
}

/// A single JOIN clause parsed from the `joins: [...]` bracket.
struct JoinClause {
    join_type: syn::Ident,
    table: syn::LitStr,
    on: syn::LitStr,
}

fn parse_joins(content: syn::parse::ParseStream) -> syn::Result<Vec<JoinClause>> {
    let mut joins = Vec::new();
    while !content.is_empty() {
        let join_type: syn::Ident = content.parse()?;
        let table: syn::LitStr = content.parse()?;
        let _on_kw: syn::Ident = content.parse()?;
        let on: syn::LitStr = content.parse()?;
        joins.push(JoinClause {
            join_type,
            table,
            on,
        });
        let _ = content.parse::<syn::Token![,]>();
    }
    Ok(joins)
}

/// `crud_join!(pool, Type, select: [...], from: "...", joins: [LEFT "table" ON "..."], where: WhereExpr, tenant_alias: "...", tenant: tid, method: fetch_one)`
struct JoinInput {
    pool: syn::Expr,
    ty: syn::Type,
    sel_cols: Vec<syn::LitStr>,
    from: syn::LitStr,
    joins: Vec<JoinClause>,
    dsl_where: Option<crate::where_dsl::WhereExpr>,
    tenant_alias: Option<syn::LitStr>,
    tid: Option<syn::Expr>,
    method: syn::Ident,
    order_by: Option<syn::LitStr>,
    limit: Option<syn::Expr>,
    offset: Option<syn::Expr>,
}

impl syn::parse::Parse for JoinInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;

        let mut sel_cols = Vec::new();
        let mut from = None;
        let mut joins = Vec::new();
        let mut dsl_where = None;
        let mut tenant_alias = None;
        let mut tid = None;
        let mut method = None;
        let mut order_by = None;
        let mut limit = None;
        let mut offset = None;

        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            if section == "select" {
                let content;
                syn::bracketed!(content in input);
                while !content.is_empty() {
                    sel_cols.push(content.parse()?);
                    let _ = content.parse::<syn::Token![,]>();
                }
            } else if section == "from" {
                from = Some(input.parse()?);
            } else if section == "joins" {
                let content;
                syn::bracketed!(content in input);
                joins = parse_joins(&content)?;
            } else if section == "where" {
                dsl_where = Some(input.parse()?);
            } else if section == "tenant_alias" {
                tenant_alias = Some(input.parse()?);
            } else if section == "tenant" {
                tid = Some(input.parse()?);
            } else if section == "method" {
                method = Some(input.parse()?);
            } else if section == "order_by" {
                order_by = Some(input.parse()?);
            } else if section == "limit" {
                limit = Some(input.parse()?);
            } else if section == "offset" {
                offset = Some(input.parse()?);
            }
        }

        Ok(Self {
            pool,
            ty,
            sel_cols,
            from: from.unwrap_or_else(|| syn::LitStr::new("", proc_macro2::Span::call_site())),
            joins,
            dsl_where,
            tenant_alias,
            tid,
            method: method.unwrap_or_else(|| {
                syn::Ident::new("fetch_optional", proc_macro2::Span::call_site())
            }),
            order_by,
            limit,
            offset,
        })
    }
}

fn qi_table_ref(d: &crate::schema::Dialect, table_ref: &str) -> String {
    let mut parts = table_ref.splitn(2, char::is_whitespace);
    let table = parts.next().unwrap_or(table_ref);
    let alias = parts.next().unwrap_or("");
    if alias.is_empty() {
        d.qi(table)
    } else {
        format!("{} {}", d.qi(table), alias)
    }
}

pub fn crud_join(input: TokenStream) -> TokenStream {
    expand_join(input)
}

// ── crud_join_paged! ────────────────────────────────────────────────

pub fn crud_join_paged(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as JoinPagedInput);
    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let d = dialect();
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .collect::<Vec<_>>()
        .join(", ");
    let from_str = parsed.from.value();
    let qi_from = qi_table_ref(&d, &from_str);
    let join_parts: Vec<String> = parsed
        .joins
        .iter()
        .map(|j| {
            let jt = j.join_type.to_string().to_uppercase();
            format!(
                "{} JOIN {} ON {}",
                jt,
                qi_table_ref(&d, &j.table.value()),
                j.on.value()
            )
        })
        .collect();
    let join_str = join_parts.join(" ");

    let sel_lit = syn::LitStr::new(&sel_str, proc_macro2::Span::call_site());
    let from_lit = syn::LitStr::new(&qi_from, proc_macro2::Span::call_site());
    let join_lit = syn::LitStr::new(&join_str, proc_macro2::Span::call_site());
    let from_for_count = syn::LitStr::new(
        &d.qi(from_str.split_whitespace().next().unwrap_or("")),
        proc_macro2::Span::call_site(),
    );

    let order_str = match &parsed.order_by {
        Some(ob) => format!(" ORDER BY {}", ob.value()),
        None => String::new(),
    };
    let order_lit = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());

    let page = &parsed.page;
    let page_size = &parsed.page_size;

    let tenant_alias_ref = parsed.tenant_alias.as_ref();

    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
    let ph_prefix_lit = syn::LitStr::new(
        match d {
            Dialect::Postgres => "$",
            _ => "?",
        },
        proc_macro2::Span::call_site(),
    );

    let tenant_sql_code = if let Some(tid_expr) = &parsed.tid {
        let tenant_col = match tenant_alias_ref {
            Some(a) => format!("{}.{}", a.value(), d.qi("tenant_id")),
            None => d.qi("tenant_id"),
        };
        let tenant_col_lit = syn::LitStr::new(&tenant_col, proc_macro2::Span::call_site());
        quote! {
            let (__tenant_sql, __tid_val) = match #tid_expr {
                Some(_tid) => {
                    let __tph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    (format!(" AND {} = {}", #tenant_col_lit, __tph), Some(_tid))
                },
                None => (String::new(), None),
            };
        }
    } else {
        quote! { let __tenant_sql = String::new(); let __tid_val: Option<_> = None; }
    };

    let tenant_bind = if parsed.tid.is_some() {
        quote! {
            if let Some(_tid) = __tid_val {
                __dq = __dq.bind(_tid);
                __cq = __cq.bind(_tid);
            }
        }
    } else {
        quote! {}
    };

    let (where_sql_code, local_stmts, bind_stmts_dq, bind_stmts_cq) =
        if let Some(ref dsl_where) = parsed.dsl_where {
            let wr = crate::where_dsl::generate_where_runtime(dsl_where, d);
            let ls: Vec<_> = wr
                .binds
                .iter()
                .enumerate()
                .map(|(i, bk)| match bk {
                    crate::where_dsl::BindKind::Static(expr) => {
                        let ident =
                            syn::Ident::new(&format!("__wb_{}", i), proc_macro2::Span::call_site());
                        quote! { let #ident = #expr; }
                    }
                    crate::where_dsl::BindKind::InLoop(expr) => {
                        let ident =
                            syn::Ident::new(&format!("__in_{}", i), proc_macro2::Span::call_site());
                        quote! { let #ident = #expr; }
                    }
                })
                .collect();
            let bs_dq: Vec<_> = wr
                .binds
                .iter()
                .enumerate()
                .map(|(i, bk)| match bk {
                    crate::where_dsl::BindKind::Static(_) => {
                        let ident =
                            syn::Ident::new(&format!("__wb_{}", i), proc_macro2::Span::call_site());
                        quote! { __dq = __dq.bind(#ident); }
                    }
                    crate::where_dsl::BindKind::InLoop(_) => {
                        let ident =
                            syn::Ident::new(&format!("__in_{}", i), proc_macro2::Span::call_site());
                        quote! { for __iv in #ident { __dq = __dq.bind(__iv.clone()); } }
                    }
                })
                .collect();
            let bs_cq: Vec<_> = wr
                .binds
                .iter()
                .enumerate()
                .map(|(i, bk)| match bk {
                    crate::where_dsl::BindKind::Static(_) => {
                        let ident =
                            syn::Ident::new(&format!("__wb_{}", i), proc_macro2::Span::call_site());
                        quote! { __cq = __cq.bind(#ident); }
                    }
                    crate::where_dsl::BindKind::InLoop(_) => {
                        let ident =
                            syn::Ident::new(&format!("__in_{}", i), proc_macro2::Span::call_site());
                        quote! { for __iv in #ident { __cq = __cq.bind(__iv.clone()); } }
                    }
                })
                .collect();
            (wr.sql_code, ls, bs_dq, bs_cq)
        } else {
            let fallback = syn::LitStr::new("1=1", proc_macro2::Span::call_site());
            let code = quote! { let __where_sql = #fallback.to_string(); };
            (code, Vec::new(), Vec::new(), Vec::new())
        };

    let limit_sql_code = quote! {
        let __limit_ph = if #numbered_lit {
            format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
        } else {
            "?".to_string()
        };
        __ph_idx += 1;
        let __offset_ph = if #numbered_lit {
            format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
        } else {
            "?".to_string()
        };
        __ph_idx += 1;
    };

    let expanded = quote! {
        {
            #(#local_stmts)*
            let __page_size = #page_size;
            let __offset = (#page - 1).max(0) * __page_size;
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #where_sql_code
            #tenant_sql_code
            #limit_sql_code
            let __data_sql = format!("SELECT {} FROM {} {} WHERE {}{}{}{} LIMIT {} OFFSET {}", #sel_lit, #from_lit, #join_lit, __where_sql, __tenant_sql, #order_lit, "", __limit_ph, __offset_ph);
            let __count_sql = format!("SELECT COUNT(*) FROM {} WHERE {}{}", #from_for_count, __where_sql, __tenant_sql);
            let mut __dq = sqlx::query_as::<crate::db::pool::Db, #ty>(&__data_sql);
            let mut __cq = sqlx::query_scalar::<crate::db::pool::Db, i64>(&__count_sql);
            #(#bind_stmts_dq)*
            #(#bind_stmts_cq)*
            #tenant_bind
            __dq = __dq.bind(__page_size).bind(__offset);
            let __data = __dq.fetch_all(#pool).await?;
            let __total = __cq.fetch_one(#pool).await?;
            (__data, __total)
        }
    };

    TokenStream::from(expanded)
}

struct JoinPagedInput {
    pool: syn::Expr,
    ty: syn::Type,
    sel_cols: Vec<syn::LitStr>,
    from: syn::LitStr,
    joins: Vec<JoinClause>,
    dsl_where: Option<crate::where_dsl::WhereExpr>,
    tenant_alias: Option<syn::LitStr>,
    tid: Option<syn::Expr>,
    order_by: Option<syn::LitStr>,
    page: syn::Expr,
    page_size: syn::Expr,
}

impl syn::parse::Parse for JoinPagedInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;

        let mut sel_cols = Vec::new();
        let mut from = None;
        let mut joins = Vec::new();
        let mut dsl_where = None;
        let mut tenant_alias = None;
        let mut tid = None;
        let mut order_by = None;
        let mut page = None;
        let mut page_size = None;

        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            if section == "select" {
                let content;
                syn::bracketed!(content in input);
                while !content.is_empty() {
                    sel_cols.push(content.parse()?);
                    let _ = content.parse::<syn::Token![,]>();
                }
            } else if section == "from" {
                from = Some(input.parse()?);
            } else if section == "joins" {
                let content;
                syn::bracketed!(content in input);
                joins = parse_joins(&content)?;
            } else if section == "where" {
                dsl_where = Some(input.parse()?);
            } else if section == "tenant_alias" {
                tenant_alias = Some(input.parse()?);
            } else if section == "tenant" {
                tid = Some(input.parse()?);
            } else if section == "order_by" {
                order_by = Some(input.parse()?);
            } else if section == "page" {
                page = Some(input.parse()?);
            } else if section == "page_size" {
                page_size = Some(input.parse()?);
            }
        }

        let from = from.ok_or_else(|| syn::Error::new(ty.span(), "missing `from:` section"))?;
        let page = page.ok_or_else(|| syn::Error::new(ty.span(), "missing `page:` section"))?;
        let page_size =
            page_size.ok_or_else(|| syn::Error::new(ty.span(), "missing `page_size:` section"))?;

        Ok(Self {
            pool,
            ty,
            sel_cols,
            from,
            joins,
            dsl_where,
            tenant_alias,
            tid,
            order_by,
            page,
            page_size,
        })
    }
}

fn expand_join(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as JoinInput);
    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let d = dialect();
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .collect::<Vec<_>>()
        .join(", ");
    let from_str = parsed.from.value();
    let qi_from = qi_table_ref(&d, &from_str);

    let join_parts: Vec<String> = parsed
        .joins
        .iter()
        .map(|j| {
            let jt = j.join_type.to_string().to_uppercase();
            format!(
                "{} JOIN {} ON {}",
                jt,
                qi_table_ref(&d, &j.table.value()),
                j.on.value()
            )
        })
        .collect();
    let join_str = join_parts.join(" ");

    let sel_lit = syn::LitStr::new(&sel_str, proc_macro2::Span::call_site());
    let from_lit = syn::LitStr::new(&qi_from, proc_macro2::Span::call_site());
    let join_lit = syn::LitStr::new(&join_str, proc_macro2::Span::call_site());
    let method = &parsed.method;

    let order_str = match &parsed.order_by {
        Some(ob) => format!(" ORDER BY {}", ob.value()),
        None => String::new(),
    };
    let order_lit = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());

    let tenant_alias_ref = parsed.tenant_alias.as_ref();
    let (tenant_sql, tenant_bind) = emit_tenant_code(&parsed.tid, d, tenant_alias_ref);

    let (where_sql_code, local_stmts, bind_stmts) = if let Some(ref dsl_where) = parsed.dsl_where {
        let wr = crate::where_dsl::generate_where_runtime(dsl_where, d);
        let (ls, bs) = emit_runtime_binds(&wr.binds);
        (wr.sql_code, ls, bs)
    } else {
        let fallback = syn::LitStr::new("1=1", proc_macro2::Span::call_site());
        let code = quote! { let __where_sql = #fallback.to_string(); };
        (code, Vec::new(), Vec::new())
    };

    let limit_sql_code = if parsed.limit.is_some() {
        let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
        let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
        let ph_prefix_lit = syn::LitStr::new(
            match d {
                Dialect::Postgres => "$",
                _ => "?",
            },
            proc_macro2::Span::call_site(),
        );
        if parsed.offset.is_some() {
            quote! {
                {
                    let __limit_ph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    let __offset_ph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    __sql.push_str(&format!(" LIMIT {} OFFSET {}", __limit_ph, __offset_ph));
                }
            }
        } else {
            quote! {
                {
                    let __limit_ph = if #numbered_lit {
                        format!(concat!(#ph_prefix_lit, "{}"), __ph_idx)
                    } else {
                        "?".to_string()
                    };
                    __ph_idx += 1;
                    __sql.push_str(&format!(" LIMIT {}", __limit_ph));
                }
            }
        }
    } else {
        quote! {}
    };

    let limit_bind = if let Some(lim) = &parsed.limit {
        let off = parsed.offset.as_ref();
        match off {
            Some(o) => quote! { __q = __q.bind(#lim).bind(#o); },
            None => quote! { __q = __q.bind(#lim); },
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        {
            #(#local_stmts)*
            let mut __ph_idx: usize = 1;
            let mut __where_sql = String::new();
            #where_sql_code
            #tenant_sql
            let mut __sql = format!("SELECT {} FROM {} {} WHERE {}{}{}", #sel_lit, #from_lit, #join_lit, __where_sql, __tenant_sql, #order_lit);
            #limit_sql_code
            let mut __q = sqlx::query_as::<crate::db::pool::Db, #ty>(&__sql);
            #(#bind_stmts)*
            #tenant_bind
            #limit_bind
            __q.#method(#pool).await
        }
    };
    TokenStream::from(expanded)
}

/// `crud_insert!(pool, "table", ["col" => val, ...] [, tenant: expr])`
struct InsertInput {
    pool: syn::Expr,
    table: syn::LitStr,
    cols: Vec<syn::LitStr>,
    vals: Vec<syn::Expr>,
    tid: Option<syn::Expr>,
}

impl syn::parse::Parse for InsertInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let (pool, table, cols, vals) = parse_insert_body(input)?;
        let mut tid = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }
        Ok(Self {
            pool,
            table,
            cols,
            vals,
            tid,
        })
    }
}

// ── Scalar / Query inputs ──

/// `crud_scalar!(pool, Type, sql, [val1, val2], method [, tenant: expr])`
struct ScalarInput {
    pool: syn::Expr,
    ty: syn::Type,
    sql: syn::Expr,
    vals: Vec<syn::Expr>,
    method: syn::Ident,
    tid: Option<syn::Expr>,
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
        let method: syn::Ident = input.parse()?;
        let mut tid = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }
        Ok(Self {
            pool,
            ty,
            sql,
            vals,
            method,
            tid,
        })
    }
}

/// `crud_query!(pool, Type, sql, [val1, val2], method [, tenant: expr])`
struct QueryInput {
    pool: syn::Expr,
    ty: syn::Type,
    sql: syn::Expr,
    vals: Vec<syn::Expr>,
    method: syn::Ident,
    tid: Option<syn::Expr>,
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
        let method: syn::Ident = input.parse()?;
        let mut tid = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }
        Ok(Self {
            pool,
            ty,
            sql,
            vals,
            method,
            tid,
        })
    }
}

// ── Find inputs ──

/// `crud_find!(pool, "table", Type, where: WhereExpr [, tenant: expr, order_by: "expr"])`
struct FindInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    dsl_where: crate::where_dsl::WhereExpr,
    tid: Option<syn::Expr>,
    order_by: Option<syn::LitStr>,
}

impl syn::parse::Parse for FindInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ty: syn::Type = input.parse()?;

        let mut dsl_where = None;
        let mut tid = None;
        let mut order_by = None;

        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "where" {
                dsl_where = Some(input.parse()?);
            } else if section == "tenant" {
                tid = Some(input.parse()?);
            } else if section == "order_by" {
                order_by = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        let dsl_where =
            dsl_where.ok_or_else(|| syn::Error::new(ty.span(), "missing `where:` section"))?;

        Ok(Self {
            pool,
            table,
            ty,
            dsl_where,
            tid,
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
    raw_pairs: Vec<(syn::LitStr, syn::Expr)>,
    dsl_where: crate::where_dsl::WhereExpr,
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

/// Parse `["col" => expr, ...]` bracket content into (col, raw_expr) pairs.
/// The left side is a string literal; the right side is any expression.
fn parse_raw_bracket(
    content: &syn::parse::ParseBuffer,
) -> syn::Result<Vec<(syn::LitStr, syn::Expr)>> {
    let mut pairs = Vec::new();
    while !content.is_empty() {
        let col: syn::LitStr = content.parse()?;
        let _: syn::Token![=>] = content.parse()?;
        let val: syn::Expr = content.parse()?;
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
        let mut raw_pairs: Vec<(syn::LitStr, syn::Expr)> = Vec::new();
        let mut dsl_where = None;
        let mut tid = None;

        while !input.is_empty() {
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
                    dsl_where = Some(input.parse()?);
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
            let _ = input.parse::<syn::Token![,]>();
        }

        let dsl_where =
            dsl_where.ok_or_else(|| syn::Error::new(table.span(), "missing `where:` section"))?;

        Ok(Self {
            pool,
            table,
            bind_cols,
            bind_vals,
            opt_cols,
            opt_vals,
            raw_pairs,
            dsl_where,
            tid,
        })
    }
}

// ── QueryPaged input ──

/// `crud_query_paged!(pool, Type, table: "...", where: DSL, order_by: "...", tenant: tid, page: page, page_size: page_size)`
struct QueryPagedInput {
    pool: syn::Expr,
    ty: syn::Type,
    table: syn::LitStr,
    dsl_where: Option<crate::where_dsl::WhereExpr>,
    order_by: Option<syn::LitStr>,
    tid: Option<syn::Expr>,
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

        let mut table = None;
        let mut dsl_where = None;
        let mut order_by = None;
        let mut tid = None;
        let mut page = None;
        let mut page_size = None;
        let mut where_cols = Vec::new();
        let mut where_vals = Vec::new();

        while !input.is_empty() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            match section.to_string().as_str() {
                "table" => {
                    table = Some(input.parse()?);
                }
                "where" => {
                    let fork = input.fork();
                    if fork.peek(syn::token::Bracket) {
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
                    } else {
                        dsl_where = Some(input.parse()?);
                    }
                }
                "order_by" => {
                    order_by = Some(input.parse()?);
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
                other => {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section: {}", other),
                    ));
                }
            }
            let _ = input.parse::<syn::Token![,]>();
        }

        let table = table.ok_or_else(|| syn::Error::new(ty.span(), "missing `table:` section"))?;
        let page = page.ok_or_else(|| syn::Error::new(ty.span(), "missing `page:` section"))?;
        let page_size =
            page_size.ok_or_else(|| syn::Error::new(ty.span(), "missing `page_size:` section"))?;

        Ok(Self {
            pool,
            ty,
            table,
            dsl_where,
            order_by,
            tid,
            page,
            page_size,
            where_cols,
            where_vals,
        })
    }
}

// ── ResolveId input ──

/// `crud_resolve_id!(pool, table, id [, tenant: expr])`
struct ResolveIdInput {
    pool: syn::Expr,
    table: syn::Expr,
    id: syn::Expr,
    tid: Option<syn::Expr>,
}

/// `crud_resolve_ids!(pool, table, ids)`
struct ResolveIdsInput {
    pool: syn::Expr,
    table: syn::Expr,
    ids: syn::Expr,
}

impl syn::parse::Parse for ResolveIdInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let id: syn::Expr = input.parse()?;

        let mut tid = None;
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;
            if section == "tenant" {
                tid = Some(input.parse()?);
            } else {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        Ok(Self {
            pool,
            table,
            id,
            tid,
        })
    }
}

impl syn::parse::Parse for ResolveIdsInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let ids: syn::Expr = input.parse()?;
        Ok(Self { pool, table, ids })
    }
}

// ── Upsert input ──

/// `crud_upsert!(pool, "table", key: ["col1"], bind: ["col" => val, ...], update: ["col1", "col2"] [, tenant: expr])`
struct UpsertInput {
    pool: syn::Expr,
    table: syn::LitStr,
    conflict_cols: Vec<syn::LitStr>,
    cols: Vec<syn::LitStr>,
    vals: Vec<syn::Expr>,
    update_cols: Vec<syn::LitStr>,
    tid: Option<syn::Expr>,
}

impl syn::parse::Parse for UpsertInput {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let pool: syn::Expr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let table: syn::LitStr = input.parse()?;
        let _: syn::Token![,] = input.parse()?;

        let mut conflict_cols = Vec::new();
        let mut cols = Vec::new();
        let mut vals = Vec::new();
        let mut update_cols = Vec::new();
        let mut tid = None;

        while !input.is_empty() {
            let section: syn::Ident = input.call(syn::Ident::parse_any)?;
            let _: syn::Token![:] = input.parse()?;

            match section.to_string().as_str() {
                "key" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        conflict_cols.push(content.parse()?);
                        let _ = content.parse::<syn::Token![,]>();
                    }
                }
                "bind" => {
                    let content;
                    syn::bracketed!(content in input);
                    let (c, v) = parse_kv_bracket(&content)?;
                    cols = c;
                    vals = v;
                }
                "update" => {
                    let content;
                    syn::bracketed!(content in input);
                    while !content.is_empty() {
                        update_cols.push(content.parse()?);
                        let _ = content.parse::<syn::Token![,]>();
                    }
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
            let _ = input.parse::<syn::Token![,]>();
        }

        if conflict_cols.is_empty() {
            return Err(syn::Error::new(
                table.span(),
                "missing `key:` section (conflict columns)",
            ));
        }
        if cols.is_empty() {
            return Err(syn::Error::new(table.span(), "missing `bind:` section"));
        }
        if update_cols.is_empty() {
            return Err(syn::Error::new(table.span(), "missing `update:` section"));
        }

        Ok(Self {
            pool,
            table,
            conflict_cols,
            cols,
            vals,
            update_cols,
            tid,
        })
    }
}
