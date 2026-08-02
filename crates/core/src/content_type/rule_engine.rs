//! API Rule expression engine
//!
//! Implements PocketBase-style expression-level access control.
//! Parses rule expressions into an AST, then compiles them to SQL WHERE clauses or runtime boolean evaluation.
//!
//! # Supported syntax (v1 + Phase 1)
//!
//! ```text
//! # Comparison
//! status = "published"                          — field comparison
//! author_id = @request.auth.id                  — authenticated user comparison
//! @request.auth.role = "admin"                  — role check
//! status = "published" && author_id = @request.auth.id
//!
//! # Request context (Phase 1)
//! @request.body.title != ""                     — request body field
//! @request.query.category = "news"              — URL query parameter
//!
//! # Time (Phase 1)
//! created_at > @now                             — current time
//!
//! # Suffix operations (Phase 1)
//! title:isset                                   — field non-null check
//! tags:length > 0                               — string/array length
//! ```
//!
//! # Architecture
//!
//! Expression → Token → AST → SQL WHERE clause / runtime evaluation
//!
//! # Environment variable configuration
//!
//! All hardcoded values for SQL compilation and runtime evaluation can be overridden
//! via `RuleEngineConfig` (environment variables), making it easy to adapt to different
//! database backends and adjust caching strategies. See `AppConfig::rule_engine`.

use crate::config::app::RuleEngineConfig;
use crate::db::DbDriver;
use serde_json::Value;
use std::cmp::Ordering;

/// Expression evaluation context
pub struct RuleContext {
    pub auth_user_id: Option<String>,
    pub auth_role: Option<String>,
    pub body: Option<Value>,
    pub query_params: Option<Value>,
}

impl RuleContext {
    /// Build context from request authentication info
    pub fn from_auth(auth: &crate::middleware::auth::AuthUser) -> Self {
        Self {
            auth_user_id: auth.user_id().map(|id| id.to_string()),
            auth_role: if auth.is_authenticated() {
                Some(auth.role().to_string())
            } else {
                None
            },
            body: None,
            query_params: None,
        }
    }
}

/// Token types
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Identifier(String),
    StringLit(String),
    NumberLit(f64),
    BoolLit(bool),
    NullLit,
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
    And,
    Or,
    LParen,
    RParen,
    Colon,
}

/// AST node
#[derive(Debug, Clone)]
pub enum Expr {
    Compare {
        left: Operand,
        op: CmpOp,
        right: Operand,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    IsSet(Operand),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CmpOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
}

#[derive(Debug, Clone)]
pub enum Operand {
    Field(String),
    AuthId,
    AuthRole,
    RequestBody(String),
    RequestQuery(String),
    Now,
    StringLit(String),
    NumberLit(f64),
    BoolLit(bool),
    Null,
    Length(Box<Operand>),
}

/// Intermediate type for parse_comparison: may be an operand or a sub-expression
enum ExprOrOperand {
    Expr(Expr),
    Operand(Operand),
}

/// Lexer
struct Lexer {
    chars: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.chars.get(self.pos).copied();
        self.pos += 1;
        ch
    }

    fn skip_whitespace(&mut self) {
        while let Some(ch) = self.peek() {
            if ch.is_whitespace() {
                self.advance();
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self, now_keyword: &str) -> Result<Vec<Token>, String> {
        let mut tokens = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => break,
                Some('"') => {
                    self.advance();
                    let mut s = String::new();
                    loop {
                        match self.advance() {
                            None => return Err("unterminated string".into()),
                            Some('"') => break,
                            Some('\\') => match self.advance() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some(c) => s.push(c),
                                None => return Err("unterminated escape".into()),
                            },
                            Some(c) => s.push(c),
                        }
                    }
                    tokens.push(Token::StringLit(s));
                }
                Some(c) if c.is_ascii_digit() => {
                    let mut num = String::new();
                    while let Some(ch) = self.peek() {
                        if ch.is_ascii_digit() || ch == '.' {
                            num.push(ch);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let n: f64 = num.parse().map_err(|_| format!("invalid number: {num}"))?;
                    tokens.push(Token::NumberLit(n));
                }
                Some('=') => {
                    self.advance();
                    tokens.push(Token::Eq);
                }
                Some('!') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Neq);
                    } else if self.peek() == Some('~') {
                        self.advance();
                        tokens.push(Token::NotLike);
                    } else {
                        return Err("expected = or ~ after !".into());
                    }
                }
                Some('>') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Gte);
                    } else {
                        tokens.push(Token::Gt);
                    }
                }
                Some('<') => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token::Lte);
                    } else {
                        tokens.push(Token::Lt);
                    }
                }
                Some('~') => {
                    self.advance();
                    tokens.push(Token::Like);
                }
                Some('(') => {
                    self.advance();
                    tokens.push(Token::LParen);
                }
                Some(')') => {
                    self.advance();
                    tokens.push(Token::RParen);
                }
                Some('&') => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        tokens.push(Token::And);
                    } else {
                        return Err("expected &&".into());
                    }
                }
                Some('|') => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        tokens.push(Token::Or);
                    } else {
                        return Err("expected ||".into());
                    }
                }
                Some(':') => {
                    self.advance();
                    tokens.push(Token::Colon);
                }
                Some('@') => {
                    self.advance();
                    let mut ident = String::from("@");
                    while let Some(ch) = self.peek() {
                        if ch.is_alphanumeric() || ch == '.' || ch == '_' {
                            ident.push(ch);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::Identifier(ident));
                }
                Some(c) if c.is_alphanumeric() || c == '_' => {
                    let mut ident = String::new();
                    while let Some(ch) = self.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            ident.push(ch);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    match ident.as_str() {
                        "true" => tokens.push(Token::BoolLit(true)),
                        "false" => tokens.push(Token::BoolLit(false)),
                        "null" => tokens.push(Token::NullLit),
                        "AND" | "and" => tokens.push(Token::And),
                        "OR" | "or" => tokens.push(Token::Or),
                        kw if kw == now_keyword => {
                            let prefixed = format!("@{kw}");
                            tokens.push(Token::Identifier(prefixed));
                        }
                        _ => tokens.push(Token::Identifier(ident)),
                    };
                }
                Some(c) => return Err(format!("unexpected character: {c}")),
            }
        }
        Ok(tokens)
    }
}

/// Recursive descent parser
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        while self.peek() == Some(&Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let left = self.parse_operand()?;

        if self.peek() == Some(&Token::Colon)
            && let ExprOrOperand::Operand(operand) = left
        {
            self.advance();
            return match self.advance() {
                Some(Token::Identifier(s)) if s == "isset" => Ok(Expr::IsSet(operand)),
                Some(Token::Identifier(s)) if s == "length" => {
                    let inner = Operand::Length(Box::new(operand));
                    let op = match self.peek() {
                        Some(Token::Eq) => CmpOp::Eq,
                        Some(Token::Neq) => CmpOp::Neq,
                        Some(Token::Gt) => CmpOp::Gt,
                        Some(Token::Gte) => CmpOp::Gte,
                        Some(Token::Lt) => CmpOp::Lt,
                        Some(Token::Lte) => CmpOp::Lte,
                        Some(Token::Like) => CmpOp::Like,
                        Some(Token::NotLike) => CmpOp::NotLike,
                        _ => return Err("expected comparison operator after :length".into()),
                    };
                    self.advance();
                    let right = self.parse_atom_inner()?;
                    Ok(Expr::Compare {
                        left: inner,
                        op,
                        right,
                    })
                }
                other => Err(format!("expected isset or length after :, got {other:?}")),
            };
        }

        let op = match self.peek() {
            Some(Token::Eq) => CmpOp::Eq,
            Some(Token::Neq) => CmpOp::Neq,
            Some(Token::Gt) => CmpOp::Gt,
            Some(Token::Gte) => CmpOp::Gte,
            Some(Token::Lt) => CmpOp::Lt,
            Some(Token::Lte) => CmpOp::Lte,
            Some(Token::Like) => CmpOp::Like,
            Some(Token::NotLike) => CmpOp::NotLike,
            _ => {
                return match left {
                    ExprOrOperand::Expr(e) => Ok(e),
                    ExprOrOperand::Operand(_) => Err("bare operand without comparison".into()),
                };
            }
        };
        self.advance();
        let right = self.parse_operand()?;
        match (left, right) {
            (ExprOrOperand::Operand(left), ExprOrOperand::Operand(right)) => {
                Ok(Expr::Compare { left, op, right })
            }
            (ExprOrOperand::Expr(_), _) | (_, ExprOrOperand::Expr(_)) => {
                Err("nested expressions not supported in comparison".into())
            }
        }
    }

    fn parse_operand(&mut self) -> Result<ExprOrOperand, String> {
        match self.peek() {
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                if self.peek() != Some(&Token::RParen) {
                    return Err("expected )".into());
                }
                self.advance();
                Ok(ExprOrOperand::Expr(expr))
            }
            _ => {
                let operand = self.parse_atom_inner()?;
                Ok(ExprOrOperand::Operand(operand))
            }
        }
    }

    fn parse_atom_inner(&mut self) -> Result<Operand, String> {
        match self.advance() {
            Some(Token::Identifier(s)) => match s.as_str() {
                "@request.auth.id" => Ok(Operand::AuthId),
                "@request.auth.role" => Ok(Operand::AuthRole),
                "@now" => Ok(Operand::Now),
                s if s.starts_with("@request.body.") => {
                    let field = s.strip_prefix("@request.body.").unwrap();
                    Ok(Operand::RequestBody(field.to_string()))
                }
                s if s.starts_with("@request.query.") => {
                    let field = s.strip_prefix("@request.query.").unwrap();
                    Ok(Operand::RequestQuery(field.to_string()))
                }
                _ => Ok(Operand::Field(s)),
            },
            Some(Token::StringLit(s)) => Ok(Operand::StringLit(s)),
            Some(Token::NumberLit(n)) => Ok(Operand::NumberLit(n)),
            Some(Token::BoolLit(b)) => Ok(Operand::BoolLit(b)),
            Some(Token::NullLit) => Ok(Operand::Null),
            other => Err(format!("expected operand, got {other:?}")),
        }
    }
}

/// Parsed rule
#[derive(Debug, Clone)]
pub struct Rule {
    expr: Expr,
}

impl Rule {
    /// Parse a rule expression string into an AST
    pub fn parse(source: &str, config: &RuleEngineConfig) -> Result<Self, String> {
        let now_keyword = config
            .prefix_now
            .strip_prefix('@')
            .unwrap_or("now")
            .to_string();
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize(&now_keyword)?;
        let mut parser = Parser::new(tokens);
        let expr = parser.parse_expr()?;
        Ok(Self { expr })
    }

    /// Compile to a SQL WHERE clause fragment and parameter list
    ///
    /// `offset` is the starting SQL parameter index (used for combining multiple rules),
    /// returns `(sql_fragment, params)`.
    pub fn to_sql(&self, offset: usize, config: &RuleEngineConfig) -> (String, Vec<String>) {
        let mut ctx = SqlContext::new(offset, config);
        ctx.compile_expr(&self.expr);
        (ctx.sql, ctx.params)
    }

    /// Runtime evaluation: check whether the rule holds for a database record and request context
    pub fn evaluate(
        &self,
        record: &Value,
        context: &RuleContext,
        config: &RuleEngineConfig,
    ) -> bool {
        eval_expr(&self.expr, record, context, config)
    }
}

/// SQL compilation context
struct SqlContext<'a> {
    sql: String,
    params: Vec<String>,
    param_idx: usize,
    config: &'a RuleEngineConfig,
}

impl<'a> SqlContext<'a> {
    fn new(offset: usize, config: &'a RuleEngineConfig) -> Self {
        Self {
            sql: String::new(),
            params: Vec::new(),
            param_idx: offset + 1,
            config,
        }
    }

    fn next_param(&mut self) -> usize {
        let idx = self.param_idx;
        self.param_idx += 1;
        idx
    }

    fn compile_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Compare { left, op, right } => {
                let left_sql = self.compile_operand_to_sql(left);
                let right_sql = self.compile_operand_to_sql(right);
                let op_sql = match op {
                    CmpOp::Eq => "=",
                    CmpOp::Neq => "!=",
                    CmpOp::Gt => ">",
                    CmpOp::Gte => ">=",
                    CmpOp::Lt => "<",
                    CmpOp::Lte => "<=",
                    CmpOp::Like => "LIKE",
                    CmpOp::NotLike => "NOT LIKE",
                };
                self.sql
                    .push_str(&format!("{left_sql} {op_sql} {right_sql}"));
            }
            Expr::And(lhs, rhs) => {
                self.sql.push('(');
                self.compile_expr(lhs);
                self.sql.push_str(" AND ");
                self.compile_expr(rhs);
                self.sql.push(')');
            }
            Expr::Or(lhs, rhs) => {
                self.sql.push('(');
                self.compile_expr(lhs);
                self.sql.push_str(" OR ");
                self.compile_expr(rhs);
                self.sql.push(')');
            }
            Expr::IsSet(operand) => {
                let sql = self.compile_operand_to_sql(operand);
                self.sql
                    .push_str(&format!("{} {}", sql, self.config.sql_isset_op));
            }
        }
    }

    fn compile_operand_to_sql(&mut self, operand: &Operand) -> String {
        match operand {
            Operand::Field(name) => format!("\"{name}\""),
            Operand::AuthId => "__AUTH_ID__".to_string(),
            Operand::AuthRole => "__AUTH_ROLE__".to_string(),
            Operand::RequestBody(_) | Operand::RequestQuery(_) => "''".to_string(),
            Operand::Now => self.config.sql_now_fn.clone(),
            Operand::StringLit(s) => {
                let idx = self.next_param();
                self.params.push(s.clone());
                crate::db::Driver::ph(idx)
            }
            Operand::NumberLit(n) => {
                let idx = self.next_param();
                self.params.push(n.to_string());
                crate::db::Driver::ph(idx)
            }
            Operand::BoolLit(b) => {
                let idx = self.next_param();
                self.params.push(if *b { "1" } else { "0" }.to_string());
                crate::db::Driver::ph(idx)
            }
            Operand::Null => "NULL".to_string(),
            Operand::Length(inner) => {
                let inner_sql = self.compile_operand_to_sql(inner);
                format!("{}({inner_sql})", self.config.sql_length_fn)
            }
        }
    }
}

/// SQL compilation with auth replacement (public interface)
///
/// Compiles the rule into a SQL WHERE clause, replacing `@request.auth.id` / `@request.auth.role`
/// with actual values parsed from the JWT. Returns None if the rule references auth but the user is not logged in.
pub fn compile_rule_sql(
    rule: &Rule,
    offset: usize,
    auth: &crate::middleware::auth::AuthUser,
    config: &RuleEngineConfig,
) -> Option<(String, Vec<String>)> {
    let needs_auth = expr_needs_auth(&rule.expr);
    if needs_auth && !auth.is_authenticated() {
        return None;
    }

    let (mut sql, mut params) = rule.to_sql(offset, config);

    if auth.is_authenticated() {
        if expr_has_auth_id(&rule.expr) {
            let idx = offset + params.len() + 1;
            params.push(auth.user_id().map_or_else(String::new, |id| id.to_string()));
            sql = sql.replace("__AUTH_ID__", &crate::db::Driver::ph(idx));
        }
        if expr_has_auth_role(&rule.expr) {
            let idx = offset + params.len() + 1;
            params.push(auth.role().to_string());
            sql = sql.replace("__AUTH_ROLE__", &crate::db::Driver::ph(idx));
        }
    }

    Some((sql, params))
}

fn expr_needs_auth(expr: &Expr) -> bool {
    match expr {
        Expr::Compare { left, right, .. } => operand_needs_auth(left) || operand_needs_auth(right),
        Expr::And(l, r) | Expr::Or(l, r) => expr_needs_auth(l) || expr_needs_auth(r),
        Expr::IsSet(op) => operand_needs_auth(op),
    }
}

fn operand_needs_auth(op: &Operand) -> bool {
    match op {
        Operand::AuthId | Operand::AuthRole => true,
        Operand::Length(inner) => operand_needs_auth(inner),
        _ => false,
    }
}

fn expr_has_auth_id(expr: &Expr) -> bool {
    match expr {
        Expr::Compare { left, right, .. } => operand_is_auth_id(left) || operand_is_auth_id(right),
        Expr::And(l, r) | Expr::Or(l, r) => expr_has_auth_id(l) || expr_has_auth_id(r),
        Expr::IsSet(op) => operand_is_auth_id(op),
    }
}

fn operand_is_auth_id(op: &Operand) -> bool {
    match op {
        Operand::AuthId => true,
        Operand::Length(inner) => operand_is_auth_id(inner),
        _ => false,
    }
}

fn expr_has_auth_role(expr: &Expr) -> bool {
    match expr {
        Expr::Compare { left, right, .. } => {
            operand_is_auth_role(left) || operand_is_auth_role(right)
        }
        Expr::And(l, r) | Expr::Or(l, r) => expr_has_auth_role(l) || expr_has_auth_role(r),
        Expr::IsSet(op) => operand_is_auth_role(op),
    }
}

fn operand_is_auth_role(op: &Operand) -> bool {
    match op {
        Operand::AuthRole => true,
        Operand::Length(inner) => operand_is_auth_role(inner),
        _ => false,
    }
}

// ── Runtime evaluation ──────────────────────────────────────────

fn eval_expr(expr: &Expr, record: &Value, ctx: &RuleContext, config: &RuleEngineConfig) -> bool {
    match expr {
        Expr::Compare { left, op, right } => {
            let lv = eval_operand(left, record, ctx);
            let rv = eval_operand(right, record, ctx);
            compare_values(&lv, &rv, op, config)
        }
        Expr::And(l, r) => eval_expr(l, record, ctx, config) && eval_expr(r, record, ctx, config),
        Expr::Or(l, r) => eval_expr(l, record, ctx, config) || eval_expr(r, record, ctx, config),
        Expr::IsSet(operand) => {
            let v = eval_operand(operand, record, ctx);
            !v.is_null()
        }
    }
}

fn eval_operand(operand: &Operand, record: &Value, ctx: &RuleContext) -> Value {
    match operand {
        Operand::Field(name) => record.get(name).cloned().unwrap_or(Value::Null),
        Operand::AuthId => ctx
            .auth_user_id
            .as_ref()
            .map(|s| Value::String(s.clone()))
            .unwrap_or(Value::Null),
        Operand::AuthRole => ctx
            .auth_role
            .as_ref()
            .map(|s| Value::String(s.clone()))
            .unwrap_or(Value::Null),
        Operand::RequestBody(field) => ctx
            .body
            .as_ref()
            .and_then(|b| b.get(field))
            .cloned()
            .unwrap_or(Value::Null),
        Operand::RequestQuery(field) => ctx
            .query_params
            .as_ref()
            .and_then(|q| q.get(field))
            .cloned()
            .unwrap_or(Value::Null),
        Operand::Now => {
            let now = chrono::Utc::now().to_rfc3339();
            Value::String(now)
        }
        Operand::StringLit(s) => Value::String(s.clone()),
        Operand::NumberLit(n) => serde_json::Number::from_f64(*n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Operand::BoolLit(b) => Value::Bool(*b),
        Operand::Null => Value::Null,
        Operand::Length(inner) => {
            let v = eval_operand(inner, record, ctx);
            let len = match &v {
                Value::String(s) => Some(s.len() as f64),
                Value::Array(arr) => Some(arr.len() as f64),
                Value::Object(obj) => Some(obj.len() as f64),
                _ => None,
            };
            len.and_then(|n| serde_json::Number::from_f64(n).map(Value::Number))
                .unwrap_or(Value::Null)
        }
    }
}

fn compare_values(left: &Value, right: &Value, op: &CmpOp, config: &RuleEngineConfig) -> bool {
    match op {
        CmpOp::Eq => value_eq(left, right),
        CmpOp::Neq => !value_eq(left, right),
        CmpOp::Gt => value_ord(left, right) == Some(Ordering::Greater),
        CmpOp::Gte => value_ord(left, right).is_some_and(|o| o != Ordering::Less),
        CmpOp::Lt => value_ord(left, right) == Some(Ordering::Less),
        CmpOp::Lte => value_ord(left, right).is_some_and(|o| o != Ordering::Greater),
        CmpOp::Like => value_like(left, right, config),
        CmpOp::NotLike => !value_like(left, right, config),
    }
}

fn value_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => {
            if let (Some(xf), Some(yf)) = (x.as_f64(), y.as_f64()) {
                (xf - yf).abs() < f64::EPSILON
            } else {
                false
            }
        }
        (Value::String(x), Value::String(y)) => x == y,
        _ => false,
    }
}

fn value_ord(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let xf = x.as_f64()?;
            let yf = y.as_f64()?;
            xf.partial_cmp(&yf)
        }
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

fn value_like(val: &Value, pattern: &Value, config: &RuleEngineConfig) -> bool {
    let (Value::String(s), Value::String(p)) = (val, pattern) else {
        return false;
    };
    let regex_pattern = p
        .replace(&config.sql_like_wildcard, &config.regex_like_wildcard)
        .replace(&config.sql_like_single_char, &config.regex_like_single_char);
    regex::Regex::new(&format!("^{regex_pattern}$")).is_ok_and(|re| re.is_match(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn default_config() -> RuleEngineConfig {
        RuleEngineConfig::default()
    }

    // ── Lexer tests ──────────────────────────────────────────────

    #[test]
    fn tokenize_simple_comparison() {
        let cfg = default_config();
        let mut lexer = Lexer::new(r#"status = "published""#);
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], Token::Identifier("status".into()));
        assert_eq!(tokens[1], Token::Eq);
        assert_eq!(tokens[2], Token::StringLit("published".into()));
    }

    #[test]
    fn tokenize_auth_variable() {
        let cfg = default_config();
        let mut lexer = Lexer::new("author_id = @request.auth.id");
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[0], Token::Identifier("author_id".into()));
        assert_eq!(tokens[1], Token::Eq);
        assert_eq!(tokens[2], Token::Identifier("@request.auth.id".into()));
    }

    #[test]
    fn tokenize_operators() {
        let cfg = default_config();
        let mut lexer = Lexer::new(r#"a >= 1 && b != 2 || c ~ "%test%""#);
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[1], Token::Gte);
        assert_eq!(tokens[2], Token::NumberLit(1.0));
        assert_eq!(tokens[3], Token::And);
        assert_eq!(tokens[5], Token::Neq);
        assert_eq!(tokens[6], Token::NumberLit(2.0));
        assert_eq!(tokens[7], Token::Or);
        assert_eq!(tokens[9], Token::Like);
    }

    #[test]
    fn tokenize_now_keyword() {
        let cfg = default_config();
        let mut lexer = Lexer::new("created_at > @now");
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[0], Token::Identifier("created_at".into()));
        assert_eq!(tokens[1], Token::Gt);
        assert_eq!(tokens[2], Token::Identifier("@now".into()));
    }

    #[test]
    fn tokenize_request_body() {
        let cfg = default_config();
        let mut lexer = Lexer::new("@request.body.title != \"\"");
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[0], Token::Identifier("@request.body.title".into()));
        assert_eq!(tokens[1], Token::Neq);
        assert_eq!(tokens[2], Token::StringLit(String::new()));
    }

    #[test]
    fn tokenize_colon_suffix() {
        let cfg = default_config();
        let mut lexer = Lexer::new("title:isset");
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[0], Token::Identifier("title".into()));
        assert_eq!(tokens[1], Token::Colon);
        assert_eq!(tokens[2], Token::Identifier("isset".into()));
    }

    #[test]
    fn tokenize_custom_now_keyword() {
        let mut cfg = default_config();
        cfg.prefix_now = "@current_time".into();
        let mut lexer = Lexer::new("created_at > @current_time");
        let tokens = lexer.tokenize(&cfg.prefix_now).unwrap();
        assert_eq!(tokens[2], Token::Identifier("@current_time".into()));
    }

    // ── Parser tests ─────────────────────────────────────────────

    #[test]
    fn parse_simple_eq() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, op, right } => {
                assert!(matches!(left, Operand::Field(s) if s == "status"));
                assert_eq!(*op, CmpOp::Eq);
                assert!(matches!(right, Operand::StringLit(s) if s == "published"));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_auth_id() {
        let cfg = default_config();
        let rule = Rule::parse("author_id = @request.auth.id", &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, right, .. } => {
                assert!(matches!(left, Operand::Field(s) if s == "author_id"));
                assert!(matches!(right, Operand::AuthId));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_and_or() {
        let cfg = default_config();
        let rule = Rule::parse(
            r#"status = "published" && author_id = @request.auth.id || status = "draft""#,
            &cfg,
        )
        .unwrap();
        match &rule.expr {
            Expr::Or(_, rhs) => match rhs.as_ref() {
                Expr::Compare { left, .. } => {
                    assert!(matches!(left, Operand::Field(s) if s == "status"));
                }
                _ => panic!("expected Compare in OR rhs"),
            },
            _ => panic!("expected Or"),
        }
    }

    #[test]
    fn parse_parenthesized() {
        let cfg = default_config();
        let rule = Rule::parse(r#"(a = 1 || b = 2) && c = 3"#, &cfg).unwrap();
        match &rule.expr {
            Expr::And(lhs, rhs) => {
                assert!(matches!(lhs.as_ref(), Expr::Or(_, _)));
                match rhs.as_ref() {
                    Expr::Compare { left, .. } => {
                        assert!(matches!(left, Operand::Field(s) if s == "c"));
                    }
                    _ => panic!("expected Compare"),
                }
            }
            _ => panic!("expected And"),
        }
    }

    #[test]
    fn parse_isset() {
        let cfg = default_config();
        let rule = Rule::parse("title:isset", &cfg).unwrap();
        match &rule.expr {
            Expr::IsSet(op) => {
                assert!(matches!(op, Operand::Field(s) if s == "title"));
            }
            _ => panic!("expected IsSet, got {:?}", rule.expr),
        }
    }

    #[test]
    fn parse_length_comparison() {
        let cfg = default_config();
        let rule = Rule::parse("tags:length > 0", &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, op, right } => {
                assert!(
                    matches!(left, Operand::Length(inner) if matches!(inner.as_ref(), Operand::Field(s) if s == "tags"))
                );
                assert_eq!(*op, CmpOp::Gt);
                assert!(matches!(right, Operand::NumberLit(n) if *n == 0.0));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_now() {
        let cfg = default_config();
        let rule = Rule::parse("created_at > @now", &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, right, .. } => {
                assert!(matches!(left, Operand::Field(s) if s == "created_at"));
                assert!(matches!(right, Operand::Now));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_request_body() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.body.title != """#, &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, right, .. } => {
                assert!(matches!(left, Operand::RequestBody(s) if s == "title"));
                assert!(matches!(right, Operand::StringLit(s) if s.is_empty()));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_request_query() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.query.category = "news""#, &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, right, .. } => {
                assert!(matches!(left, Operand::RequestQuery(s) if s == "category"));
                assert!(matches!(right, Operand::StringLit(s) if s == "news"));
            }
            _ => panic!("expected Compare"),
        }
    }

    #[test]
    fn parse_auth_role() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.auth.role = "admin""#, &cfg).unwrap();
        match &rule.expr {
            Expr::Compare { left, right, .. } => {
                assert!(matches!(left, Operand::AuthRole));
                assert!(matches!(right, Operand::StringLit(s) if s == "admin"));
            }
            _ => panic!("expected Compare"),
        }
    }

    // ── to_sql tests ─────────────────────────────────────────────

    #[test]
    fn sql_simple_comparison() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        let (sql, params) = rule.to_sql(0, &cfg);
        assert_eq!(sql, r#""status" = ?"#);
        assert_eq!(params, vec!["published"]);
    }

    #[test]
    fn sql_with_offset() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        let (sql, params) = rule.to_sql(2, &cfg);
        assert_eq!(sql, r#""status" = ?"#);
        assert_eq!(params, vec!["published"]);
    }

    #[test]
    fn sql_and_or() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published" && author_id = "abc""#, &cfg).unwrap();
        let (sql, params) = rule.to_sql(0, &cfg);
        assert!(sql.contains("AND"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn sql_now() {
        let cfg = default_config();
        let rule = Rule::parse("created_at > @now", &cfg).unwrap();
        let (sql, _params) = rule.to_sql(0, &cfg);
        assert!(sql.contains("strftime('%Y-%m-%dT%H:%M:%SZ', 'now')"));
    }

    #[test]
    fn sql_now_custom_fn() {
        let mut cfg = default_config();
        cfg.sql_now_fn = "NOW()".into();
        let rule = Rule::parse("created_at > @now", &cfg).unwrap();
        let (sql, _) = rule.to_sql(0, &cfg);
        assert!(sql.contains("NOW()"));
        assert!(!sql.contains("datetime"));
    }

    #[test]
    fn sql_isset() {
        let cfg = default_config();
        let rule = Rule::parse("title:isset", &cfg).unwrap();
        let (sql, _params) = rule.to_sql(0, &cfg);
        assert!(sql.contains("IS NOT NULL"));
    }

    #[test]
    fn sql_isset_custom_op() {
        let mut cfg = default_config();
        cfg.sql_isset_op = "IS NOT NULL".into(); // same, just testing config path
        let rule = Rule::parse("title:isset", &cfg).unwrap();
        let (sql, _) = rule.to_sql(0, &cfg);
        assert!(sql.contains(&cfg.sql_isset_op));
    }

    #[test]
    fn sql_length() {
        let cfg = default_config();
        let rule = Rule::parse("tags:length > 0", &cfg).unwrap();
        let (sql, params) = rule.to_sql(0, &cfg);
        assert!(sql.contains("LENGTH("));
        assert_eq!(params, vec!["0"]);
    }

    #[test]
    fn sql_length_custom_fn() {
        let mut cfg = default_config();
        cfg.sql_length_fn = "CHAR_LENGTH".into();
        let rule = Rule::parse("tags:length > 0", &cfg).unwrap();
        let (sql, _) = rule.to_sql(0, &cfg);
        assert!(sql.contains("CHAR_LENGTH(\"tags\")"));
    }

    // ── evaluate tests ───────────────────────────────────────────

    #[test]
    fn eval_eq_match() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        let record = json!({"status": "published"});
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));
        assert!(rule.evaluate(&record, &ctx, &cfg));
    }

    #[test]
    fn eval_eq_no_match() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        let record = json!({"status": "draft"});
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));
        assert!(!rule.evaluate(&record, &ctx, &cfg));
    }

    #[test]
    fn eval_auth_id_match() {
        let cfg = default_config();
        let rule = Rule::parse("author_id = @request.auth.id", &cfg).unwrap();
        let record = json!({"author_id": "user123"});
        let ctx = RuleContext {
            auth_user_id: Some("user123".into()),
            auth_role: Some("member".into()),
            body: None,
            query_params: None,
        };
        assert!(rule.evaluate(&record, &ctx, &cfg));
    }

    #[test]
    fn eval_auth_id_no_match() {
        let cfg = default_config();
        let rule = Rule::parse("author_id = @request.auth.id", &cfg).unwrap();
        let record = json!({"author_id": "user123"});
        let ctx = RuleContext {
            auth_user_id: Some("other".into()),
            auth_role: Some("member".into()),
            body: None,
            query_params: None,
        };
        assert!(!rule.evaluate(&record, &ctx, &cfg));
    }

    #[test]
    fn eval_and_or() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published" || status = "draft""#, &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        let record1 = json!({"status": "published"});
        assert!(rule.evaluate(&record1, &ctx, &cfg));

        let record2 = json!({"status": "draft"});
        assert!(rule.evaluate(&record2, &ctx, &cfg));

        let record3 = json!({"status": "archived"});
        assert!(!rule.evaluate(&record3, &ctx, &cfg));
    }

    #[test]
    fn eval_comparison_operators() {
        let cfg = default_config();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        let r1 = Rule::parse("age > 18", &cfg).unwrap();
        assert!(r1.evaluate(&json!({"age": 25}), &ctx, &cfg));
        assert!(!r1.evaluate(&json!({"age": 10}), &ctx, &cfg));

        let r2 = Rule::parse("age >= 18", &cfg).unwrap();
        assert!(r2.evaluate(&json!({"age": 18}), &ctx, &cfg));

        let r3 = Rule::parse("age < 18", &cfg).unwrap();
        assert!(r3.evaluate(&json!({"age": 10}), &ctx, &cfg));

        let r4 = Rule::parse("age <= 18", &cfg).unwrap();
        assert!(r4.evaluate(&json!({"age": 18}), &ctx, &cfg));
    }

    #[test]
    fn eval_like() {
        let cfg = default_config();
        let rule = Rule::parse(r#"title ~ "%rust%""#, &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        assert!(rule.evaluate(&json!({"title": "learning rust"}), &ctx, &cfg));
        assert!(!rule.evaluate(&json!({"title": "learning go"}), &ctx, &cfg));
    }

    #[test]
    fn eval_not_like() {
        let cfg = default_config();
        let rule = Rule::parse(r#"title !~ "%spam%""#, &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        assert!(rule.evaluate(&json!({"title": "hello world"}), &ctx, &cfg));
        assert!(!rule.evaluate(&json!({"title": "spam content"}), &ctx, &cfg));
    }

    #[test]
    fn eval_isset() {
        let cfg = default_config();
        let rule = Rule::parse("title:isset", &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        assert!(rule.evaluate(&json!({"title": "hello"}), &ctx, &cfg));
        assert!(!rule.evaluate(&json!({"title": null}), &ctx, &cfg));
        assert!(!rule.evaluate(&json!({}), &ctx, &cfg));
    }

    #[test]
    fn eval_length() {
        let cfg = default_config();
        let rule = Rule::parse("tags:length > 0", &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        let record1 = json!({"tags": ["rust", "go"]});
        assert!(rule.evaluate(&record1, &ctx, &cfg));

        let record2 = json!({"tags": []});
        assert!(!rule.evaluate(&record2, &ctx, &cfg));

        let rule_str = Rule::parse("name:length > 3", &cfg).unwrap();
        assert!(rule_str.evaluate(&json!({"name": "hello"}), &ctx, &cfg));
        assert!(!rule_str.evaluate(&json!({"name": "hi"}), &ctx, &cfg));
    }

    #[test]
    fn eval_request_body() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.body.title != """#, &cfg).unwrap();
        let ctx = RuleContext {
            auth_user_id: None,
            auth_role: None,
            body: Some(json!({"title": "hello"})),
            query_params: None,
        };
        assert!(rule.evaluate(&json!({}), &ctx, &cfg));

        let ctx_empty = RuleContext {
            auth_user_id: None,
            auth_role: None,
            body: Some(json!({"title": ""})),
            query_params: None,
        };
        assert!(!rule.evaluate(&json!({}), &ctx_empty, &cfg));
    }

    #[test]
    fn eval_request_query() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.query.category = "news""#, &cfg).unwrap();
        let ctx = RuleContext {
            auth_user_id: None,
            auth_role: None,
            body: None,
            query_params: Some(json!({"category": "news"})),
        };
        assert!(rule.evaluate(&json!({}), &ctx, &cfg));

        let ctx_other = RuleContext {
            auth_user_id: None,
            auth_role: None,
            body: None,
            query_params: Some(json!({"category": "tech"})),
        };
        assert!(!rule.evaluate(&json!({}), &ctx_other, &cfg));
    }

    #[test]
    fn eval_auth_role() {
        let cfg = default_config();
        let rule = Rule::parse(r#"@request.auth.role = "admin""#, &cfg).unwrap();
        let ctx_admin = RuleContext {
            auth_user_id: Some("u1".into()),
            auth_role: Some("admin".into()),
            body: None,
            query_params: None,
        };
        assert!(rule.evaluate(&json!({}), &ctx_admin, &cfg));

        let ctx_member = RuleContext {
            auth_user_id: Some("u2".into()),
            auth_role: Some("member".into()),
            body: None,
            query_params: None,
        };
        assert!(!rule.evaluate(&json!({}), &ctx_member, &cfg));
    }

    #[test]
    fn eval_null_comparison() {
        let cfg = default_config();
        let rule = Rule::parse("status = null", &cfg).unwrap();
        let ctx = RuleContext::from_auth(&crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        ));

        assert!(rule.evaluate(&json!({"status": null}), &ctx, &cfg));
        assert!(!rule.evaluate(&json!({"status": "active"}), &ctx, &cfg));
    }

    // ── compile_rule_sql tests ────────────────────────────────────

    #[test]
    fn compile_no_auth() {
        let cfg = default_config();
        let rule = Rule::parse(r#"status = "published""#, &cfg).unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        );
        let (sql, params) = compile_rule_sql(&rule, 0, &auth, &cfg).unwrap();
        assert!(sql.contains("status"));
        assert_eq!(params, vec!["published"]);
    }

    #[test]
    fn compile_with_auth() {
        let cfg = default_config();
        let rule = Rule::parse("author_id = @request.auth.id", &cfg).unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            123,
            crate::models::user::UserRole::Reader,
            "",
        );
        let (sql, params) = compile_rule_sql(&rule, 0, &auth, &cfg).unwrap();
        assert!(!sql.contains("__AUTH_ID__"));
        assert!(params.contains(&"123".to_string()));
    }

    #[test]
    fn compile_needs_auth_but_none_returns_none() {
        let cfg = default_config();
        let rule = Rule::parse("author_id = @request.auth.id", &cfg).unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            0,
            crate::models::user::UserRole::Reader,
            "",
        );
        assert!(compile_rule_sql(&rule, 0, &auth, &cfg).is_none());
    }

    #[test]
    fn compile_combined_filter() {
        let cfg = default_config();
        let rule = Rule::parse(
            r#"status = "published" || author_id = @request.auth.id"#,
            &cfg,
        )
        .unwrap();
        let auth = crate::middleware::auth::AuthUser::new_test(
            999,
            crate::models::user::UserRole::Reader,
            "",
        );
        let (sql, params) = compile_rule_sql(&rule, 0, &auth, &cfg).unwrap();
        assert!(sql.contains("OR"));
        assert!(!sql.contains("__AUTH_ID__"));
        assert!(params.contains(&"published".to_string()));
        assert!(params.contains(&"999".to_string()));
    }

    // ── Configuration tests ──────────────────────────────────────

    #[test]
    fn custom_sql_now_fn_postgres() {
        let mut cfg = default_config();
        cfg.sql_now_fn = "NOW()".into();
        let rule = Rule::parse("created_at > @now", &cfg).unwrap();
        let (sql, _) = rule.to_sql(0, &cfg);
        assert_eq!(sql, "\"created_at\" > NOW()");
    }

    #[test]
    fn custom_sql_length_fn_postgres() {
        let mut cfg = default_config();
        cfg.sql_length_fn = "CHAR_LENGTH".into();
        let rule = Rule::parse("name:length > 3", &cfg).unwrap();
        let (sql, _) = rule.to_sql(0, &cfg);
        assert!(sql.contains("CHAR_LENGTH(\"name\")"));
    }

    #[test]
    fn custom_cache_ttl() {
        let mut cfg = default_config();
        cfg.cms_cache_ttl_secs = 60;
        assert_eq!(cfg.cms_cache_ttl_secs, 60);
    }
}
