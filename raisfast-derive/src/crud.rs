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

// ── Extra conditions (and_null / and_gt / and_lt / and_gte / and_lte) ────
//
// Shared across all macros that support `and:`. These extra parameters allow
// IS NULL checks and comparison operators alongside the existing `and:` (equality).

#[derive(Default)]
struct ExtraConds {
    null_cols: Vec<syn::LitStr>,
    gt_cols: Vec<syn::LitStr>,
    gt_vals: Vec<syn::Expr>,
    lt_cols: Vec<syn::LitStr>,
    lt_vals: Vec<syn::Expr>,
    gte_cols: Vec<syn::LitStr>,
    gte_vals: Vec<syn::Expr>,
    lte_cols: Vec<syn::LitStr>,
    lte_vals: Vec<syn::Expr>,
    in_cols: Vec<syn::LitStr>,
    in_vals: Vec<syn::Expr>,
}

impl ExtraConds {
    fn is_empty(&self) -> bool {
        self.null_cols.is_empty()
            && self.gt_cols.is_empty()
            && self.lt_cols.is_empty()
            && self.gte_cols.is_empty()
            && self.lte_cols.is_empty()
            && self.in_cols.is_empty()
    }
}

/// Parse extra condition parameters from a `while` loop section dispatch.
/// Call this inside the section-matching branch for each supported keyword.
fn parse_extra_conds_section(
    ecs: &mut ExtraConds,
    section: &syn::Ident,
    input: syn::parse::ParseStream,
) -> syn::Result<bool> {
    let s = section.to_string();
    match s.as_str() {
        "and_null" => {
            let content;
            syn::bracketed!(content in input);
            while !content.is_empty() {
                ecs.null_cols.push(content.parse()?);
                let _ = content.parse::<syn::Token![,]>();
            }
            Ok(true)
        }
        "and_gt" => {
            let content;
            syn::bracketed!(content in input);
            let (c, v) = parse_kv_bracket(&content)?;
            ecs.gt_cols = c;
            ecs.gt_vals = v;
            Ok(true)
        }
        "and_lt" => {
            let content;
            syn::bracketed!(content in input);
            let (c, v) = parse_kv_bracket(&content)?;
            ecs.lt_cols = c;
            ecs.lt_vals = v;
            Ok(true)
        }
        "and_gte" => {
            let content;
            syn::bracketed!(content in input);
            let (c, v) = parse_kv_bracket(&content)?;
            ecs.gte_cols = c;
            ecs.gte_vals = v;
            Ok(true)
        }
        "and_lte" => {
            let content;
            syn::bracketed!(content in input);
            let (c, v) = parse_kv_bracket(&content)?;
            ecs.lte_cols = c;
            ecs.lte_vals = v;
            Ok(true)
        }
        "and_in" => {
            let content;
            syn::bracketed!(content in input);
            let (c, v) = parse_kv_bracket(&content)?;
            ecs.in_cols = c;
            ecs.in_vals = v;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Build SQL fragments for extra conditions, appending to `parts` and incrementing `ph_idx`.
/// Returns a list of value expressions to bind.
fn build_extra_conds_sql(
    ecs: &ExtraConds,
    d: Dialect,
    ph_idx: &mut usize,
) -> (Vec<String>, Vec<syn::Expr>) {
    let mut parts = Vec::new();
    let mut vals = Vec::new();

    for col in &ecs.null_cols {
        parts.push(format!("AND {} IS NULL", col.value()));
    }
    for (col, val) in ecs.gt_cols.iter().zip(&ecs.gt_vals) {
        let ph = d.ph(*ph_idx);
        *ph_idx += 1;
        parts.push(format!("AND {} > {}", col.value(), ph));
        vals.push(val.clone());
    }
    for (col, val) in ecs.lt_cols.iter().zip(&ecs.lt_vals) {
        let ph = d.ph(*ph_idx);
        *ph_idx += 1;
        parts.push(format!("AND {} < {}", col.value(), ph));
        vals.push(val.clone());
    }
    for (col, val) in ecs.gte_cols.iter().zip(&ecs.gte_vals) {
        let ph = d.ph(*ph_idx);
        *ph_idx += 1;
        parts.push(format!("AND {} >= {}", col.value(), ph));
        vals.push(val.clone());
    }
    for (col, val) in ecs.lte_cols.iter().zip(&ecs.lte_vals) {
        let ph = d.ph(*ph_idx);
        *ph_idx += 1;
        parts.push(format!("AND {} <= {}", col.value(), ph));
        vals.push(val.clone());
    }

    (parts, vals)
}

/// Build a list of extra column names for schema validation.
fn extra_conds_columns(ecs: &ExtraConds) -> Vec<syn::LitStr> {
    let mut cols = Vec::new();
    cols.extend_from_slice(&ecs.null_cols);
    cols.extend_from_slice(&ecs.gt_cols);
    cols.extend_from_slice(&ecs.lt_cols);
    cols.extend_from_slice(&ecs.gte_cols);
    cols.extend_from_slice(&ecs.lte_cols);
    cols.extend_from_slice(&ecs.in_cols);
    cols
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_str = col.value();
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let d = dialect();
        let mut ph_idx = 1usize;
        let col_ph = d.ph(ph_idx);
        ph_idx += 1;
        let mut and_parts: Vec<String> = and_cols
            .iter()
            .map(|ac| {
                let ph = d.ph(ph_idx);
                ph_idx += 1;
                format!("AND {} = {}", ac.value(), ph)
            })
            .collect();
        let (ecs_parts, ecs_vals) = build_extra_conds_sql(&parsed.ecs, d, &mut ph_idx);
        and_parts.extend(ecs_parts);
        let all_extra_vals: Vec<syn::Expr> =
            and_vals.iter().chain(ecs_vals.iter()).cloned().collect();
        let and_str = and_parts.join(" ");
        let tid_ph = d.ph(ph_idx);

        let has_extra = !and_cols.is_empty() || !parsed.ecs.is_empty();

        let expanded = if !has_extra {
            let sql_with_tenant = syn::LitStr::new(
                &format!(
                    "DELETE FROM {} WHERE {} = {} AND tenant_id = {}",
                    table_str, col_str, col_ph, tid_ph
                ),
                table.span(),
            );
            let sql_without_tenant = syn::LitStr::new(
                &format!("DELETE FROM {} WHERE {} = {}", table_str, col_str, col_ph),
                table.span(),
            );
            quote! {
                match #tid {
                    Some(_tid) => sqlx::query!(#sql_with_tenant, #val, _tid).execute(#pool).await,
                    None => sqlx::query!(#sql_without_tenant, #val).execute(#pool).await,
                }
            }
        } else {
            let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__da_{}", i), proc_macro2::Span::call_site()))
                .collect();
            let sql_with_tenant = syn::LitStr::new(
                &format!(
                    "DELETE FROM {} WHERE {} = {} {} AND tenant_id = {}",
                    table_str, col_str, col_ph, and_str, tid_ph
                ),
                table.span(),
            );
            let sql_without_tenant = syn::LitStr::new(
                &format!(
                    "DELETE FROM {} WHERE {} = {} {}",
                    table_str, col_str, col_ph, and_str
                ),
                table.span(),
            );
            quote! {
                {
                    #(let #extra_idents = #all_extra_vals;)*
                    match #tid {
                        Some(_tid) => sqlx::query!(#sql_with_tenant, #val, #(#extra_idents),*, _tid).execute(#pool).await,
                        None => sqlx::query!(#sql_without_tenant, #val, #(#extra_idents),*).execute(#pool).await,
                    }
                }
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let table_str = table.value();
        let col_str = col.value();
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let d = dialect();
        let mut ph_idx = 1usize;
        let col_ph = d.ph(ph_idx);
        ph_idx += 1;
        let mut and_parts: Vec<String> = and_cols
            .iter()
            .map(|ac| {
                let ph = d.ph(ph_idx);
                ph_idx += 1;
                format!("AND {} = {}", ac.value(), ph)
            })
            .collect();
        let (ecs_parts, ecs_vals) = build_extra_conds_sql(&parsed.ecs, d, &mut ph_idx);
        and_parts.extend(ecs_parts);
        let all_extra_vals: Vec<syn::Expr> =
            and_vals.iter().chain(ecs_vals.iter()).cloned().collect();
        let and_str = and_parts.join(" ");

        let has_extra = !and_cols.is_empty() || !parsed.ecs.is_empty();

        let expanded = if !has_extra {
            let sql_lit = syn::LitStr::new(
                &format!("DELETE FROM {} WHERE {} = {}", table_str, col_str, col_ph),
                table.span(),
            );
            quote! {
                sqlx::query!(#sql_lit, #val).execute(#pool).await
            }
        } else {
            let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__da_{}", i), proc_macro2::Span::call_site()))
                .collect();
            let sql_lit = syn::LitStr::new(
                &format!(
                    "DELETE FROM {} WHERE {} = {} {}",
                    table_str, col_str, col_ph, and_str
                ),
                table.span(),
            );
            quote! {
                {
                    #(let #extra_idents = #all_extra_vals;)*
                    sqlx::query!(#sql_lit, #val, #(#extra_idents),*).execute(#pool).await
                }
            }
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
    let input = parse_macro_input!(input as CrudScalarInput);
    let pool = &input.pool;
    let ty = &input.ty;
    let sql = &input.sql;
    let vals = &input.vals;
    let method = &input.method;
    let expanded = quote! {
        sqlx::query_scalar::<_, #ty>(#sql)#(.bind(#vals))*.#method(#pool).await
    };
    TokenStream::from(expanded)
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

// ── tenant_select! / crud_select! ─────────────────────────────────────

pub fn tenant_select(input: TokenStream) -> TokenStream {
    expand_select(input, true)
}

pub fn crud_select(input: TokenStream) -> TokenStream {
    expand_select(input, false)
}

fn expand_select(input: TokenStream, with_tenant: bool) -> TokenStream {
    let parsed = parse_macro_input!(input as SelectInput);
    let table = &parsed.table;

    if let Some(err) = validate_table(table) {
        return err;
    }
    if let Some(err) = validate_column(table, &parsed.col) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.sel_cols) {
        return err;
    }
    if let Some(err) = validate_columns(table, &parsed.and_cols) {
        return err;
    }

    let pool = &parsed.pool;
    let val = &parsed.val;
    let table_str = table.value();
    let col_str = parsed.col.value();
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .collect::<Vec<_>>()
        .join(", ");
    let and_cols = &parsed.and_cols;
    let and_vals = &parsed.and_vals;

    let table_lit = syn::LitStr::new(&table_str, table.span());
    let sel_lit = syn::LitStr::new(&sel_str, table.span());
    let col_lit = syn::LitStr::new(&col_str, table.span());

    let and_col_lits: Vec<syn::LitStr> = and_cols
        .iter()
        .map(|ac| syn::LitStr::new(&format!(" AND {} = ?", ac.value()), table.span()))
        .collect();

    let and_val_idents: Vec<syn::Ident> = (0..and_vals.len())
        .map(|i| syn::Ident::new(&format!("__sav_{}", i), proc_macro2::Span::call_site()))
        .collect();

    if with_tenant {
        let tid = &parsed.tid;
        let expanded = quote! {
            {
                let __sv = #val;
                #(let #and_val_idents = #and_vals;)*
                let __and_sql: &str = concat!(#(#and_col_lits),*);
                let __sql = match #tid {
                    Some(_tid) => format!("SELECT {} FROM {} WHERE {} = ?{} AND tenant_id = ?", #sel_lit, #table_lit, #col_lit, __and_sql),
                    None => format!("SELECT {} FROM {} WHERE {} = ?{}", #sel_lit, #table_lit, #col_lit, __and_sql),
                };
                let mut _q = sqlx::query_as::<_, _>(&__sql).bind(__sv);
                #(_q = _q.bind(#and_val_idents);)*
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                _q.fetch_optional(#pool).await
            }
        };
        TokenStream::from(expanded)
    } else {
        let and_sql: String = and_cols
            .iter()
            .map(|ac| format!(" AND {} = ?", ac.value()))
            .collect();
        let sql_str = format!(
            "SELECT {} FROM {} WHERE {} = ?{}",
            sel_str, table_str, col_str, and_sql
        );
        let sql = syn::LitStr::new(&sql_str, table.span());

        if and_vals.is_empty() {
            let expanded = quote! {
                sqlx::query_as::<_, _>(#sql).bind(#val).fetch_optional(#pool).await
            };
            TokenStream::from(expanded)
        } else {
            let expanded = quote! {
                {
                    #(let #and_val_idents = #and_vals;)*
                    let mut _q = sqlx::query_as::<_, _>(#sql).bind(#val);
                    #(_q = _q.bind(#and_val_idents);)*
                    _q.fetch_optional(#pool).await
                }
            };
            TokenStream::from(expanded)
        }
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let ty = &parsed.ty;
        let val = &parsed.val;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_str = parsed.col.value();
        let cols = get_select_columns(table);
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let method_call = match &method {
            FindMethod::FetchOptional => quote! { fetch_optional(#pool).await },
            FindMethod::FetchOne => quote! { fetch_one(#pool).await },
            FindMethod::FetchAll => quote! { fetch_all(#pool).await },
        };

        let col_lit = syn::LitStr::new(&col_str, table.span());
        let table_lit = syn::LitStr::new(&table_str, table.span());
        let cols_lit = syn::LitStr::new(&cols, table.span());

        let and_col_lits: Vec<syn::LitStr> = and_cols
            .iter()
            .map(|ac| syn::LitStr::new(&format!(" AND {} = ?", ac.value()), table.span()))
            .collect();

        let ecs_null_lits: Vec<syn::LitStr> = parsed
            .ecs
            .null_cols
            .iter()
            .map(|c| syn::LitStr::new(&format!(" AND {} IS NULL", c.value()), table.span()))
            .collect();

        let mut all_bind_vals: Vec<syn::Expr> = and_vals.to_vec();
        all_bind_vals.extend(parsed.ecs.gt_vals.iter().cloned());
        all_bind_vals.extend(parsed.ecs.lt_vals.iter().cloned());
        all_bind_vals.extend(parsed.ecs.gte_vals.iter().cloned());
        all_bind_vals.extend(parsed.ecs.lte_vals.iter().cloned());

        let ecs_cmp_lits: Vec<syn::LitStr> = {
            let d = dialect();
            let mut idx = 1usize;
            let mut lits = Vec::new();
            for _ in &parsed.ecs.gt_vals {
                let ph = d.ph(idx);
                idx += 1;
                let col = &parsed.ecs.gt_cols[lits.len()];
                lits.push(syn::LitStr::new(
                    &format!(" AND {} > {}", col.value(), ph),
                    table.span(),
                ));
            }
            let mut lits2 = Vec::new();
            for _ in &parsed.ecs.lt_vals {
                let ph = d.ph(idx);
                idx += 1;
                let col = &parsed.ecs.lt_cols[lits2.len()];
                lits2.push(syn::LitStr::new(
                    &format!(" AND {} < {}", col.value(), ph),
                    table.span(),
                ));
            }
            let mut lits3 = Vec::new();
            for _ in &parsed.ecs.gte_vals {
                let ph = d.ph(idx);
                idx += 1;
                let col = &parsed.ecs.gte_cols[lits3.len()];
                lits3.push(syn::LitStr::new(
                    &format!(" AND {} >= {}", col.value(), ph),
                    table.span(),
                ));
            }
            let mut lits4 = Vec::new();
            for _ in &parsed.ecs.lte_vals {
                let ph = d.ph(idx);
                idx += 1;
                let col = &parsed.ecs.lte_cols[lits4.len()];
                lits4.push(syn::LitStr::new(
                    &format!(" AND {} <= {}", col.value(), ph),
                    table.span(),
                ));
            }
            lits.into_iter()
                .chain(lits2)
                .chain(lits3)
                .chain(lits4)
                .collect()
        };

        let in_col_lits: Vec<syn::LitStr> = parsed
            .ecs
            .in_cols
            .iter()
            .map(|c| syn::LitStr::new(&c.value(), table.span()))
            .collect();
        let in_vals = &parsed.ecs.in_vals;

        let and_idents: Vec<syn::Ident> = (0..all_bind_vals.len())
            .map(|i| syn::Ident::new(&format!("__fa_{}", i), proc_macro2::Span::call_site()))
            .collect();

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
                #(let #and_idents = #all_bind_vals;)*
                let __ob: &str = #order_by_fragment;
                let mut __and_sql = String::new();
                #(__and_sql.push_str(#and_col_lits);)*
                #(__and_sql.push_str(#ecs_null_lits);)*
                #(__and_sql.push_str(#ecs_cmp_lits);)*
                #(
                    if !#in_vals.is_empty() {
                        let __in_ph: String = (0..#in_vals.len()).map(|_| "?").collect::<Vec<_>>().join(",");
                        __and_sql.push_str(&format!(" AND {} IN ({})", #in_col_lits, __in_ph));
                    }
                )*
                let __sql = match #tid {
                    Some(_tid) => format!("SELECT {} FROM {} WHERE {} = ?{} AND tenant_id = ?{}", #cols_lit, #table_lit, #col_lit, __and_sql, __ob),
                    None => format!("SELECT {} FROM {} WHERE {} = ?{}{}", #cols_lit, #table_lit, #col_lit, __and_sql, __ob),
                };
                let mut _q = sqlx::query_as::<_, #ty>(&__sql).bind(__fv);
                #(_q = _q.bind(#and_idents);)*
                #(
                    for __iv in #in_vals {
                        _q = _q.bind(__iv);
                    }
                )*
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let ty = &parsed.ty;
        let val = &parsed.val;
        let table_str = table.value();
        let col_str = parsed.col.value();
        let cols = get_select_columns(table);
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let method_call = match &method {
            FindMethod::FetchOptional => quote! { fetch_optional(#pool).await },
            FindMethod::FetchOne => quote! { fetch_one(#pool).await },
            FindMethod::FetchAll => quote! { fetch_all(#pool).await },
        };

        let d = dialect();
        let mut sql_str = format!("SELECT {} FROM {} WHERE {} = ?", cols, table_str, col_str);
        let mut all_extra_vals: Vec<syn::Expr> = and_vals.to_vec();
        for ac in and_cols {
            sql_str.push_str(&format!(" AND {} = ?", ac.value()));
        }
        for c in &parsed.ecs.null_cols {
            sql_str.push_str(&format!(" AND {} IS NULL", c.value()));
        }
        {
            let mut idx = sql_str.matches('?').count() + 1;
            for (c, v) in parsed.ecs.gt_cols.iter().zip(&parsed.ecs.gt_vals) {
                sql_str.push_str(&format!(" AND {} > {}", c.value(), d.ph(idx)));
                idx += 1;
                all_extra_vals.push(v.clone());
            }
            for (c, v) in parsed.ecs.lt_cols.iter().zip(&parsed.ecs.lt_vals) {
                sql_str.push_str(&format!(" AND {} < {}", c.value(), d.ph(idx)));
                idx += 1;
                all_extra_vals.push(v.clone());
            }
            for (c, v) in parsed.ecs.gte_cols.iter().zip(&parsed.ecs.gte_vals) {
                sql_str.push_str(&format!(" AND {} >= {}", c.value(), d.ph(idx)));
                idx += 1;
                all_extra_vals.push(v.clone());
            }
            for (c, v) in parsed.ecs.lte_cols.iter().zip(&parsed.ecs.lte_vals) {
                sql_str.push_str(&format!(" AND {} <= {}", c.value(), d.ph(idx)));
                idx += 1;
                all_extra_vals.push(v.clone());
            }
        }
        if let Some(ref ob) = parsed.order_by {
            sql_str.push_str(&format!(" ORDER BY {}", ob.value()));
        }

        let has_in = !parsed.ecs.in_cols.is_empty();

        if has_in {
            let in_col_lits: Vec<syn::LitStr> = parsed
                .ecs
                .in_cols
                .iter()
                .map(|c| syn::LitStr::new(&c.value(), table.span()))
                .collect();
            let in_vals = &parsed.ecs.in_vals;
            let order_str = match &parsed.order_by {
                Some(ob) => format!(" ORDER BY {}", ob.value()),
                None => String::new(),
            };
            let sql_prefix_lit = syn::LitStr::new(&sql_str, table.span());
            let order_lit = syn::LitStr::new(&order_str, table.span());

            let and_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__fa_{}", i), proc_macro2::Span::call_site()))
                .collect();

            let expanded = quote! {
                {
                    #(let #and_idents = #all_extra_vals;)*
                    let mut __sql = #sql_prefix_lit.to_string();
                    #(
                        if !#in_vals.is_empty() {
                            let __in_ph: String = (0..#in_vals.len()).map(|_| "?").collect::<Vec<_>>().join(",");
                            __sql.push_str(&format!(" AND {} IN ({})", #in_col_lits, __in_ph));
                        }
                    )*
                    __sql.push_str(#order_lit);
                    let mut _q = sqlx::query_as::<_, #ty>(&__sql).bind(#val);
                    #(_q = _q.bind(#and_idents);)*
                    #(
                        for __iv in #in_vals {
                            _q = _q.bind(__iv);
                        }
                    )*
                    _q.#method_call
                }
            };
            TokenStream::from(expanded)
        } else {
            let sql = syn::LitStr::new(&sql_str, table.span());

            let and_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__fa_{}", i), proc_macro2::Span::call_site()))
                .collect();

            let expanded = quote! {
                {
                    #(let #and_idents = #all_extra_vals;)*
                    sqlx::query_as::<_, #ty>(#sql).bind(#val)#(.bind(#and_idents))*.#method_call
                }
            };
            TokenStream::from(expanded)
        }
    }
}

// ── tenant_count! / crud_count! ─────────────────────────────────────────

pub fn tenant_count(input: TokenStream) -> TokenStream {
    expand_count(input, true)
}

pub fn crud_count(input: TokenStream) -> TokenStream {
    expand_count(input, false)
}

fn expand_count(input: TokenStream, with_tenant: bool) -> TokenStream {
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let tid = &parsed.tid;
        let table_str = table.value();
        let col_str = col.value();
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let has_in = !parsed.ecs.in_cols.is_empty();

        let d = dialect();
        let mut ph_idx = 1usize;
        let col_ph = d.ph(ph_idx);
        ph_idx += 1;
        let mut and_parts: Vec<String> = and_cols
            .iter()
            .map(|ac| {
                let ph = d.ph(ph_idx);
                ph_idx += 1;
                format!("AND {} = {}", ac.value(), ph)
            })
            .collect();
        let (ecs_parts, ecs_vals) = build_extra_conds_sql(&parsed.ecs, d, &mut ph_idx);
        and_parts.extend(ecs_parts);
        let all_extra_vals: Vec<syn::Expr> =
            and_vals.iter().chain(ecs_vals.iter()).cloned().collect();
        let and_str = and_parts.join(" ");

        let has_extra = !and_cols.is_empty() || !parsed.ecs.is_empty();

        let in_col_lits: Vec<syn::LitStr> = parsed
            .ecs
            .in_cols
            .iter()
            .map(|c| syn::LitStr::new(&c.value(), table.span()))
            .collect();
        let in_vals = &parsed.ecs.in_vals;

        if has_in {
            let and_parts_in: Vec<String> = and_cols
                .iter()
                .map(|ac| format!("AND {} = ?", ac.value()))
                .collect();
            let and_str_in = and_parts_in.join(" ");
            let sql_prefix = format!(
                "SELECT COUNT(*) FROM {} WHERE {} = ?{}{}",
                table_str,
                col_str,
                if and_str_in.is_empty() {
                    String::new()
                } else {
                    format!(" {}", and_str_in)
                },
                String::new()
            );
            let sql_prefix_lit = syn::LitStr::new(&sql_prefix, table.span());
            let tenant_sql_lit = syn::LitStr::new(" AND tenant_id = ?", table.span());
            let empty_lit = syn::LitStr::new("", table.span());

            let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__cnt_{}", i), proc_macro2::Span::call_site()))
                .collect();

            let and_bind = if all_extra_vals.is_empty() {
                quote! {}
            } else {
                quote! { #(_q = _q.bind(#extra_idents);)* }
            };

            let expanded = quote! {
                {
                    #(let #extra_idents = #all_extra_vals;)*
                    let mut __sql = #sql_prefix_lit.to_string();
                    #(
                        if !#in_vals.is_empty() {
                            let __in_ph: String = (0..#in_vals.len()).map(|_| "?").collect::<Vec<_>>().join(",");
                            __sql.push_str(&format!(" AND {} IN ({})", #in_col_lits, __in_ph));
                        }
                    )*
                    let __tenant_sql: &str = match #tid {
                        Some(_) => #tenant_sql_lit,
                        None => #empty_lit,
                    };
                    __sql.push_str(__tenant_sql);
                    let mut _q = sqlx::query_scalar::<_, i64>(&__sql).bind(#val);
                    #and_bind
                    #(
                        for __iv in #in_vals {
                            _q = _q.bind(__iv);
                        }
                    )*
                    if let Some(_tid) = #tid {
                        _q = _q.bind(_tid);
                    }
                    _q.fetch_one(#pool).await
                }
            };
            TokenStream::from(expanded)
        } else {
            let tid_ph = d.ph(ph_idx);

            let expanded = if !has_extra {
                let sql_with = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {} AND tenant_id = {}",
                        table_str, col_str, col_ph, tid_ph
                    ),
                    table.span(),
                );
                let sql_without = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {}",
                        table_str, col_str, col_ph
                    ),
                    table.span(),
                );
                quote! {
                    {
                        match #tid {
                            Some(_tid) => sqlx::query_scalar::<_, i64>(#sql_with).bind(#val).bind(_tid).fetch_one(#pool).await,
                            None => sqlx::query_scalar::< _, i64>(#sql_without).bind(#val).fetch_one(#pool).await,
                        }
                    }
                }
            } else {
                let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                    .map(|i| {
                        syn::Ident::new(&format!("__cnt_{}", i), proc_macro2::Span::call_site())
                    })
                    .collect();
                let sql_with = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {} {} AND tenant_id = {}",
                        table_str, col_str, col_ph, and_str, tid_ph
                    ),
                    table.span(),
                );
                let sql_without = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {} {}",
                        table_str, col_str, col_ph, and_str
                    ),
                    table.span(),
                );
                quote! {
                    {
                        #(let #extra_idents = #all_extra_vals;)*
                        match #tid {
                            Some(_tid) => sqlx::query_scalar::< _, i64>(#sql_with).bind(#val)#(.bind(#extra_idents))*.bind(_tid).fetch_one(#pool).await,
                            None => sqlx::query_scalar::< _, i64>(#sql_without).bind(#val)#(.bind(#extra_idents))*.fetch_one(#pool).await,
                        }
                    }
                }
            };
            TokenStream::from(expanded)
        }
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
        if let Some(err) = validate_columns(table, &parsed.and_cols) {
            return err;
        }
        if let Some(err) = validate_columns(table, &extra_conds_columns(&parsed.ecs)) {
            return err;
        }

        let pool = &parsed.pool;
        let val = &parsed.val;
        let table_str = table.value();
        let col_str = col.value();
        let and_cols = &parsed.and_cols;
        let and_vals = &parsed.and_vals;

        let has_in = !parsed.ecs.in_cols.is_empty();

        let d = dialect();
        let mut ph_idx = 1usize;
        let col_ph = d.ph(ph_idx);
        ph_idx += 1;
        let mut and_parts: Vec<String> = and_cols
            .iter()
            .map(|ac| {
                let ph = d.ph(ph_idx);
                ph_idx += 1;
                format!("AND {} = {}", ac.value(), ph)
            })
            .collect();
        let (ecs_parts, ecs_vals) = build_extra_conds_sql(&parsed.ecs, d, &mut ph_idx);
        and_parts.extend(ecs_parts);
        let all_extra_vals: Vec<syn::Expr> =
            and_vals.iter().chain(ecs_vals.iter()).cloned().collect();
        let and_str = and_parts.join(" ");

        let has_extra = !and_cols.is_empty() || !parsed.ecs.is_empty();

        if has_in {
            let and_parts_in: Vec<String> = and_cols
                .iter()
                .map(|ac| format!("AND {} = ?", ac.value()))
                .collect();
            let and_str_in = and_parts_in.join(" ");
            let sql_prefix = format!(
                "SELECT COUNT(*) FROM {} WHERE {} = ?{}",
                table_str,
                col_str,
                if and_str_in.is_empty() {
                    String::new()
                } else {
                    format!(" {}", and_str_in)
                }
            );
            let sql_prefix_lit = syn::LitStr::new(&sql_prefix, table.span());

            let in_col_lits: Vec<syn::LitStr> = parsed
                .ecs
                .in_cols
                .iter()
                .map(|c| syn::LitStr::new(&c.value(), table.span()))
                .collect();
            let in_vals = &parsed.ecs.in_vals;

            let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                .map(|i| syn::Ident::new(&format!("__cnt_{}", i), proc_macro2::Span::call_site()))
                .collect();

            let and_bind = if all_extra_vals.is_empty() {
                quote! {}
            } else {
                quote! { #(_q = _q.bind(#extra_idents);)* }
            };

            let expanded = quote! {
                {
                    #(let #extra_idents = #all_extra_vals;)*
                    let mut __sql = #sql_prefix_lit.to_string();
                    #(
                        if !#in_vals.is_empty() {
                            let __in_ph: String = (0..#in_vals.len()).map(|_| "?").collect::<Vec<_>>().join(",");
                            __sql.push_str(&format!(" AND {} IN ({})", #in_col_lits, __in_ph));
                        }
                    )*
                    let mut _q = sqlx::query_scalar::<_, i64>(&__sql).bind(#val);
                    #and_bind
                    #(
                        for __iv in #in_vals {
                            _q = _q.bind(__iv);
                        }
                    )*
                    _q.fetch_one(#pool).await
                }
            };
            TokenStream::from(expanded)
        } else {
            let expanded = if !has_extra {
                let sql_lit = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {}",
                        table_str, col_str, col_ph
                    ),
                    table.span(),
                );
                quote! {
                    sqlx::query_scalar::< _, i64>(#sql_lit).bind(#val).fetch_one(#pool).await
                }
            } else {
                let extra_idents: Vec<syn::Ident> = (0..all_extra_vals.len())
                    .map(|i| {
                        syn::Ident::new(&format!("__cnt_{}", i), proc_macro2::Span::call_site())
                    })
                    .collect();
                let sql_lit = syn::LitStr::new(
                    &format!(
                        "SELECT COUNT(*) FROM {} WHERE {} = {} {}",
                        table_str, col_str, col_ph, and_str
                    ),
                    table.span(),
                );
                quote! {
                    {
                        #(let #extra_idents = #all_extra_vals;)*
                        sqlx::query_scalar::< _, i64>(#sql_lit).bind(#val)#(.bind(#extra_idents))*.fetch_one(#pool).await
                    }
                }
            };
            TokenStream::from(expanded)
        }
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
            .map(|i| syn::Ident::new(&format!("__obv_{}", i), proc_macro2::Span::call_site()))
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

/// `tenant_delete!(pool, "table", "col" => val, tenant_id [, and: ["c" => v, ...]])`
struct TenantDeleteInput {
    pool: syn::Expr,
    table: syn::LitStr,
    col: syn::LitStr,
    val: syn::Expr,
    tid: syn::Expr,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    ecs: ExtraConds,
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

        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
            } else if !parse_extra_conds_section(&mut ecs, &section, input)? {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        Ok(Self {
            pool,
            table,
            col,
            val,
            tid,
            and_cols,
            and_vals,
            ecs,
        })
    }
}

/// `crud_delete!(pool, "table", "col" => val [, and: ["c" => v, ...]])`
struct CrudDeleteInput {
    pool: syn::Expr,
    table: syn::LitStr,
    col: syn::LitStr,
    val: syn::Expr,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    ecs: ExtraConds,
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

        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
            } else if !parse_extra_conds_section(&mut ecs, &section, input)? {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        Ok(Self {
            pool,
            table,
            col,
            val,
            and_cols,
            and_vals,
            ecs,
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

/// `tenant_select!(pool, "table", ["col1", "col2"], "where_col" => val, tenant_id [, and: ["col" => val]])`
/// `crud_select!(pool, "table", ["col1", "col2"], "where_col" => val [, and: ["col" => val]])`
struct SelectInput {
    pool: syn::Expr,
    table: syn::LitStr,
    sel_cols: Vec<syn::LitStr>,
    col: syn::LitStr,
    val: syn::Expr,
    tid: Option<syn::Expr>,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    #[allow(dead_code)]
    ecs: ExtraConds,
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
        let _: syn::Token![,] = input.parse()?;
        let col: syn::LitStr = input.parse()?;
        let _: syn::Token![=>] = input.parse()?;
        let val: syn::Expr = input.parse()?;

        let mut tid = None;
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();

        while input.parse::<syn::Token![,]>().is_ok() {
            if input.peek(syn::Ident)
                && input.peek2(syn::Token![:])
                && !input.peek2(syn::Token![::])
            {
                let section: syn::Ident = input.parse()?;
                let _: syn::Token![:] = input.parse()?;
                if section == "and" {
                    let ac;
                    syn::bracketed!(ac in input);
                    let (c, v) = parse_kv_bracket(&ac)?;
                    and_cols = c;
                    and_vals = v;
                } else if !parse_extra_conds_section(&mut ecs, &section, input)? {
                    return Err(syn::Error::new(
                        section.span(),
                        format!("unknown section: {}", section),
                    ));
                }
            } else {
                tid = Some(input.parse()?);
            }
        }

        Ok(Self {
            pool,
            table,
            sel_cols,
            col,
            val,
            tid,
            and_cols,
            and_vals,
            ecs,
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

/// `tenant_join!(pool, Type, select: [...], from: "...", joins: [LEFT "table" ON "..."], where: "col" => val, tenant_alias: "...", tenant: tid, method: fetch_one)`
struct JoinInput {
    pool: syn::Expr,
    ty: syn::Type,
    sel_cols: Vec<syn::LitStr>,
    from: syn::LitStr,
    joins: Vec<JoinClause>,
    where_col: Option<syn::LitStr>,
    where_val: Option<syn::Expr>,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    ecs: ExtraConds,
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
        let mut where_col = None;
        let mut where_val = None;
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();
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
                where_col = Some(input.parse()?);
                let _: syn::Token![=>] = input.parse()?;
                where_val = Some(input.parse()?);
            } else if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
            } else if parse_extra_conds_section(&mut ecs, &section, input)? {
                // handled
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
            where_col,
            where_val,
            and_cols,
            and_vals,
            ecs,
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

pub fn tenant_join(input: TokenStream) -> TokenStream {
    expand_join(input, true)
}

pub fn crud_join(input: TokenStream) -> TokenStream {
    expand_join(input, false)
}

// ── tenant_join_paged! ────────────────────────────────────────────────

pub fn tenant_join_paged(input: TokenStream) -> TokenStream {
    let parsed = parse_macro_input!(input as JoinPagedInput);
    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .collect::<Vec<_>>()
        .join(", ");
    let from_str = parsed.from.value();
    let join_parts: Vec<String> = parsed
        .joins
        .iter()
        .map(|j| {
            let jt = j.join_type.to_string().to_uppercase();
            format!("{} JOIN {} ON {}", jt, j.table.value(), j.on.value())
        })
        .collect();
    let join_str = join_parts.join(" ");

    let mut where_parts: Vec<String> = Vec::new();
    if let Some(ref wc) = parsed.where_col {
        where_parts.push(format!("{} = ?", wc.value()));
    }
    for ac in &parsed.and_cols {
        where_parts.push(format!("{} = ?", ac.value()));
    }

    let all_and_vals: Vec<syn::Expr> = parsed.and_vals.to_vec();

    let has_primary_where = parsed.where_col.is_some();
    let has_where = !where_parts.is_empty();
    let where_str = if has_where {
        where_parts.join(" AND ")
    } else {
        "1=1".to_string()
    };

    let order_str = match &parsed.order_by {
        Some(ob) => format!(" ORDER BY {}", ob.value()),
        None => String::new(),
    };

    let sel_lit = syn::LitStr::new(&sel_str, proc_macro2::Span::call_site());
    let from_lit = syn::LitStr::new(&from_str, proc_macro2::Span::call_site());
    let join_lit = syn::LitStr::new(&join_str, proc_macro2::Span::call_site());
    let where_lit = syn::LitStr::new(&where_str, proc_macro2::Span::call_site());
    let order_lit = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());
    let from_for_count = syn::LitStr::new(
        from_str.split_whitespace().next().unwrap_or(""),
        proc_macro2::Span::call_site(),
    );

    let where_val = &parsed.where_val;
    let tid = &parsed.tid;
    let page = &parsed.page;
    let page_size = &parsed.page_size;
    let tenant_alias = &parsed.tenant_alias;

    let and_val_idents: Vec<syn::Ident> = (0..all_and_vals.len())
        .map(|i| syn::Ident::new(&format!("__jav_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let tenant_sql_with = match tenant_alias {
        Some(alias) => format!(" AND {}.tenant_id = ?", alias.value()),
        None => " AND tenant_id = ?".to_string(),
    };
    let tenant_sql_with_lit = syn::LitStr::new(&tenant_sql_with, proc_macro2::Span::call_site());
    let tenant_sql_empty_lit = syn::LitStr::new("", proc_macro2::Span::call_site());

    let where_bind = if has_primary_where {
        quote! { __dq = __dq.bind(__wv); __cq = __cq.bind(__wv); }
    } else {
        quote! {}
    };

    let expanded = quote! {
        {
            let __page_size = #page_size;
            let __offset = (#page - 1).max(0) * __page_size;
            let __tenant_sql: &str = match #tid {
                Some(_) => #tenant_sql_with_lit,
                None => #tenant_sql_empty_lit,
            };
            #(let #and_val_idents = #all_and_vals;)*
            let __data_sql = format!("SELECT {} FROM {} {} WHERE {}{}{}", #sel_lit, #from_lit, #join_lit, #where_lit, __tenant_sql, #order_lit);
            let __count_sql = format!("SELECT COUNT(*) FROM {} WHERE {}{}", #from_for_count, #where_lit, __tenant_sql);
            let mut __dq = sqlx::query_as::<_, #ty>(&__data_sql);
            let mut __cq = sqlx::query_scalar::<_, i64>(&__count_sql);
            #where_bind
            #(__dq = __dq.bind(#and_val_idents); __cq = __cq.bind(#and_val_idents);)*
            if let Some(_tid) = #tid {
                __dq = __dq.bind(_tid);
                __cq = __cq.bind(_tid);
            }
            __dq = __dq.bind(__page_size).bind(__offset);
            let __data = __dq.fetch_all(#pool).await?;
            let __total = __cq.fetch_one(#pool).await?;
            (__data, __total)
        }
    };

    let full_code = if has_primary_where {
        quote! {
            {
                let __wv = #where_val;
                #expanded
            }
        }
    } else {
        expanded
    };

    TokenStream::from(full_code)
}

struct JoinPagedInput {
    pool: syn::Expr,
    ty: syn::Type,
    sel_cols: Vec<syn::LitStr>,
    from: syn::LitStr,
    joins: Vec<JoinClause>,
    where_col: Option<syn::LitStr>,
    where_val: Option<syn::Expr>,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
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
        let mut where_col = None;
        let mut where_val = None;
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
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
                where_col = Some(input.parse()?);
                let _: syn::Token![=>] = input.parse()?;
                where_val = Some(input.parse()?);
            } else if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
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
            where_col,
            where_val,
            and_cols,
            and_vals,
            tenant_alias,
            tid,
            order_by,
            page,
            page_size,
        })
    }
}

fn expand_join(input: TokenStream, with_tenant: bool) -> TokenStream {
    let parsed = parse_macro_input!(input as JoinInput);
    let pool = &parsed.pool;
    let ty = &parsed.ty;
    let sel_str: String = parsed
        .sel_cols
        .iter()
        .map(|l| l.value())
        .collect::<Vec<_>>()
        .join(", ");
    let from_str = parsed.from.value();

    let join_parts: Vec<String> = parsed
        .joins
        .iter()
        .map(|j| {
            let jt = j.join_type.to_string().to_uppercase();
            format!("{} JOIN {} ON {}", jt, j.table.value(), j.on.value())
        })
        .collect();
    let join_str = join_parts.join(" ");

    let mut where_parts: Vec<String> = Vec::new();
    if let Some(ref wc) = parsed.where_col {
        where_parts.push(format!("{} = ?", wc.value()));
    }
    for ac in &parsed.and_cols {
        where_parts.push(format!("{} = ?", ac.value()));
    }
    for c in &parsed.ecs.null_cols {
        where_parts.push(format!("{} IS NULL", c.value()));
    }

    let mut all_and_vals: Vec<syn::Expr> = parsed.and_vals.to_vec();
    all_and_vals.extend(parsed.ecs.gt_vals.iter().cloned());
    all_and_vals.extend(parsed.ecs.lt_vals.iter().cloned());
    all_and_vals.extend(parsed.ecs.gte_vals.iter().cloned());
    all_and_vals.extend(parsed.ecs.lte_vals.iter().cloned());

    let d = dialect();
    {
        let mut idx = where_parts
            .iter()
            .map(|p| p.matches('?').count())
            .sum::<usize>()
            + 1;
        for (c, _) in parsed.ecs.gt_cols.iter().zip(&parsed.ecs.gt_vals) {
            where_parts.push(format!("{} > {}", c.value(), d.ph(idx)));
            idx += 1;
        }
        for (c, _) in parsed.ecs.lt_cols.iter().zip(&parsed.ecs.lt_vals) {
            where_parts.push(format!("{} < {}", c.value(), d.ph(idx)));
            idx += 1;
        }
        for (c, _) in parsed.ecs.gte_cols.iter().zip(&parsed.ecs.gte_vals) {
            where_parts.push(format!("{} >= {}", c.value(), d.ph(idx)));
            idx += 1;
        }
        for (c, _) in parsed.ecs.lte_cols.iter().zip(&parsed.ecs.lte_vals) {
            where_parts.push(format!("{} <= {}", c.value(), d.ph(idx)));
            idx += 1;
        }
    }

    let has_in = !parsed.ecs.in_cols.is_empty();

    let and_val_idents: Vec<syn::Ident> = (0..all_and_vals.len())
        .map(|i| syn::Ident::new(&format!("__jav_{}", i), proc_macro2::Span::call_site()))
        .collect();

    let has_primary_where = parsed.where_col.is_some();
    let has_where = !where_parts.is_empty();
    let where_str = if has_where {
        where_parts.join(" AND ")
    } else {
        "1=1".to_string()
    };

    let order_str = match &parsed.order_by {
        Some(ob) => format!(" ORDER BY {}", ob.value()),
        None => String::new(),
    };

    let sel_lit = syn::LitStr::new(&sel_str, proc_macro2::Span::call_site());
    let from_lit = syn::LitStr::new(&from_str, proc_macro2::Span::call_site());
    let join_lit = syn::LitStr::new(&join_str, proc_macro2::Span::call_site());
    let where_lit = syn::LitStr::new(&where_str, proc_macro2::Span::call_site());
    let _order_lit = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());

    let where_val = &parsed.where_val;
    let method = &parsed.method;

    let in_col_lits: Vec<syn::LitStr> = parsed
        .ecs
        .in_cols
        .iter()
        .map(|c| syn::LitStr::new(&c.value(), proc_macro2::Span::call_site()))
        .collect();
    let in_vals = &parsed.ecs.in_vals;

    let in_sql_code = if has_in {
        quote! {
            #(
                if !#in_vals.is_empty() {
                    let __in_ph: String = (0..#in_vals.len()).map(|_| "?").collect::<Vec<_>>().join(",");
                    __sql.push_str(&format!(" AND {} IN ({})", #in_col_lits, __in_ph));
                }
            )*
        }
    } else {
        quote! {}
    };

    let in_bind_code = if has_in {
        quote! {
            #(
                for __iv in #in_vals {
                    _q = _q.bind(__iv);
                }
            )*
        }
    } else {
        quote! {}
    };

    if with_tenant {
        let tid = &parsed.tid;
        let tenant_alias = &parsed.tenant_alias;
        let tenant_sql_with = match tenant_alias {
            Some(alias) => format!(" AND {}.tenant_id = ?", alias.value()),
            None => " AND tenant_id = ?".to_string(),
        };
        let tenant_sql_with_lit =
            syn::LitStr::new(&tenant_sql_with, proc_macro2::Span::call_site());
        let tenant_sql_empty_lit = syn::LitStr::new("", proc_macro2::Span::call_site());

        let limit_code = if parsed.limit.is_some() {
            match &parsed.offset {
                Some(_) => quote! { __sql.push_str(" LIMIT ? OFFSET ?"); },
                None => quote! { __sql.push_str(" LIMIT ?"); },
            }
        } else {
            quote! {}
        };

        let limit_bind = if let Some(lim) = &parsed.limit {
            let off = parsed.offset.as_ref();
            match off {
                Some(o) => quote! { _q = _q.bind(#lim).bind(#o); },
                None => quote! { _q = _q.bind(#lim); },
            }
        } else {
            quote! {}
        };

        let where_bind = if has_primary_where {
            quote! { _q = _q.bind(__wv); }
        } else {
            quote! {}
        };

        let order_lit_for_sql = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());

        let expanded = quote! {
            {
                let __tenant_sql: &str = match #tid {
                    Some(_) => #tenant_sql_with_lit,
                    None => #tenant_sql_empty_lit,
                };
                let mut __sql = format!("SELECT {} FROM {} {} WHERE {}{}", #sel_lit, #from_lit, #join_lit, #where_lit, __tenant_sql);
                #in_sql_code
                __sql.push_str(#order_lit_for_sql);
                #limit_code
                let mut _q = sqlx::query_as::<_, #ty>(&__sql);
                #where_bind
                #(_q = _q.bind(#and_val_idents);)*
                #in_bind_code
                if let Some(_tid) = #tid {
                    _q = _q.bind(_tid);
                }
                #limit_bind
                _q.#method(#pool).await
            }
        };

        let full_code = if has_primary_where {
            quote! {
                {
                    let __wv = #where_val;
                    #(let #and_val_idents = #all_and_vals;)*
                    #expanded
                }
            }
        } else {
            quote! {
                {
                    #(let #and_val_idents = #all_and_vals;)*
                    #expanded
                }
            }
        };

        TokenStream::from(full_code)
    } else {
        let sql_prefix = format!(
            "SELECT {} FROM {} {} WHERE {}",
            sel_str, from_str, join_str, where_str
        );

        let lim = &parsed.limit;
        let off = &parsed.offset;

        let limit_code = if let Some(l) = lim.as_ref() {
            let o = off.as_ref().unwrap();
            quote! { _q = _q.bind(#l).bind(#o); }
        } else {
            quote! {}
        };

        if has_in {
            let sql_prefix_lit = syn::LitStr::new(&sql_prefix, proc_macro2::Span::call_site());
            let order_lit_inner = syn::LitStr::new(&order_str, proc_macro2::Span::call_site());

            let limit_sql_code = if parsed.limit.is_some() {
                if parsed.offset.is_some() {
                    quote! { __sql.push_str(" LIMIT ? OFFSET ?"); }
                } else {
                    quote! { __sql.push_str(" LIMIT ?"); }
                }
            } else {
                quote! {}
            };

            let full_code = if has_primary_where {
                quote! {
                    {
                        let __wv = #where_val;
                        #(let #and_val_idents = #all_and_vals;)*
                        let mut __sql = #sql_prefix_lit.to_string();
                        #in_sql_code
                        __sql.push_str(#order_lit_inner);
                        #limit_sql_code
                        let mut _q = sqlx::query_as::<_, #ty>(&__sql).bind(__wv);
                        #(_q = _q.bind(#and_val_idents);)*
                        #in_bind_code
                        #limit_code
                        _q.#method(#pool).await
                    }
                }
            } else {
                quote! {
                    {
                        #(let #and_val_idents = #all_and_vals;)*
                        let mut __sql = #sql_prefix_lit.to_string();
                        #in_sql_code
                        __sql.push_str(#order_lit_inner);
                        #limit_sql_code
                        let mut _q = sqlx::query_as::<_, #ty>(&__sql);
                        #(_q = _q.bind(#and_val_idents);)*
                        #in_bind_code
                        #limit_code
                        _q.#method(#pool).await
                    }
                }
            };
            TokenStream::from(full_code)
        } else {
            let mut sql_str = format!("{}{}", sql_prefix, order_str);

            let limit_sql_code = if let Some(l) = lim.as_ref() {
                sql_str.push_str(" LIMIT ? OFFSET ?");
                let o = off.as_ref().unwrap();
                quote! { _q = _q.bind(#l).bind(#o); }
            } else {
                quote! {}
            };

            if has_primary_where {
                let sql_lit = syn::LitStr::new(&sql_str, proc_macro2::Span::call_site());
                let expanded = quote! {
                    {
                        let __wv = #where_val;
                        #(let #and_val_idents = #all_and_vals;)*
                        let mut _q = sqlx::query_as::<_, #ty>(#sql_lit).bind(__wv);
                        #(_q = _q.bind(#and_val_idents);)*
                        #limit_sql_code
                        _q.#method(#pool).await
                    }
                };
                TokenStream::from(expanded)
            } else {
                let sql_lit = syn::LitStr::new(&sql_str, proc_macro2::Span::call_site());
                let expanded = quote! {
                    {
                        #(let #and_val_idents = #all_and_vals;)*
                        let mut _q = sqlx::query_as::<_, #ty>(#sql_lit);
                        #(_q = _q.bind(#and_val_idents);)*
                        #limit_sql_code
                        _q.#method(#pool).await
                    }
                };
                TokenStream::from(expanded)
            }
        }
    }
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

struct CrudScalarInput {
    pool: syn::Expr,
    ty: syn::Type,
    sql: syn::Expr,
    vals: Vec<syn::Expr>,
    method: syn::Ident,
}

impl syn::parse::Parse for CrudScalarInput {
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
        Ok(Self {
            pool,
            ty,
            sql,
            vals,
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

/// `tenant_find!(pool, "table", Type, "col" => val, tenant_id [, and: ["c" => v, ...], order_by: "expr"])`
struct TenantFindInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    col: syn::LitStr,
    val: syn::Expr,
    tid: syn::Expr,
    order_by: Option<syn::LitStr>,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    ecs: ExtraConds,
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
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "order_by" {
                order_by = Some(input.parse()?);
            } else if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
            } else if !parse_extra_conds_section(&mut ecs, &section, input)? {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
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
            and_cols,
            and_vals,
            ecs,
        })
    }
}

/// `crud_find!(pool, "table", Type, "col" => val [, and: ["c" => v, ...], order_by: "expr"])`
struct CrudFindInput {
    pool: syn::Expr,
    table: syn::LitStr,
    ty: syn::Type,
    col: syn::LitStr,
    val: syn::Expr,
    order_by: Option<syn::LitStr>,
    and_cols: Vec<syn::LitStr>,
    and_vals: Vec<syn::Expr>,
    ecs: ExtraConds,
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
        let mut and_cols = Vec::new();
        let mut and_vals = Vec::new();
        let mut ecs = ExtraConds::default();
        while input.parse::<syn::Token![,]>().is_ok() {
            let section: syn::Ident = input.parse()?;
            let _: syn::Token![:] = input.parse()?;
            if section == "order_by" {
                order_by = Some(input.parse()?);
            } else if section == "and" {
                let content;
                syn::bracketed!(content in input);
                let (c, v) = parse_kv_bracket(&content)?;
                and_cols = c;
                and_vals = v;
            } else if !parse_extra_conds_section(&mut ecs, &section, input)? {
                return Err(syn::Error::new(
                    section.span(),
                    format!("unknown section: {}", section),
                ));
            }
        }

        Ok(Self {
            pool,
            table,
            ty,
            col,
            val,
            order_by,
            and_cols,
            and_vals,
            ecs,
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
    #[allow(dead_code)]
    ecs: ExtraConds,
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
        let mut ecs = ExtraConds::default();
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
                    if !parse_extra_conds_section(&mut ecs, &section, input)? {
                        return Err(syn::Error::new(
                            section.span(),
                            format!("unknown section: {}", other),
                        ));
                    }
                }
            }
            let _ = input.parse::<syn::Token![,]>();
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
            ecs,
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
