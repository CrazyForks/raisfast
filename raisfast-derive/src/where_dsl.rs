use syn::parse::discouraged::Speculative;

use crate::schema::Dialect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
    In,
    NotIn,
    IsNull,
    NotNull,
}

const OP_KEYWORDS: &[&str] = &[
    "EQ", "NEQ", "GT", "GTE", "LT", "LTE", "LIKE", "NOT_LIKE", "IN", "NOT_IN", "IS_NULL",
    "NOT_NULL",
];

fn is_operator_keyword(s: &str) -> bool {
    OP_KEYWORDS.contains(&s)
}

fn parse_op(s: &str, span: proc_macro2::Span) -> syn::Result<CmpOp> {
    match s {
        "EQ" => Ok(CmpOp::Eq),
        "NEQ" => Ok(CmpOp::Neq),
        "GT" => Ok(CmpOp::Gt),
        "GTE" => Ok(CmpOp::Gte),
        "LT" => Ok(CmpOp::Lt),
        "LTE" => Ok(CmpOp::Lte),
        "LIKE" => Ok(CmpOp::Like),
        "NOT_LIKE" => Ok(CmpOp::NotLike),
        "IN" => Ok(CmpOp::In),
        "NOT_IN" => Ok(CmpOp::NotIn),
        "IS_NULL" => Ok(CmpOp::IsNull),
        "NOT_NULL" => Ok(CmpOp::NotNull),
        _ => Err(syn::Error::new(span, format!("unknown operator: {s}"))),
    }
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum WhereExpr {
    Condition {
        col: String,
        col_span: proc_macro2::Span,
        op: CmpOp,
        value: Option<syn::Expr>,
    },
    And(Vec<WhereExpr>),
    Or(Vec<WhereExpr>),
}

impl syn::parse::Parse for WhereExpr {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(syn::Ident) {
            let ident: syn::Ident = input.parse()?;
            match ident.to_string().as_str() {
                "AND" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let mut exprs = Vec::new();
                    while !content.is_empty() {
                        exprs.push(content.parse()?);
                        let _ = content.parse::<syn::Token![,]>();
                    }
                    if exprs.is_empty() {
                        return Err(syn::Error::new(
                            ident.span(),
                            "AND requires at least one condition",
                        ));
                    }
                    Ok(WhereExpr::And(exprs))
                }
                "OR" => {
                    let content;
                    syn::parenthesized!(content in input);
                    let mut exprs = Vec::new();
                    while !content.is_empty() {
                        exprs.push(content.parse()?);
                        let _ = content.parse::<syn::Token![,]>();
                    }
                    if exprs.is_empty() {
                        return Err(syn::Error::new(
                            ident.span(),
                            "OR requires at least one condition",
                        ));
                    }
                    Ok(WhereExpr::Or(exprs))
                }
                _ => Err(syn::Error::new(
                    ident.span(),
                    "expected AND or OR, or a condition tuple",
                )),
            }
        } else if lookahead.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in input);

            let col: syn::LitStr = content.parse()?;
            let _: syn::Token![,] = content.parse()?;

            let fork = content.fork();
            if let Ok(ident) = fork.parse::<syn::Ident>()
                && is_operator_keyword(&ident.to_string())
            {
                content.advance_to(&fork);
                let op = parse_op(&ident.to_string(), ident.span())?;

                if matches!(op, CmpOp::IsNull | CmpOp::NotNull) {
                    return Ok(WhereExpr::Condition {
                        col: col.value(),
                        col_span: col.span(),
                        op,
                        value: None,
                    });
                }

                let _: syn::Token![,] = content.parse()?;
                let value: syn::Expr = content.parse()?;
                return Ok(WhereExpr::Condition {
                    col: col.value(),
                    col_span: col.span(),
                    op,
                    value: Some(value),
                });
            }

            let value: syn::Expr = content.parse()?;
            Ok(WhereExpr::Condition {
                col: col.value(),
                col_span: col.span(),
                op: CmpOp::Eq,
                value: Some(value),
            })
        } else {
            Err(lookahead.error())
        }
    }
}

pub struct WhereCodegen {
    pub sql: String,
    pub binds: Vec<syn::Expr>,
    pub next_idx: usize,
    pub d: Dialect,
}

impl WhereCodegen {
    pub fn new(d: Dialect, start_idx: usize) -> Self {
        Self {
            sql: String::new(),
            binds: Vec::new(),
            next_idx: start_idx,
            d,
        }
    }

    pub fn generate(&mut self, expr: &WhereExpr) {
        match expr {
            WhereExpr::And(exprs) => {
                self.sql.push('(');
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        self.sql.push_str(" AND ");
                    }
                    self.generate(e);
                }
                self.sql.push(')');
            }
            WhereExpr::Or(exprs) => {
                self.sql.push('(');
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        self.sql.push_str(" OR ");
                    }
                    self.generate(e);
                }
                self.sql.push(')');
            }
            WhereExpr::Condition { col, op, value, .. } => {
                self.sql.push_str(&self.d.qi_col(col));
                match op {
                    CmpOp::Eq => {
                        self.sql.push('=');
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Neq => {
                        self.sql.push_str("!=");
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Gt => {
                        self.sql.push('>');
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Gte => {
                        self.sql.push_str(">=");
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Lt => {
                        self.sql.push('<');
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Lte => {
                        self.sql.push_str("<=");
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::Like => {
                        self.sql.push_str(" LIKE ");
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::NotLike => {
                        self.sql.push_str(" NOT LIKE ");
                        self.push_ph();
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::In | CmpOp::NotIn => {
                        let kw = if *op == CmpOp::In { " IN " } else { " NOT IN " };
                        self.sql.push_str(kw);
                        self.sql.push('(');
                        self.push_ph();
                        self.sql.push(')');
                        self.binds.push(value.as_ref().unwrap().clone());
                    }
                    CmpOp::IsNull => {
                        self.sql.push_str(" IS NULL");
                    }
                    CmpOp::NotNull => {
                        self.sql.push_str(" IS NOT NULL");
                    }
                }
            }
        }
    }

    fn push_ph(&mut self) {
        self.sql.push_str(&self.d.ph(self.next_idx));
        self.next_idx += 1;
    }
}

#[derive(Debug)]
pub enum BindKind {
    Static(syn::Expr),
    InLoop(syn::Expr),
}

pub struct WhereRuntimeResult {
    pub sql_code: proc_macro2::TokenStream,
    pub binds: Vec<BindKind>,
}

pub fn generate_where_runtime(expr: &WhereExpr, d: Dialect) -> WhereRuntimeResult {
    let mut result = WhereRuntimeResult {
        sql_code: proc_macro2::TokenStream::new(),
        binds: Vec::new(),
    };
    build_runtime(expr, d, &mut result);
    result
}

fn ph_code(d: Dialect) -> proc_macro2::TokenStream {
    match d {
        Dialect::Sqlite => quote::quote! { format!("?{}", __ph_idx) },
        Dialect::Postgres => quote::quote! { format!("${}", __ph_idx) },
        Dialect::Mysql => quote::quote! { "?".to_string() },
    }
}

fn build_runtime(expr: &WhereExpr, d: Dialect, result: &mut WhereRuntimeResult) {
    match expr {
        WhereExpr::And(exprs) => {
            result
                .sql_code
                .extend(quote::quote! { __where_sql.push('('); });
            for (i, e) in exprs.iter().enumerate() {
                if i > 0 {
                    result
                        .sql_code
                        .extend(quote::quote! { __where_sql.push_str(" AND "); });
                }
                build_runtime(e, d, result);
            }
            result
                .sql_code
                .extend(quote::quote! { __where_sql.push(')'); });
        }
        WhereExpr::Or(exprs) => {
            result
                .sql_code
                .extend(quote::quote! { __where_sql.push('('); });
            for (i, e) in exprs.iter().enumerate() {
                if i > 0 {
                    result
                        .sql_code
                        .extend(quote::quote! { __where_sql.push_str(" OR "); });
                }
                build_runtime(e, d, result);
            }
            result
                .sql_code
                .extend(quote::quote! { __where_sql.push(')'); });
        }
        WhereExpr::Condition { col, op, value, .. } => {
            let col_lit = syn::LitStr::new(&d.qi_col(col), proc_macro2::Span::call_site());
            match op {
                CmpOp::Eq => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push('=');
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Neq => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str("!=");
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Gt => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push('>');
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Gte => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str(">=");
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Lt => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push('<');
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Lte => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str("<=");
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::Like => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str(" LIKE ");
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::NotLike => {
                    let val = value.as_ref().unwrap();
                    let ph = ph_code(d);
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str(" NOT LIKE ");
                        __where_sql.push_str(&{ #ph });
                        __ph_idx += 1;
                    });
                    result.binds.push(BindKind::Static(val.clone()));
                }
                CmpOp::In | CmpOp::NotIn => {
                    let val = value.as_ref().unwrap();
                    let kw = if *op == CmpOp::In { " IN " } else { " NOT IN " };
                    let kw_lit = syn::LitStr::new(kw, proc_macro2::Span::call_site());
                    let numbered = matches!(d, Dialect::Sqlite | Dialect::Postgres);
                    let numbered_lit = syn::LitBool::new(numbered, proc_macro2::Span::call_site());
                    let ph_prefix_lit = syn::LitStr::new(
                        match d {
                            Dialect::Postgres => "$",
                            _ => "?",
                        },
                        proc_macro2::Span::call_site(),
                    );
                    let bind_idx = result.binds.len();
                    let in_ident = syn::Ident::new(
                        &format!("__in_{}", bind_idx),
                        proc_macro2::Span::call_site(),
                    );
                    let fallback = if *op == CmpOp::In { "1=0" } else { "1=1" };
                    let fallback_lit = syn::LitStr::new(fallback, proc_macro2::Span::call_site());
                    result.sql_code.extend(quote::quote! {
                        if !#in_ident.is_empty() {
                            __where_sql.push_str(#col_lit);
                            __where_sql.push_str(#kw_lit);
                            __where_sql.push('(');
                            let __in_ph: String = if #numbered_lit {
                                (0..#in_ident.len()).map(|i| format!(concat!(#ph_prefix_lit, "{}"), __ph_idx + i)).collect::<Vec<_>>().join(",")
                            } else {
                                (0..#in_ident.len()).map(|_| "?").collect::<Vec<_>>().join(",")
                            };
                            __ph_idx += #in_ident.len();
                            __where_sql.push_str(&__in_ph);
                            __where_sql.push(')');
                        } else {
                            __where_sql.push_str(#col_lit);
                            __where_sql.push_str(#kw_lit);
                            __where_sql.push_str(#fallback_lit);
                        }
                    });
                    result.binds.push(BindKind::InLoop(val.clone()));
                }
                CmpOp::IsNull => {
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str(" IS NULL");
                    });
                }
                CmpOp::NotNull => {
                    result.sql_code.extend(quote::quote! {
                        __where_sql.push_str(#col_lit);
                        __where_sql.push_str(" IS NOT NULL");
                    });
                }
            }
        }
    }
}

pub fn validate_where_columns(expr: &WhereExpr, table: &str) -> Option<proc_macro2::TokenStream> {
    match expr {
        WhereExpr::And(exprs) | WhereExpr::Or(exprs) => {
            for e in exprs {
                if let Some(err) = validate_where_columns(e, table) {
                    return Some(err);
                }
            }
            None
        }
        WhereExpr::Condition { col, col_span, .. } => {
            let col_lit = syn::LitStr::new(col, *col_span);
            crate::crud::validate_column_inner(table, &col_lit)
        }
    }
}
