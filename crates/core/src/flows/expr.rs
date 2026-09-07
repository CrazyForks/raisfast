//! Minimal value/template & safe-expression evaluation (contracts.md C3).
//!
//! Pure-Rust, language-agnostic. v1 subset:
//! - refs `{{#ns.name.child#}}` (whole-string → typed value)
//! - template interpolation in strings (inline `{{#…#}}`)
//! - expressions: null / numbers / strings / booleans + `+ - * / % > >= < <=
//!   == != && || ! ( )` and a small cleaning-function set (transform-node.md):
//!   `concat to_number to_string upper lower trim replace substring length
//!   default round if first sum join now date_add format_date regex_test
//!   regex_extract regex_replace`（数组可经 `[a, b]` 字面量与数组 ref 进入表达式）
//!
//! Unified value model (2026-09-07): the parser produces typed [`Value`]s
//! throughout; [`eval_bool`] is [`eval_value`] plus the legacy truthiness
//! coercion. Deliberate behavior fixes over the old bool-centric parser
//! (characterization-tested):
//! - `(x)` returns the inner VALUE (was: forced `Bool`) — `(5)==5` was `false`
//! - unary minus `-5` parses (was: "表达式意外结束")
//! - `null` is a literal (needed by `default()`)
//!
//! Arithmetic stays f64-numeric (`+` never concatenates — use `concat()`);
//! div-by-zero keeps its historical `null` result.

use std::collections::HashMap;

use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};

use super::engine::Pool;

fn resolve_ref_in_pool(pool: &Pool, sel: &[&str]) -> AppResult<Value> {
    // v2 D7: a single-segment `[ns]` resolves to the node's whole namespace
    // (object of its declared fields).
    if sel.len() == 1 {
        let ns = sel[0];
        let m = pool
            .get(ns)
            .ok_or_else(|| AppError::BadRequest(format!("ref 引用不存在: {ns}")))?;
        let map: serde_json::Map<String, Value> = m.clone().into_iter().collect();
        return Ok(Value::Object(map));
    }
    if sel.is_empty() {
        return Err(AppError::BadRequest("ref 不能为空".into()));
    }
    let ns = sel[0];
    let name = sel[1];
    let mut v = pool
        .get(ns)
        .and_then(|m| m.get(name))
        .cloned()
        .ok_or_else(|| AppError::BadRequest(format!("ref 引用不存在: {ns}.{name}")))?;
    for part in &sel[2..] {
        v = v
            .get(part)
            .cloned()
            .ok_or_else(|| AppError::BadRequest(format!("ref 子路径不存在: {part}")))?;
    }
    Ok(v)
}

/// Positions + inner selector of `{{#sel#}}` tokens in a string.
///
/// `i` always sits on a UTF-8 char boundary (tokens are ASCII, so they can
/// only start on boundaries anyway); advancing byte-by-byte would slice into
/// multi-byte characters and panic on CJK text.
fn find_tokens(text: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if text[i..].starts_with("{{#")
            && let Some(rel) = text[i + 3..].find("#}}")
        {
            let inner = &text[i + 3..i + 3 + rel];
            out.push((i, i + 3 + rel + 3, inner.to_string()));
            i += 3 + rel + 3;
            continue;
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Inner selectors (`ns.field.child`) of every `{{#…#}}` token in `text`.
/// Shared with the publish-time reference lint (design D4).
#[must_use]
pub fn selectors_in_text(text: &str) -> Vec<String> {
    find_tokens(text)
        .into_iter()
        .map(|(_, _, inner)| inner)
        .collect()
}

/// Resolve a string possibly containing `{{#sel#}}`: a single whole-string
/// token returns the typed value; otherwise tokens are interpolated as text.
pub fn resolve_text(text: &str, pool: &Pool) -> AppResult<Value> {
    let tokens = find_tokens(text);
    if tokens.is_empty() {
        return Ok(Value::String(text.to_string()));
    }
    let whole = tokens.len() == 1 && tokens[0].0 == 0 && tokens[0].1 == text.len();
    if whole {
        let sel: Vec<&str> = tokens[0].2.split('.').collect();
        return resolve_ref_in_pool(pool, &sel);
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, inner) in tokens {
        out.push_str(&text[cursor..start]);
        let sel: Vec<&str> = inner.split('.').collect();
        let v = resolve_ref_in_pool(pool, &sel)?;
        out.push_str(&scalar_text(&v));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    Ok(Value::String(out))
}

fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// Evaluate an expression string to a typed value (C1.2 third state — the
/// cleaning entrypoint for `{expr: …}` ValueExprs). `{{#…#}}` refs are
/// resolved to literals before parsing.
pub fn eval_value(expr: &str, pool: &Pool) -> AppResult<Value> {
    if expr.len() > EXPR_MAX_LEN {
        return Err(AppError::BadRequest(format!(
            "表达式超长（≤{EXPR_MAX_LEN} 字符）"
        )));
    }
    let normalized = normalize_refs(expr, pool)?;
    Parser::new(&normalized).parse()
}

/// Evaluate an expression string to a boolean. `{{#…#}}` refs are resolved to
/// literals before parsing (used by branch `when`/`skip_if`/`retry_if`).
/// Legacy truthiness coercion preserved exactly: bool as-is, numbers
/// non-zero → true, null/other → false.
pub fn eval_bool(expr: &str, pool: &Pool) -> AppResult<bool> {
    Ok(coerce_bool(&eval_value(expr, pool)?))
}

/// Shared `{{#…#}}` → literal-syntax normalization.
fn normalize_refs(expr: &str, pool: &Pool) -> AppResult<String> {
    let mut normalized = String::new();
    let mut rest = expr;
    while let Some(start) = rest.find("{{#") {
        normalized.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let Some(end_rel) = after.find("#}}") else {
            return Err(AppError::BadRequest(format!("表达式含未闭合引用: {expr}")));
        };
        let sel: Vec<&str> = after[..end_rel].split('.').collect();
        let v = resolve_ref_in_pool(pool, &sel)?;
        normalized.push_str(&literal_syntax(&v)?);
        rest = &after[end_rel + 3..];
    }
    normalized.push_str(rest);
    Ok(normalized)
}

/// Legacy truthiness (kept byte-for-byte from the old `Parser::as_bool`).
fn coerce_bool(v: &Value) -> bool {
    v.as_bool()
        .unwrap_or_else(|| !v.is_null() && v.as_f64().unwrap_or(0.0) != 0.0)
}

/// Expression hard caps (admin-authored surface; hygiene against pathological
/// input, [自造] guard — see transform-node.md).
const EXPR_MAX_LEN: usize = 4096;
const EXPR_MAX_DEPTH: usize = 128;

fn literal_syntax(v: &Value) -> AppResult<String> {
    Ok(match v {
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::String(s) => format!("{s:?}"),
        Value::Array(_) => serde_json::to_string(v)
            .map_err(|e| AppError::BadRequest(format!("数组序列化失败: {e}")))?,
        other => {
            return Err(AppError::BadRequest(format!(
                "表达式引用不支持该类型（仅标量）: {other}"
            )));
        }
    })
}

/// f64 → JSON number: integer-valued floats kept as i64 so display stays
/// `3` (not `3.0`) and `to_string`/`concat` read naturally. Non-finite
/// (div-by-zero overflow) keeps the historical `null` result.
fn num_value(f: f64) -> Value {
    if !f.is_finite() {
        return Value::Null;
    }
    if f.fract() == 0.0 && f.abs() <= 9.0e15 {
        Value::from(f as i64)
    } else {
        Value::from(f)
    }
}

/// Numeric operand coercion (strict, loud failure — unchanged semantics).
fn num_operand(v: &Value) -> AppResult<f64> {
    v.as_f64()
        .ok_or_else(|| AppError::BadRequest(format!("表达式需要数字，遇到 {v}")))
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s,
            b: s.as_bytes(),
            pos: 0,
            depth: 0,
        }
    }
    fn peek(&self) -> Option<char> {
        self.b.get(self.pos).copied().map(char::from)
    }
    fn skip_ws(&mut self) {
        while self.pos < self.b.len() && (self.b[self.pos] as char).is_whitespace() {
            self.pos += 1;
        }
    }
    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn take_op(&mut self, op: &str) -> bool {
        self.skip_ws();
        if self.s[self.pos..].starts_with(op) {
            self.pos += op.len();
            true
        } else {
            false
        }
    }
    fn enter(&mut self) -> AppResult<()> {
        self.depth += 1;
        if self.depth > EXPR_MAX_DEPTH {
            return Err(AppError::BadRequest(format!(
                "表达式嵌套过深（≤{EXPR_MAX_DEPTH}）"
            )));
        }
        Ok(())
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
    fn parse(&mut self) -> AppResult<Value> {
        let v = self.parse_or()?;
        self.skip_ws();
        if self.pos != self.b.len() {
            return Err(AppError::BadRequest(format!(
                "表达式尾部有多余内容: {}",
                &self.s[self.pos..]
            )));
        }
        Ok(v)
    }
    /// `||` / `&&` evaluate BOTH sides (no parse-level short-circuit) and
    /// always produce Bool — truth tables identical to the old parser for
    /// bool contexts; garbage on the right side errors either way.
    fn parse_or(&mut self) -> AppResult<Value> {
        let mut v = self.parse_and()?;
        while self.take_op("||") {
            let r = self.parse_and()?;
            v = Value::Bool(coerce_bool(&v) || coerce_bool(&r));
        }
        Ok(v)
    }
    fn parse_and(&mut self) -> AppResult<Value> {
        let mut v = self.parse_cmp()?;
        while self.take_op("&&") {
            let r = self.parse_cmp()?;
            v = Value::Bool(coerce_bool(&v) && coerce_bool(&r));
        }
        Ok(v)
    }
    fn parse_cmp(&mut self) -> AppResult<Value> {
        let left = self.parse_arith()?;
        self.skip_ws();
        let op = if self.take_op("==") {
            Some("==")
        } else if self.take_op("!=") {
            Some("!=")
        } else if self.take_op(">=") {
            Some(">=")
        } else if self.take_op("<=") {
            Some("<=")
        } else if self.take_op(">") {
            Some(">")
        } else if self.take_op("<") {
            Some("<")
        } else {
            None
        };
        let Some(op) = op else {
            // No comparison: the VALUE flows through (unified model) —
            // eval_bool coerces at its boundary, preserving old behavior.
            return Ok(left);
        };
        let right = self.parse_arith()?;
        Ok(Value::Bool(cmp(op, &left, &right)))
    }
    fn parse_arith(&mut self) -> AppResult<Value> {
        let mut v = self.parse_unary()?;
        loop {
            self.skip_ws();
            let c = self.peek();
            let op = match c {
                Some('+') => Some('+'),
                Some('-') => Some('-'),
                Some('*') => Some('*'),
                Some('/') => Some('/'),
                Some('%') => Some('%'),
                _ => None,
            };
            let Some(op) = op else { return Ok(v) };
            self.pos += 1;
            let r = self.parse_unary()?;
            // `+` stays strictly numeric (never string concat — use concat()).
            let (a, b) = (num_operand(&v)?, num_operand(&r)?);
            v = num_value(match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' => a / b,
                '%' => a % b,
                _ => unreachable!(),
            });
        }
    }
    fn parse_unary(&mut self) -> AppResult<Value> {
        self.skip_ws();
        if self.eat('!') {
            self.enter()?;
            let v = self.parse_unary()?;
            self.leave();
            return Ok(Value::Bool(!coerce_bool(&v)));
        }
        if self.eat('-') {
            // Unary minus (new): `-5` used to fail tokenization.
            self.enter()?;
            let v = self.parse_unary()?;
            self.leave();
            return num_operand(&v).map(|n| num_value(-n));
        }
        if self.eat('(') {
            // Parenthesized VALUE (fix): the old parser forced `Bool` here,
            // silently breaking `(5)==5` and nested arithmetic.
            self.enter()?;
            let v = self.parse_or()?;
            self.leave();
            if !self.eat(')') {
                return Err(AppError::BadRequest("表达式缺 ')'".into()));
            }
            return Ok(v);
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> AppResult<Value> {
        self.skip_ws();
        // Array literal `[a, b, …]` — how array refs enter the expression
        // (literal_syntax serializes them as JSON, the grammar re-parses).
        if self.eat('[') {
            self.enter()?;
            let mut items = Vec::new();
            let out = (|| {
                if self.eat(']') {
                    return Ok(());
                }
                loop {
                    items.push(self.parse_or()?);
                    if self.eat(',') {
                        continue;
                    }
                    if self.eat(']') {
                        return Ok(());
                    }
                    return Err(AppError::BadRequest("数组缺 ',' 或 ']'".into()));
                }
            })();
            self.leave();
            out?;
            return Ok(Value::Array(items));
        }
        // Quoted string: scan to the MATCHING quote (escape-aware) — the old
        // naive delimiter scan broke on spaces inside literals
        // (`== "hello world"` never parsed).
        if let Some(q) = self.peek()
            && (q == '"' || q == '\'')
        {
            self.pos += 1;
            let mut inner = String::new();
            // Decode REAL chars (byte-wise `as char` mangles CJK sequences).
            while let Some(c) = self.s[self.pos..].chars().next() {
                if c == '\\'
                    && let Some(next) = self.s[self.pos + 1..].chars().next()
                {
                    if next == 'u'
                        && let Ok(hex) = self.s[self.pos + 2..]
                            .get(..4)
                            .ok_or(())
                            .and_then(|h| u32::from_str_radix(h, 16).map_err(|_| ()))
                            .map(char::from_u32)
                        && let Some(ch) = hex
                    {
                        inner.push(ch);
                        self.pos += 6;
                        continue;
                    }
                    match next {
                        'n' => inner.push('\n'),
                        't' => inner.push('\t'),
                        'r' => inner.push('\r'),
                        // Standard escapes emitted by literal_syntax (Rust
                        // Debug / JSON) — decode, or quotes/backslashes in
                        // data would leak as `\"`/`\\` through concat.
                        '"' => inner.push('"'),
                        '\\' => inner.push('\\'),
                        // Unknown escapes keep BOTH chars: regex patterns
                        // (`\d`, `\w`) must survive verbatim.
                        other => {
                            inner.push('\\');
                            inner.push(other);
                        }
                    }
                    self.pos += 1 + next.len_utf8();
                    continue;
                }
                if c == q {
                    self.pos += 1;
                    return Ok(Value::String(inner));
                }
                inner.push(c);
                self.pos += c.len_utf8();
            }
            return Err(AppError::BadRequest("字符串缺结束引号".into()));
        }
        let start = self.pos;
        while self.pos < self.b.len() {
            let c = self.b[self.pos] as char;
            if c.is_whitespace()
                || matches!(
                    c,
                    ')' | '>'
                        | '<'
                        | '='
                        | '!'
                        | '&'
                        | '|'
                        | '+'
                        | '-'
                        | '*'
                        | '/'
                        | '%'
                        | '('
                        | ','
                        | '['
                        | ']'
                )
            {
                break;
            }
            self.pos += 1;
        }
        let tok = &self.s[start..self.pos];
        if tok.is_empty() {
            return Err(AppError::BadRequest("表达式意外结束".into()));
        }
        if tok == "true" {
            return Ok(Value::Bool(true));
        }
        if tok == "false" {
            return Ok(Value::Bool(false));
        }
        if tok == "null" {
            return Ok(Value::Null);
        }
        if let Ok(f) = tok.parse::<f64>() {
            // Integer-valued numbers become i64 (display `3`, not `3.0`).
            return Ok(num_value(f));
        }
        // Identifier followed by `(` → cleaning-function call.
        self.skip_ws();
        if self.peek() == Some('(') {
            let args = self.parse_args()?;
            return apply_function(tok, args);
        }
        Err(AppError::BadRequest(format!("表达式无法解析: {tok}")))
    }
    /// `name(arg, arg, …)` — pos sits on `(`.
    fn parse_args(&mut self) -> AppResult<Vec<Value>> {
        self.pos += 1; // consume '('
        if self.eat(')') {
            return Ok(Vec::new());
        }
        let mut args = Vec::new();
        loop {
            self.enter()?;
            let a = self.parse_or()?;
            self.leave();
            args.push(a);
            if self.eat(',') {
                continue;
            }
            if self.eat(')') {
                break;
            }
            return Err(AppError::BadRequest("函数参数缺少 ',' 或 ')'".into()));
        }
        Ok(args)
    }
}

/// Cleaning-function set (transform-node.md §2.3; [自造] minimal set — n8n
/// uses full JS / Dify jinja, neither worth wholesale copying). Scalar
/// transforms are null-passthrough so `default(to_number({{#x#}}), 0)`
/// composes; type mismatches fail loudly (BadRequest, authoring-visible).
fn apply_function(name: &str, args: Vec<Value>) -> AppResult<Value> {
    let arity = |min: usize, max: usize| -> AppResult<()> {
        if args.len() < min || args.len() > max {
            return Err(AppError::BadRequest(format!(
                "{name} 参数数量错误（{min}..={max}，收到 {}）",
                args.len()
            )));
        }
        Ok(())
    };
    let want_str = |i: usize| -> AppResult<&str> {
        args.get(i)
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::BadRequest(format!("{name} 第 {} 参数须为字符串", i + 1)))
    };
    match name {
        "concat" => {
            arity(1, usize::MAX)?;
            let mut out = String::new();
            for a in &args {
                out.push_str(&fn_to_string(a)?);
            }
            Ok(Value::String(out))
        }
        "to_number" => {
            arity(1, 1)?;
            match &args[0] {
                Value::Number(_) => Ok(args[0].clone()),
                Value::Null => Ok(Value::Null),
                Value::String(s) => s
                    .trim()
                    .parse::<f64>()
                    .map(num_value)
                    .map_err(|_| AppError::BadRequest(format!("to_number 无法解析: {s:?}"))),
                other => Err(AppError::BadRequest(format!(
                    "to_number 不支持该类型: {other}"
                ))),
            }
        }
        "to_string" => {
            arity(1, 1)?;
            fn_to_string(&args[0]).map(Value::String)
        }
        "upper" | "lower" | "trim" => {
            arity(1, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::String(match name {
                    "upper" => s.to_uppercase(),
                    "lower" => s.to_lowercase(),
                    _ => s.trim().to_string(),
                })),
                other => Err(AppError::BadRequest(format!(
                    "{name} 须为字符串，遇到 {other}"
                ))),
            }
        }
        "replace" => {
            arity(3, 3)?;
            let (s, from, to) = (want_str(0)?, want_str(1)?, want_str(2)?);
            Ok(Value::String(s.replace(from, to)))
        }
        "substring" => {
            arity(2, 3)?;
            let s = want_str(0)?;
            let start = want_int(name, &args, 1)? as usize;
            let end = match args.get(2) {
                None => s.chars().count(),
                Some(_) => want_int(name, &args, 2)?.max(0) as usize,
            };
            let chars: Vec<char> = s.chars().collect();
            let start = start.min(chars.len());
            let end = end.clamp(start, chars.len());
            Ok(Value::String(chars[start..end].iter().collect()))
        }
        "length" => {
            arity(1, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::String(s) => Ok(Value::from(s.chars().count() as i64)),
                Value::Array(xs) => Ok(Value::from(xs.len() as i64)),
                other => Err(AppError::BadRequest(format!(
                    "length 须为字符串或数组: {other}"
                ))),
            }
        }
        "if" => {
            // Conditional VALUE (transform-node.md §2.5): truthiness via the
            // legacy coercion; BOTH branches evaluate eagerly (no AST) — an
            // erroring untaken branch still fails the whole expression; the
            // null-passthrough design keeps the fallback pattern usable.
            arity(3, 3)?;
            if coerce_bool(&args[0]) {
                Ok(args[1].clone())
            } else {
                Ok(args[2].clone())
            }
        }
        "default" => {
            // null-only fallback (`??` semantics, not `||`) — empty strings
            // and 0 are VALUES and do not trigger the fallback.
            arity(2, 2)?;
            if args[0].is_null() {
                Ok(args[1].clone())
            } else {
                Ok(args[0].clone())
            }
        }
        "round" => {
            arity(1, 2)?;
            let x = num_operand(&args[0]).map_err(|_| {
                AppError::BadRequest(format!("round 第 1 参数须为数字: {0}", args[0]))
            })?;
            let digits = match args.get(1) {
                None => 0,
                Some(_) => want_int(name, &args, 1)?.clamp(0, 15),
            };
            let m = 10_f64.powi(i32::try_from(digits).unwrap_or(0));
            Ok(num_value((x * m).round() / m))
        }
        // ── arrays ──
        "first" => {
            arity(1, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Array(xs) => Ok(xs.first().cloned().unwrap_or(Value::Null)),
                other => Err(AppError::BadRequest(format!(
                    "first 须为数组，遇到 {other}"
                ))),
            }
        }
        "sum" => {
            arity(1, 1)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Array(xs) => {
                    let mut total = 0_f64;
                    for x in xs {
                        total += num_operand(x)?;
                    }
                    Ok(num_value(total))
                }
                other => Err(AppError::BadRequest(format!("sum 须为数组，遇到 {other}"))),
            }
        }
        "join" => {
            arity(1, 2)?;
            match &args[0] {
                Value::Null => Ok(Value::Null),
                Value::Array(xs) => {
                    let sep = if args.len() > 1 {
                        want_str(1)?.to_string()
                    } else {
                        String::new()
                    };
                    let mut parts = Vec::with_capacity(xs.len());
                    for x in xs {
                        parts.push(fn_to_string(x)?);
                    }
                    Ok(Value::String(parts.join(&sep)))
                }
                other => Err(AppError::BadRequest(format!("join 须为数组，遇到 {other}"))),
            }
        }
        // ── dates (UTC-only v1; output format matches platform Timestamp
        // serde so lexicographic comparisons stay order-correct) ──
        "now" => {
            arity(0, 0)?;
            Ok(Value::String(fmt_ts(crate::utils::tz::now_utc())))
        }
        "date_add" => {
            arity(3, 3)?;
            let ts = parse_ts(want_str(0)?)?;
            let n = want_int(name, &args, 1)?;
            let dur = match want_str(2)? {
                "days" => chrono::Duration::days(n),
                "hours" => chrono::Duration::hours(n),
                "minutes" => chrono::Duration::minutes(n),
                "seconds" => chrono::Duration::seconds(n),
                other => {
                    return Err(AppError::BadRequest(format!(
                        "date_add 单位须为 days/hours/minutes/seconds，收到 {other}"
                    )));
                }
            };
            Ok(Value::String(fmt_ts(ts + dur)))
        }
        "format_date" => {
            arity(2, 2)?;
            let ts = parse_ts(want_str(0)?)?;
            // luxon-style tokens (n8n `.toFormat` same family).
            let pat = want_str(1)?
                .replace("yyyy", "%Y")
                .replace("MM", "%m")
                .replace("dd", "%d")
                .replace("HH", "%H")
                .replace("mm", "%M")
                .replace("ss", "%S");
            Ok(Value::String(ts.format(&pat).to_string()))
        }
        // ── regex (RE2-lineage crate: linear time, no catastrophic
        // backtracking — safe to expose on the admin surface) ──
        "regex_test" => {
            arity(2, 2)?;
            let re = compile_re(want_str(1)?)?;
            Ok(Value::Bool(re.is_match(want_str(0)?)))
        }
        "regex_extract" => {
            arity(2, 2)?;
            let re = compile_re(want_str(1)?)?;
            match re.captures(want_str(0)?) {
                None => Ok(Value::Null),
                Some(c) => {
                    // ≥1 capture group → first group; else the whole match.
                    let m = if c.len() > 1 { c.get(1) } else { c.get(0) };
                    Ok(m.map(|x| Value::String(x.as_str().to_string()))
                        .unwrap_or(Value::Null))
                }
            }
        }
        "regex_replace" => {
            // Replace ALL occurrences; replacement supports $1 / ${1}.
            arity(3, 3)?;
            let re = compile_re(want_str(1)?)?;
            let out = re.replace_all(want_str(0)?, want_str(2)?).to_string();
            Ok(Value::String(out))
        }
        other => Err(AppError::BadRequest(format!(
            "未知函数: {other}（可用: concat to_number to_string upper lower trim replace substring length default round if first sum join now date_add format_date regex_test regex_extract regex_replace）"
        ))),
    }
}

fn fn_to_string(v: &Value) -> AppResult<String> {
    Ok(match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => {
            return Err(AppError::BadRequest(format!(
                "to_string 不支持该类型: {other}"
            )));
        }
    })
}

fn want_int(name: &str, args: &[Value], i: usize) -> AppResult<i64> {
    args.get(i)
        .and_then(Value::as_i64)
        .ok_or_else(|| AppError::BadRequest(format!("{name} 第 {} 参数须为整数", i + 1)))
}

fn cmp(op: &str, l: &Value, r: &Value) -> bool {
    match (l.as_f64(), r.as_f64()) {
        (Some(a), Some(b)) => match op {
            ">" => a > b,
            ">=" => a >= b,
            "<" => a < b,
            "<=" => a <= b,
            "==" => a == b,
            "!=" => a != b,
            _ => false,
        },
        // String ordering (lexicographic): same-format RFC3339 timestamps
        // compare correctly — documented ISO-compare semantics.
        _ if l.is_string() && r.is_string() => {
            let (a, b) = (l.as_str().unwrap_or(""), r.as_str().unwrap_or(""));
            match op {
                ">" => a > b,
                ">=" => a >= b,
                "<" => a < b,
                "<=" => a <= b,
                "==" => a == b,
                "!=" => a != b,
                _ => false,
            }
        }
        _ => match op {
            "==" => l == r,
            "!=" => l != r,
            _ => false,
        },
    }
}

/// Keep HashMap referenced so future token-cache work compiles cleanly.
#[allow(dead_code)]
fn _unused(_: HashMap<String, Value>) {}

/// Platform-aligned RFC3339 formatting (chrono serde uses AutoSi + Z):
/// keeps `now()`/`date_add()` outputs order-correct under string compare.
fn fmt_ts(ts: crate::utils::tz::Timestamp) -> String {
    ts.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
}

fn parse_ts(s: &str) -> AppResult<crate::utils::tz::Timestamp> {
    crate::utils::tz::parse_rfc3339(s)
        .map_err(|e| AppError::BadRequest(format!("日期须为 RFC3339（{s:?}）: {e}")))
}

fn compile_re(pat: &str) -> AppResult<regex::Regex> {
    regex::Regex::new(pat).map_err(|e| AppError::BadRequest(format!("正则非法（{pat:?}）: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn string_literals_with_spaces_and_escapes() {
        let p = pool_of("a", "msg", json!("hello world"));
        // Spaces inside quoted literals never parsed before (pre-existing
        // tokenizer bug, surfaced by the unified characterization suite).
        assert!(eval_value("{{#a.msg#}} == \"hello world\"", &p).unwrap() == json!(true));
        assert_eq!(
            eval_value("concat(\"a b\", \" c\")", &p).unwrap(),
            json!("a b c")
        );
        // Refs containing quotes escape cleanly through literal_syntax.
        let q = pool_of("a", "msg", json!("say \"hi\""));
        assert!(eval_value("{{#a.msg#}} == \"say \\\"hi\\\"\"", &q).unwrap() == json!(true));
        // Quotes/backslashes in data must NOT leak as `\"`/`\\` through
        // concat (standard escapes decode; only unknown ones stay verbatim).
        assert_eq!(
            eval_value("concat({{#a.msg#}}, \"!\")", &q).unwrap(),
            json!("say \"hi\"!")
        );
        let b = pool_of("a", "msg", json!("C:\\path"));
        assert_eq!(
            eval_value("concat({{#a.msg#}}, \"/x\")", &b).unwrap(),
            json!("C:\\path/x")
        );
    }

    #[test]
    fn unified_value_model_characterization() {
        let p = pool_of("classify", "level", json!(3));
        // (x) now returns the inner VALUE — the old bool-forcing parser
        // answered `false` for both of these (characterization flip).
        assert_eq!(eval_value("(5) == 5", &p).unwrap(), json!(true));
        assert_eq!(eval_value("(1 + 2) * 3", &p).unwrap(), json!(9));
        // Bare value flows through; eval_bool keeps legacy truthiness.
        assert_eq!(eval_value("1 + 2", &p).unwrap(), json!(3));
        assert!(eval_bool("1 + 2", &p).unwrap());
        assert_eq!(eval_value("-5 + 1", &p).unwrap(), json!(-4));
        assert_eq!(eval_value("null", &p).unwrap(), Value::Null);
        // Div-by-zero keeps its historical null result.
        assert_eq!(eval_value("1 / 0", &p).unwrap(), Value::Null);
        // Integer-valued floats display as integers (concat/to_string feed).
        assert_eq!(eval_value("6 / 2", &p).unwrap(), json!(3));
    }

    #[test]
    fn cleaning_functions() {
        let p = pool_of("start", "name", json!(" Bo Zhang "));
        let mut p = p;
        let mut m = p.remove("start").unwrap_or_default();
        m.insert("missing".to_string(), Value::Null); // skip-null semantics
        p.insert("start".to_string(), m);
        assert_eq!(
            eval_value("concat(trim({{#start.name#}}), \"-001\")", &p).unwrap(),
            json!("Bo Zhang-001")
        );
        assert_eq!(
            eval_value("upper(default({{#start.missing#}}, \"n/a\"))", &p).unwrap(),
            json!("N/A")
        );
        assert_eq!(
            eval_value("to_number(\" 42.5 \")", &p).unwrap(),
            json!(42.5)
        );
        assert_eq!(
            eval_value("to_number({{#start.missing#}})", &p).unwrap(),
            Value::Null
        );
        assert_eq!(
            eval_value("default(to_number({{#start.missing#}}), 0)", &p).unwrap(),
            json!(0)
        );
        // replace substitutes EVERY occurrence (documented semantics).
        assert_eq!(
            eval_value("replace(\"a-b-a\", \"a\", \"x\")", &p).unwrap(),
            json!("x-b-x")
        );
        // 脱敏: mask the first 3 chars of a number whose prefix occurs once.
        assert_eq!(
            eval_value(
                "concat(replace(\"139-0013\", substring(\"139-0013\", 0, 3), \"***\"), \"-ok\")",
                &p
            )
            .unwrap(),
            json!("***-0013-ok")
        );
        assert_eq!(eval_value("length(\"订单号A1\")", &p).unwrap(), json!(5));
        assert_eq!(eval_value("round(2.345, 2)", &p).unwrap(), json!(2.35));
        assert_eq!(eval_value("round(2.5)", &p).unwrap(), json!(3));
        assert_eq!(eval_value("to_string(3)", &p).unwrap(), json!("3"));
        // default is null-only: "" and 0 are values, not fallbacks.
        assert_eq!(eval_value("default(0, 9)", &p).unwrap(), json!(0));
        assert_eq!(eval_value("default(\"\", \"x\")", &p).unwrap(), json!(""));
    }

    #[test]
    fn conditional_values_with_if() {
        let mut p = Pool::new();
        let mut m = HashMap::new();
        m.insert("score".to_string(), json!(95));
        m.insert("low".to_string(), json!(60));
        m.insert("opt".to_string(), Value::Null);
        p.insert("s".to_string(), m);
        // Binary classification.
        assert_eq!(
            eval_value("if({{#s.score#}} > 80, \"A\", \"B\")", &p).unwrap(),
            json!("A")
        );
        assert_eq!(
            eval_value("if({{#s.low#}} > 80, \"A\", \"B\")", &p).unwrap(),
            json!("B")
        );
        // Nested tiers (right-nested else chains).
        assert_eq!(
            eval_value(
                "if({{#s.low#}} > 90, \"A\", if({{#s.low#}} > 80, \"B\", \"C\"))",
                &p
            )
            .unwrap(),
            json!("C")
        );
        // Null-fallback synergy: to_number(null) passes null through instead
        // of erroring, so the guard pattern works despite eager evaluation.
        assert_eq!(
            eval_value("if({{#s.opt#}} != null, to_number({{#s.opt#}}), 0)", &p).unwrap(),
            json!(0)
        );
        // Eager evaluation caveat: the UNtaken branch still evaluates — its
        // type error fails the whole expression (documented semantics).
        assert!(eval_value("if(true, 1, to_number(\"x\"))", &p).is_err());
    }

    #[test]
    fn array_expressions() {
        let mut p = Pool::new();
        let mut m = HashMap::new();
        m.insert("items".to_string(), json!([10, 20, 30]));
        m.insert("tags".to_string(), json!(["a", "b"]));
        m.insert("empty".to_string(), json!([]));
        p.insert("s".to_string(), m);
        // Literal arrays.
        assert_eq!(eval_value("sum([1, 2, 3])", &p).unwrap(), json!(6));
        assert_eq!(eval_value("first([9, 8])", &p).unwrap(), json!(9));
        // Array refs enter via literal_syntax (JSON round-trip).
        assert_eq!(eval_value("sum({{#s.items#}})", &p).unwrap(), json!(60));
        assert_eq!(
            eval_value("join({{#s.tags#}}, \"-\")", &p).unwrap(),
            json!("a-b")
        );
        assert_eq!(eval_value("length({{#s.items#}})", &p).unwrap(), json!(3));
        assert_eq!(eval_value("first({{#s.empty#}})", &p).unwrap(), Value::Null);
        assert_eq!(eval_value("join({{#s.empty#}})", &p).unwrap(), json!(""));
        // Composes with arithmetic and if().
        assert_eq!(
            eval_value("if(sum({{#s.items#}}) > 50, \"big\", \"small\")", &p).unwrap(),
            json!("big")
        );
        // Non-number items fail loudly.
        assert!(eval_value("sum([1, \"x\"])", &p).is_err());
    }

    #[test]
    fn date_functions() {
        let p = Pool::new();
        // now() round-trips through the platform parser (format alignment).
        let now = match eval_value("now()", &p).unwrap() {
            Value::String(s) => s,
            other => panic!("now() must be a string: {other}"),
        };
        assert!(
            crate::utils::tz::parse_rfc3339(&now).is_ok(),
            "format: {now}"
        );
        // Lexicographic order still holds for same-format outputs (1 day ahead > now).
        assert!(eval_bool("date_add(now(), 1, \"days\") > now()", &p).unwrap());
        assert!(eval_bool("date_add(now(), -1, \"days\") < now()", &p).unwrap());
        // Arithmetic + formatting.
        assert_eq!(
            eval_value(
                "format_date(date_add(\"2026-09-07T10:00:00Z\", 90, \"minutes\"), \"yyyy-MM-dd HH:mm\")",
                &p
            )
            .unwrap(),
            json!("2026-09-07 11:30")
        );
        assert!(eval_value("date_add(now(), 1, \"weeks\")", &p).is_err());
        assert!(eval_value("date_add(\"not-a-date\", 1, \"days\")", &p).is_err());
    }

    #[test]
    fn regex_functions() {
        let p = Pool::new();
        assert_eq!(
            eval_value("regex_test(\"ORD-2026-001\", \"^ORD-\\d+-\\d+$\")", &p).unwrap(),
            json!(true)
        );
        assert_eq!(
            eval_value("regex_test(\"hello\", \"^\\d+$\")", &p).unwrap(),
            json!(false)
        );
        // Capture group → first group; no match → null (composable with default).
        assert_eq!(
            eval_value("regex_extract(\"订单 ORD-123 已支付\", \"ORD-(\\d+)\")", &p).unwrap(),
            json!("123")
        );
        assert_eq!(
            eval_value(
                "default(regex_extract(\"无单号\", \"ORD-(\\d+)\"), \"-\")",
                &p
            )
            .unwrap(),
            json!("-")
        );
        assert_eq!(
            eval_value("regex_replace(\"a1b22c\", \"\\d+\", \"#\")", &p).unwrap(),
            json!("a#b#c")
        );
        // $1 backreference in replacement.
        assert_eq!(
            eval_value(
                "regex_replace(\"2026-09-07\", \"(\\d+)-(\\d+)-(\\d+)\", \"$3/$2/$1\")",
                &p
            )
            .unwrap(),
            json!("07/09/2026")
        );
        assert!(eval_value("regex_replace(\"x\", \"[\", \"y\")", &p).is_err());
    }

    #[test]
    fn function_and_parser_errors() {
        let p = pool_of("a", "x", json!(1));
        assert!(eval_value("nosuch(1)", &p).is_err(), "未知函数");
        assert!(eval_value("upper(1)", &p).is_err(), "类型错误");
        assert!(eval_value("default(1)", &p).is_err(), "参数数量");
        assert!(eval_value("5 $", &p).is_err(), "尾部垃圾");
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert!(eval_value(&deep, &p).is_err(), "嵌套过深");
        let long = "1 + ".repeat(2000) + "1";
        assert!(eval_value(&long, &p).is_err(), "超长");
    }

    #[test]
    fn ref_null_flows_into_expr() {
        let mut p = Pool::new();
        let mut m = HashMap::new();
        m.insert("flag".to_string(), Value::Null);
        p.insert("skipped".to_string(), m);
        // Skipped-branch refs are explicit nulls (D6): default picks it up.
        assert_eq!(
            eval_value("default({{#skipped.flag#}}, \"none\")", &p).unwrap(),
            json!("none")
        );
    }

    fn pool_of(ns: &str, name: &str, v: Value) -> Pool {
        let mut p = Pool::new();
        let mut m = HashMap::new();
        m.insert(name.to_string(), v);
        p.insert(ns.to_string(), m);
        p
    }

    #[test]
    fn text_whole_ref_is_typed() {
        let p = pool_of("start", "n", json!(42));
        assert_eq!(resolve_text("{{#start.n#}}", &p).unwrap(), json!(42));
    }

    #[test]
    fn text_interpolates_inline() {
        let p = pool_of("start", "name", json!("alice"));
        assert_eq!(
            resolve_text("hi {{#start.name#}}!", &p).unwrap(),
            json!("hi alice!")
        );
    }

    #[test]
    fn bool_expr_with_refs() {
        let p = pool_of("classify", "level", json!(5));
        assert!(eval_bool("{{#classify.level#}} >= 3", &p).unwrap());
        assert!(!eval_bool("{{#classify.level#}} >= 3 && false", &p).unwrap());
    }

    #[test]
    fn bool_expr_string_eq() {
        let p = pool_of("start", "s", json!("hi"));
        assert!(eval_bool("{{#start.s#}} == \"hi\"", &p).unwrap());
        assert!(eval_bool("{{#start.s#}} != \"x\"", &p).unwrap());
    }
}
