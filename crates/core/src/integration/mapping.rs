//! Declarative field mapping — the zero-code L2 normalizer (integration.md §7.1).
//!
//! A deliberately small, whitelist-syntax evaluator: JSONPath-style access
//! (`$.a.b[0]`), `const:` literals and four pipe functions
//! (`as_number`, `as_datetime`, `default(v)`, `regex(pattern)`).
//! Anything more complex should be a normalizer plugin, and the error
//! messages say so.

use serde_json::{Map, Value};

use crate::errors::app_error::AppError;
use crate::integration::envelope::InboundKind;

/// Normalized output of one mapping application.
#[derive(Debug, Clone)]
pub struct Normalized {
    /// Provider-side unique id — required (the idempotency key).
    pub external_id: String,
    pub sender: Option<String>,
    pub kind: InboundKind,
    /// Mapped payload object (written to the target CT).
    pub payload: Value,
}

/// Compiled mapping plan (built once per channel, cached by the pipeline).
#[derive(Debug, Clone)]
pub struct MappingPlan {
    external_id: Expr,
    sender: Option<Expr>,
    kind: InboundKind,
    payload_rules: Vec<(String, Expr)>,
    when: Option<WhenCond>,
}

/// One mapping expression: path/const + optional pipe chain.
#[derive(Debug, Clone)]
struct Expr {
    kind: ExprKind,
    pipes: Vec<Pipe>,
}

#[derive(Debug, Clone)]
enum ExprKind {
    Path(Vec<Segment>),
    Const(Value),
}

#[derive(Debug, Clone)]
enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Debug, Clone)]
enum Pipe {
    AsNumber,
    AsDatetime,
    Default(Value),
    Regex(String),
    AsJson(Option<String>),
}

/// `when` condition — M1 supports `{ "all": [ {"==": [expr, literal]}, ... ] }`
/// with `==` / `!=` only.
#[derive(Debug, Clone)]
struct WhenCond {
    all: Vec<(Expr, Value, bool)>, // (expr, literal, expect_equal)
}

// ── Compile ──────────────────────────────────────────────────────────

/// Compile a mapping definition (`itg_channels.mapping` JSON).
///
/// # Errors
///
/// `BadRequest` with actionable guidance on any unsupported syntax.
pub fn compile(mapping: &Value) -> Result<MappingPlan, AppError> {
    let obj = mapping
        .as_object()
        .ok_or_else(|| AppError::BadRequest("mapping must be a JSON object".into()))?;

    let external_id = obj
        .get("external_id")
        .ok_or_else(|| AppError::BadRequest("mapping requires 'external_id'".into()))
        .and_then(compile_expr)?;

    let sender = match obj.get("sender") {
        None => None,
        Some(v) if v.is_null() => None,
        Some(v) => Some(compile_expr(v)?),
    };

    let kind = match obj.get("kind") {
        None => InboundKind::Event,
        Some(v) => {
            let s = v.as_str().unwrap_or_default();
            let wire = s.strip_prefix("const:").unwrap_or(s);
            InboundKind::from_wire(&wire.to_ascii_lowercase()).ok_or_else(|| {
                AppError::BadRequest(format!(
                    "unknown kind '{wire}' (message|event|callback|telemetry|connection_state)"
                ))
            })?
        }
    };

    let payload_rules = match obj.get("payload") {
        None => Vec::new(),
        Some(Value::Object(rules)) => {
            let mut compiled = Vec::new();
            for (target, expr) in rules {
                compiled.push((target.clone(), compile_expr(expr)?));
            }
            compiled
        }
        Some(_) => {
            return Err(AppError::BadRequest(
                "mapping 'payload' must be an object of {target: expr}".into(),
            ));
        }
    };

    let when = match obj.get("when") {
        None => None,
        Some(v) => Some(compile_when(v)?),
    };

    Ok(MappingPlan {
        external_id,
        sender,
        kind,
        payload_rules,
        when,
    })
}

fn compile_when(v: &Value) -> Result<WhenCond, AppError> {
    let all = v
        .get("all")
        .and_then(Value::as_array)
        .ok_or_else(|| AppError::BadRequest("when requires {\"all\": [...]}".into()))?;
    let mut conds = Vec::new();
    for cond in all {
        let obj = cond
            .as_object()
            .ok_or_else(|| AppError::BadRequest("when entries must be objects".into()))?;
        for (op, args) in obj {
            let expect_equal = match op.as_str() {
                "==" => true,
                "!=" => false,
                other => {
                    return Err(AppError::BadRequest(format!(
                        "when operator '{other}' not supported (== / != only) — \
                         complex filtering belongs in a normalizer plugin"
                    )));
                }
            };
            let arr = args
                .as_array()
                .ok_or_else(|| AppError::BadRequest(format!("'{op}' expects [expr, literal]")))?;
            if arr.len() != 2 {
                return Err(AppError::BadRequest(format!(
                    "'{op}' expects exactly 2 args"
                )));
            }
            conds.push((compile_expr(&arr[0])?, arr[1].clone(), expect_equal));
        }
    }
    Ok(WhenCond { all: conds })
}

fn compile_expr(v: &Value) -> Result<Expr, AppError> {
    let Some(source) = v.as_str() else {
        // Non-string values are inline constants (numbers, bools, nested objects).
        return Ok(Expr {
            kind: ExprKind::Const(v.clone()),
            pipes: Vec::new(),
        });
    };

    let (kind_src, pipes_src) = match source.split_once('|') {
        None => (source, ""),
        Some((k, rest)) => (k, rest),
    };

    let kind = if let Some(lit) = kind_src.trim().strip_prefix("const:") {
        ExprKind::Const(parse_const(lit))
    } else if let Some(path) = kind_src.trim().strip_prefix("$.") {
        ExprKind::Path(parse_path(path)?)
    } else if kind_src.trim() == "$" {
        ExprKind::Path(Vec::new())
    } else {
        return Err(AppError::BadRequest(format!(
            "mapping expr must start with '$.' or 'const:' — got '{kind_src}' \
             (dynamic logic belongs in a normalizer plugin)"
        )));
    };

    let mut pipes = Vec::new();
    for part in pipes_src.split('|') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let pipe = match part {
            "as_number" => Pipe::AsNumber,
            "as_datetime" => Pipe::AsDatetime,
            "as_json" => Pipe::AsJson(None),
            _ if part.starts_with("as_json(") && part.ends_with(')') => {
                let inner = &part[9..part.len() - 1];
                Pipe::AsJson(Some(inner.to_string()))
            }
            _ if part.starts_with("default(") && part.ends_with(')') => {
                let inner = &part[8..part.len() - 1];
                Pipe::Default(parse_const(inner))
            }
            _ if part.starts_with("regex(") && part.ends_with(')') => {
                Pipe::Regex(part[6..part.len() - 1].to_string())
            }
            other => {
                return Err(AppError::BadRequest(format!(
                    "pipe function '{other}' not supported \
                     (as_number | as_datetime | default(v) | regex(pattern))"
                )));
            }
        };
        pipes.push(pipe);
    }

    Ok(Expr { kind, pipes })
}

fn parse_const(lit: &str) -> Value {
    // Strip surrounding double quotes so `default("(empty)")` reads as (empty).
    let lit = if lit.len() >= 2 && lit.starts_with('"') && lit.ends_with('"') {
        &lit[1..lit.len() - 1]
    } else {
        lit
    };
    if lit == "true" {
        return Value::Bool(true);
    }
    if lit == "false" {
        return Value::Bool(false);
    }
    if lit == "null" {
        return Value::Null;
    }
    if let Ok(n) = lit.parse::<i64>() {
        return Value::from(n);
    }
    if let Ok(f) = lit.parse::<f64>() {
        return Value::from(f);
    }
    Value::String(lit.to_string())
}

fn parse_path(path: &str) -> Result<Vec<Segment>, AppError> {
    let mut segments = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('[') {
            let Some(end) = after.find(']') else {
                return Err(AppError::BadRequest(format!(
                    "malformed path segment '{rest}' (unterminated [index])"
                )));
            };
            let idx_src = &after[..end];
            let idx = idx_src.parse::<usize>().map_err(|_| {
                AppError::BadRequest(format!(
                    "array index must be a number — got '[{idx_src}]' \
                     (slices/filters belong in a normalizer plugin)"
                ))
            })?;
            segments.push(Segment::Index(idx));
            rest = &after[end + 1..];
            if rest.starts_with('.') {
                rest = &rest[1..];
            }
        } else {
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            let key = &rest[..end];
            if key.is_empty() {
                return Err(AppError::BadRequest(format!(
                    "malformed path '{path}' (empty segment)"
                )));
            }
            segments.push(Segment::Key(key.to_string()));
            rest = &rest[end..];
            if rest.starts_with('.') {
                rest = &rest[1..];
            }
        }
    }
    Ok(segments)
}

// ── Apply ────────────────────────────────────────────────────────────

impl MappingPlan {
    /// Apply to the framing-decoded input. Returns [`None`] when the `when`
    /// condition does not match (envelope should be skipped).
    ///
    /// # Errors
    ///
    /// `BadRequest` when required values are missing or pipes fail.
    pub fn apply(&self, input: &Value) -> Result<Option<Normalized>, AppError> {
        if let Some(when) = &self.when
            && !when.matches(input)?
        {
            return Ok(None);
        }

        let external_id =
            eval_to_string(&self.external_id, input, "external_id")?.ok_or_else(|| {
                AppError::BadRequest("mapping 'external_id' resolved to nothing".into())
            })?;
        if external_id.is_empty() {
            return Err(AppError::BadRequest(
                "mapping 'external_id' resolved to an empty string".into(),
            ));
        }

        let sender = match &self.sender {
            Some(expr) => eval_to_string(expr, input, "sender")?,
            None => None,
        };

        let mut payload = Map::new();
        for (target, expr) in &self.payload_rules {
            payload.insert(target.clone(), eval_value(expr, input)?);
        }

        Ok(Some(Normalized {
            external_id,
            sender,
            kind: self.kind,
            payload: Value::Object(payload),
        }))
    }
}

impl WhenCond {
    fn matches(&self, input: &Value) -> Result<bool, AppError> {
        for (expr, literal, expect_equal) in &self.all {
            let actual = eval_value(expr, input)?;
            let equal = &actual == literal;
            if equal != *expect_equal {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

fn eval_to_string(expr: &Expr, input: &Value, field: &str) -> Result<Option<String>, AppError> {
    let v = eval_value(expr, input)?;
    match v {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s)),
        Value::Number(n) => Ok(Some(n.to_string())),
        Value::Bool(b) => Ok(Some(b.to_string())),
        _ => Err(AppError::BadRequest(format!(
            "mapping '{field}' must resolve to a scalar"
        ))),
    }
}

fn eval_value(expr: &Expr, input: &Value) -> Result<Value, AppError> {
    let mut v = match &expr.kind {
        ExprKind::Const(c) => c.clone(),
        ExprKind::Path(segments) => {
            let mut cur = input;
            for seg in segments {
                cur = match (seg, cur) {
                    (Segment::Key(k), Value::Object(o)) => o.get(k).unwrap_or(&Value::Null),
                    (Segment::Index(i), Value::Array(a)) => a.get(*i).unwrap_or(&Value::Null),
                    _ => &Value::Null,
                };
            }
            cur.clone()
        }
    };
    for pipe in &expr.pipes {
        v = apply_pipe(pipe, v)?;
    }
    Ok(v)
}

fn apply_pipe(pipe: &Pipe, v: Value) -> Result<Value, AppError> {
    match pipe {
        Pipe::AsNumber => {
            let n = match &v {
                Value::Number(_) => return Ok(v),
                Value::String(s) => s.parse::<f64>().map_err(|_| {
                    AppError::BadRequest(format!("as_number: '{s}' is not numeric"))
                })?,
                _ => {
                    return Err(AppError::BadRequest(
                        "as_number: value is neither number nor numeric string".into(),
                    ));
                }
            };
            if let Some(i) = n_as_i64(n) {
                Ok(Value::from(i))
            } else {
                Ok(Value::from(n))
            }
        }
        Pipe::AsJson(sub_path) => {
            // Escaped payloads: a JSON *string* holding JSON. The optional
            // sub-path (`as_json($.text)`) digs one field out of the parsed
            // value — string-in-string envelopes (IM message content, …).
            let text = v
                .as_str()
                .ok_or_else(|| AppError::BadRequest("as_json: value is not a string".into()))?;
            let parsed: Value = serde_json::from_str(text).map_err(|e| {
                AppError::BadRequest(format!("as_json: '{text}' is not valid JSON: {e}"))
            })?;
            match sub_path {
                None => Ok(parsed),
                Some(path) => {
                    let mut cur = &parsed;
                    for key in path.strip_prefix("$.").unwrap_or(path).split('.') {
                        if key.is_empty() {
                            continue;
                        }
                        cur = cur.get(key).unwrap_or(&Value::Null);
                    }
                    Ok(cur.clone())
                }
            }
        }
        Pipe::AsDatetime => {
            // Validates RFC3339-ish input; passes through unchanged.
            let s = v
                .as_str()
                .ok_or_else(|| AppError::BadRequest("as_datetime: value is not a string".into()))?;
            crate::utils::tz::parse_rfc3339(s)
                .map_err(|e| AppError::BadRequest(format!("as_datetime: {e}")))?;
            Ok(v)
        }
        Pipe::Default(fallback) => {
            if v.is_null() {
                Ok(fallback.clone())
            } else {
                Ok(v)
            }
        }
        Pipe::Regex(pattern) => {
            let s = v.as_str().map(str::to_string).unwrap_or_default();
            let re = regex::Regex::new(pattern).map_err(|e| {
                AppError::BadRequest(format!("regex: invalid pattern '{pattern}': {e}"))
            })?;
            if let Some(caps) = re.captures(&s) {
                // First capture group if present, otherwise the full match.
                let m = caps.get(1).or_else(|| caps.get(0));
                Ok(Value::String(
                    m.map(|x| x.as_str().to_string()).unwrap_or(s),
                ))
            } else {
                Ok(Value::Null)
            }
        }
    }
}

fn n_as_i64(n: f64) -> Option<i64> {
    if n.fract() == 0.0 && n.abs() < 9.007_199_254_740_992e15 {
        Some(n as i64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn compile_ok(mapping: Value) -> MappingPlan {
        compile(&mapping).expect("compile")
    }

    #[test]
    fn basic_mapping_and_pipes() {
        let plan = compile_ok(json!({
            "external_id": "$.id | as_number | default(0)",
            "sender": "$.from.open_id",
            "kind": "const:Message",
            "payload": {
                "body": "$.data.text | default(\"(empty)\")",
                "n": "$.count | as_number",
                "order_no": "$.ref | regex(#([A-Z]+\\d+))",
                "fixed": "const:yes",
                "num": "const:42"
            },
            "when": {"all": [{"==": ["$.type", "message"]}]}
        }));
        let input = json!({
            "id": "1001", "type": "message",
            "from": {"open_id": "u1"},
            "data": {"text": null},
            "count": "7",
            "ref": "ref#AB1234 tail"
        });
        let out = plan.apply(&input).expect("apply").expect("matched");
        assert_eq!(out.external_id, "1001");
        assert_eq!(out.sender.as_deref(), Some("u1"));
        assert_eq!(out.kind, InboundKind::Message);
        assert_eq!(out.payload["body"], "(empty)");
        assert_eq!(out.payload["n"], 7);
        assert_eq!(out.payload["order_no"], "AB1234");
        assert_eq!(out.payload["fixed"], "yes");
        assert_eq!(out.payload["num"], 42);
    }

    #[test]
    fn when_not_matching_returns_none() {
        let plan = compile_ok(json!({
            "external_id": "$.id",
            "payload": {},
            "when": {"all": [{"!=": ["$.type", "message"]}]}
        }));
        let out = plan
            .apply(&json!({"id": 1, "type": "message"}))
            .expect("apply");
        assert!(out.is_none());
    }

    #[test]
    fn array_index_path() {
        let plan = compile_ok(json!({"external_id": "$.items[0].id", "payload": {}}));
        let out = plan
            .apply(&json!({"items": [{"id": "a1"}]}))
            .expect("apply")
            .expect("matched");
        assert_eq!(out.external_id, "a1");
    }

    #[test]
    fn missing_scalar_is_null_sender() {
        let plan = compile_ok(json!({"external_id": "$.id", "sender": "$.ghost", "payload": {}}));
        let out = plan
            .apply(&json!({"id": 5}))
            .expect("apply")
            .expect("matched");
        assert_eq!(out.external_id, "5");
        assert!(out.sender.is_none());
    }

    #[test]
    fn syntax_errors_guide_to_plugin() {
        let err = compile(&json!({"external_id": "$..wildcard"})).expect_err("wildcard");
        assert!(
            err.to_string().contains("normalizer plugin") || err.to_string().contains("segment")
        );

        let err = compile(&json!({"external_id": "$.id | upper"})).expect_err("bad pipe");
        assert!(err.to_string().contains("not supported"));

        let err = compile(&json!({"payload": {}})).expect_err("no external_id");
        assert!(err.to_string().contains("external_id"));
    }

    #[test]
    fn as_json_pipe_parses_and_digs() {
        // Escaped-JSON envelope (IM-style message content).
        let plan = compile_ok(json!({
            "external_id": "$.id",
            "payload": {
                "whole": "$.content | as_json",
                "text": "$.content | as_json($.text)",
                "nested": "$.content | as_json($.meta.lang)"
            }
        }));
        // After one layer of JSON parsing the field VALUE is clean JSON
        // (the backslashes lived in the outer wire document).
        let content = r#"{"text":"hi","meta":{"lang":"zh"}}"#.to_string();
        let out = plan
            .apply(&json!({"id": "e1", "content": content}))
            .unwrap()
            .unwrap();
        assert_eq!(out.payload["text"], "hi");
        assert_eq!(out.payload["nested"], "zh");
        assert_eq!(out.payload["whole"]["meta"]["lang"], "zh");

        // Non-JSON string → compile fine, apply fails loudly.
        let err = plan
            .apply(&json!({"id": "e2", "content": "oops"}))
            .expect_err("bad json");
        assert!(err.to_string().contains("as_json"));
    }

    #[test]
    fn bad_datetime_rejected() {
        let plan =
            compile_ok(json!({"external_id": "$.id", "payload": {"t": "$.ts | as_datetime"}}));
        let err = plan
            .apply(&json!({"id": 1, "ts": "not-a-date"}))
            .expect_err("bad date");
        assert!(err.to_string().contains("as_datetime"));
    }
}
