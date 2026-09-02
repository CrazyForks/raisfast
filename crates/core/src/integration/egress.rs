//! L5 Egress — declarative api-client executor (integration.md §9).
//!
//! One call shape for every outbound HTTP need (LLM gateways, webhook
//! delivery, third-party APIs): resolve op template → render path/body →
//! inject sealed auth → fixed-window rate limit → single HTTP call with
//! timeout → log to `itg_egress_log` with the ambient trace id.
//!
//! No automatic retries by design (mvp-plan D2): callers (job handlers)
//! already carry their own retry semantics.

use std::collections::HashMap;

use serde_json::Value;

use crate::config::app::IntegrationConfig;
use crate::db::driver::DbDriver;
use crate::errors::app_error::{AppError, AppResult};
use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

use super::api_client::{self, ItgApiClient};
use super::vault::Vault;

/// One outbound call request.
#[derive(Debug, Clone)]
pub struct EgressRequest {
    /// Target api-client (`itg_api_clients.client_key`).
    pub client_key: String,
    /// Op name inside the client's `ops` map.
    pub op: String,
    /// Op input: renders `{var}` path placeholders and becomes the JSON body
    /// for write methods.
    pub input: Value,
    /// Explicit trace id override; `None` → ambient `TRACE_CTX`.
    pub trace_id: Option<i64>,
}

impl EgressRequest {
    /// Shorthand constructor.
    #[must_use]
    pub fn new(client_key: impl Into<String>, op: impl Into<String>, input: Value) -> Self {
        Self {
            client_key: client_key.into(),
            op: op.into(),
            input,
            trace_id: None,
        }
    }

    /// Attach an explicit trace id.
    #[must_use]
    pub fn with_trace(mut self, trace_id: i64) -> Self {
        self.trace_id = Some(trace_id);
        self
    }
}

/// Successful (2xx) call result.
#[derive(Debug, Clone)]
pub struct EgressReceipt {
    pub log_id: SnowflakeId,
    pub status: u16,
    /// Parsed JSON body, or `Value::String` when the body was not JSON.
    pub body: Value,
    /// Op `output` mapping applied to the body (identity when unset).
    pub output: Value,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub model: Option<String>,
}

/// Egress log row (admin list view).
#[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct EgressLogRow {
    pub id: SnowflakeId,
    pub trace_id: Option<SnowflakeId>,
    pub client_key: String,
    pub op: String,
    pub status: String,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub http_status: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub latency_ms: i64,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub tokens_in: Option<i64>,
    #[cfg_attr(feature = "export-types", ts(type = "number"))]
    pub tokens_out: Option<i64>,
    pub model: Option<String>,
    pub error: Option<String>,
    pub response_summary: Option<String>,
    pub created_at: Timestamp,
}

/// Per-client fixed-window limiter (`rate_limit.per_minute`, default 120).
#[derive(Default)]
struct ClientRateLimiter {
    inner: dashmap::DashMap<
        String,
        crate::middleware::rate_limit::RateLimiter<crate::middleware::rate_limit::MemoryStore>,
    >,
}

const DEFAULT_PER_MINUTE: u32 = 120;

impl ClientRateLimiter {
    async fn allow(&self, client: &ItgApiClient) -> bool {
        let per_minute = client
            .rate_limit
            .as_ref()
            .and_then(|r| r.get("per_minute"))
            .and_then(Value::as_u64)
            .map_or(DEFAULT_PER_MINUTE, |pm| pm.min(u64::from(u32::MAX)) as u32);
        let limiter = self
            .inner
            .entry(client.client_key.clone())
            .or_insert_with(|| {
                crate::middleware::rate_limit::RateLimiter::new(
                    std::sync::Arc::new(crate::middleware::rate_limit::MemoryStore::new()),
                    crate::middleware::rate_limit::RateLimitConfig {
                        max_requests: per_minute,
                        window_secs: 60,
                    },
                )
            });
        limiter.check(&client.client_key).await
    }
}

/// The outbound executor. Owned by the plane; safe to share.
pub struct EgressExecutor {
    pool: crate::db::Pool,
    vault: Option<Vault>,
    config: IntegrationConfig,
    limiter: ClientRateLimiter,
}

/// What the HTTP call produced (log-writing input shared by both paths).
struct CallOutcome {
    status: u16,
    body: Value,
    body_text: String,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    model: Option<String>,
    error: Option<String>,
}

impl EgressExecutor {
    /// Build the executor (plane wires vault + config).
    #[must_use]
    pub fn new(pool: crate::db::Pool, vault: Option<Vault>, config: IntegrationConfig) -> Self {
        Self {
            pool,
            vault,
            config,
            limiter: ClientRateLimiter::default(),
        }
    }

    /// Execute one api-client call end-to-end (see module docs). Non-2xx and
    /// transport failures return `AppError` — the log row is written either way.
    ///
    /// # Errors
    ///
    /// `AppError::NotFound` unknown/disabled client or op;
    /// `AppError::TooManyRequests` rate limited;
    /// `AppError::BadRequest` bad template/input;
    /// `AppError::Internal` transport / non-2xx failure.
    pub async fn call(&self, req: EgressRequest) -> AppResult<EgressReceipt> {
        let trace_id = req
            .trace_id
            .or_else(|| super::trace::current().map(|c| c.trace_id));

        let client = api_client::model::find_by_key(
            &self.pool,
            crate::constants::DEFAULT_TENANT,
            &req.client_key,
        )
        .await?
        .ok_or_else(|| AppError::NotFound(format!("api-client '{}' not found", req.client_key)))?;
        if !client.enabled {
            return Err(AppError::BadRequest(format!(
                "api-client '{}' is disabled",
                req.client_key
            )));
        }

        if !self.limiter.allow(&client).await {
            let log_id = self
                .write_log(&req, trace_id, &client, None, 0, Some("rate limited"))
                .await;
            tracing::warn!(client = %req.client_key, log_id = log_id.map(|i| i.0), "egress rate limited");
            return Err(AppError::TooManyRequests(format!(
                "api-client '{}' rate limited",
                req.client_key
            )));
        }

        let op = client.op(&req.op).ok_or_else(|| {
            AppError::NotFound(format!(
                "op '{}' not defined on api-client '{}'",
                req.op, req.client_key
            ))
        })?;
        let method = op.get("method").and_then(Value::as_str).unwrap_or("GET");
        let path_tpl = op.get("path").and_then(Value::as_str).unwrap_or("/");
        let path = render_path(path_tpl, &req.input)?;

        // `url-path-token` auth: embed the sealed secret into the URL path
        // (e.g. Telegram `/bot<token>/sendMessage`), between base_url and the
        // op path. The secret is resolved before the header-injection block so
        // it applies to the URL rather than a header.
        let url = if client.auth_kind() == "url-path-token" {
            let prefix = client
                .auth
                .as_ref()
                .and_then(|a| a.get("path_prefix"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let secret = self.resolve_secret(&client).await?;
            let Some(secret) = secret else {
                return Err(AppError::BadRequest(
                    "url-path-token auth requires sealed credentials".into(),
                ));
            };
            format!(
                "{}{prefix}{secret}{path}",
                client.base_url.trim_end_matches('/')
            )
        } else {
            format!("{}{path}", client.base_url.trim_end_matches('/'))
        };
        let mut url = reqwest::Url::parse(&url)
            .map_err(|e| AppError::BadRequest(format!("egress url invalid: {e}")))?;

        // op.query — query-param templating: `{"q": "{q}", "limit": 10}`.
        // Values render `{var}` placeholders from input; reqwest encodes them.
        if let Some(query) = op.get("query").and_then(Value::as_object) {
            for (key, val) in query {
                let rendered = match val {
                    Value::String(s) => render_scalar(s, &req.input)?,
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    other => {
                        return Err(AppError::BadRequest(format!(
                            "op '{}': query '{key}' must be scalar (got {other})",
                            req.op
                        )));
                    }
                };
                url.query_pairs_mut().append_pair(key, &rendered);
            }
        }

        // op.headers — per-op custom headers (values templated from input).
        let op_headers = op.get("headers").and_then(Value::as_object);
        let has_content_type = op_headers
            .map(|h| h.keys().any(|k| k.eq_ignore_ascii_case("content-type")))
            .unwrap_or(false);

        let mut request = {
            use reqwest::Method;
            let m = Method::from_bytes(method.as_bytes())
                .map_err(|_| AppError::BadRequest(format!("method '{method}' invalid")))?;
            let client_http =
                crate::plugins::http_client::client_with_proxy(self.config.egress_timeout_secs)?;
            let mut builder = client_http.request(m, url);
            // Body shapes, in priority order (validate enforces at most one):
            //   op.form      → application/x-www-form-urlencoded
            //   op.multipart → multipart/form-data (text + file parts)
            //   op.body      → JSON template (or input for write methods)
            if let Some(form) = op.get("form").and_then(Value::as_object) {
                let mut pairs: Vec<(String, String)> = Vec::with_capacity(form.len());
                for (key, val) in form {
                    let rendered = match val {
                        Value::String(s) => render_scalar(s, &req.input)?,
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        other => {
                            return Err(AppError::BadRequest(format!(
                                "op '{}': form '{key}' must be scalar (got {other})",
                                req.op
                            )));
                        }
                    };
                    pairs.push((key.clone(), rendered));
                }
                builder = builder.form(&pairs);
            } else if let Some(mp) = op.get("multipart").and_then(Value::as_object) {
                builder = build_multipart(builder, mp, &req.input)?;
            } else {
                // GET/DELETE carry no body unless an explicit op.body is given.
                let body = if let Some(tpl) = op.get("body") {
                    Some(render_body(tpl, &req.input)?)
                } else if !(method == "GET" || method == "DELETE") {
                    Some(req.input.clone())
                } else {
                    None
                };
                if let Some(body) = body {
                    if !has_content_type {
                        builder = builder.header("Content-Type", "application/json");
                    }
                    builder = builder.json(&body);
                }
            }
            builder
        };

        if let Some(headers) = op_headers {
            for (key, val) in headers {
                let v = match val {
                    Value::String(s) => render_scalar(s, &req.input)?,
                    other => {
                        return Err(AppError::BadRequest(format!(
                            "op '{}': header '{key}' must be a string (got {other})",
                            req.op
                        )));
                    }
                };
                request = request.header(key, v);
            }
        }

        if let Some(secret) = self.resolve_secret(&client).await? {
            request = match client.auth_kind() {
                // `oauth2-auth-code` resolves to a bearer access token too
                // (oauth2-egress.md §4: "bearer 注入路径零改动").
                "bearer" | "oauth2-auth-code" => {
                    let prefix = client
                        .auth
                        .as_ref()
                        .and_then(|a| a.get("prefix"))
                        .and_then(Value::as_str)
                        .unwrap_or("Bearer");
                    request.header("Authorization", format!("{prefix} {secret}"))
                }
                "basic" => {
                    // resolve_secret returns the pre-encoded `user:pass` base64.
                    let prefix = client
                        .auth
                        .as_ref()
                        .and_then(|a| a.get("prefix"))
                        .and_then(Value::as_str)
                        .unwrap_or("Basic");
                    request.header("Authorization", format!("{prefix} {secret}"))
                }
                "api-key-header" => {
                    let header = client
                        .auth
                        .as_ref()
                        .and_then(|a| a.get("header"))
                        .and_then(Value::as_str)
                        .unwrap_or("X-API-Key");
                    request.header(header, secret)
                }
                _ => request,
            };
        }

        let started = std::time::Instant::now();
        let outcome = match request.send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let bytes = resp.bytes().await.unwrap_or_default();
                let body_text = String::from_utf8_lossy(&bytes).to_string();
                let (tokens_in, tokens_out, model) = usage_of(&body_text);
                let error = if (200..300).contains(&status) {
                    None
                } else {
                    Some(format!("http {status}"))
                };
                CallOutcome {
                    status,
                    body: parse_body(&body_text),
                    body_text,
                    tokens_in,
                    tokens_out,
                    model,
                    error,
                }
            }
            Err(err) => CallOutcome {
                status: 0,
                body: Value::Null,
                body_text: String::new(),
                tokens_in: None,
                tokens_out: None,
                model: None,
                error: Some(err.to_string()),
            },
        };
        let latency_ms = started.elapsed().as_millis() as i64;

        let log_id = self
            .write_log(&req, trace_id, &client, Some(&outcome), latency_ms, None)
            .await;

        if let Some(error) = &outcome.error {
            return Err(AppError::Internal(anyhow::anyhow!(
                "egress {}.{} failed: {error} (log {})",
                req.client_key,
                req.op,
                log_id.map(|l| l.0.to_string()).unwrap_or_default()
            )));
        }

        let output = match op.get("output") {
            Some(Value::Object(rules)) => apply_output_mapping(rules, &outcome.body)?,
            _ => outcome.body.clone(),
        };
        Ok(EgressReceipt {
            log_id: log_id.unwrap_or_else(crate::utils::id::new_snowflake_id),
            status: outcome.status,
            output,
            body: outcome.body,
            tokens_in: outcome.tokens_in,
            tokens_out: outcome.tokens_out,
            model: outcome.model,
        })
    }

    /// Unseal the credential secret (None when the client has no credentials).
    async fn resolve_secret(&self, client: &ItgApiClient) -> AppResult<Option<String>> {
        let Some(sealed) = client.credentials.as_deref() else {
            return Ok(None);
        };
        let Some(vault) = &self.vault else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "api-client '{}' has credentials but the vault is sealed",
                client.client_key
            )));
        };
        let json = vault.unseal(sealed)?;
        let value: Value = serde_json::from_str(&json)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("credential json: {e}")))?;
        // OAuth client-credentials: a shared, cached dynamic token (the same
        // one the stream connectors use — one refresh serves both sides).
        if super::token::is_oauth_cc(&value) {
            return super::token::resolve_token(&client.client_key, &value)
                .await
                .map(Some);
        }
        // OAuth2 authorization-code (3-legged): persisted per (client, tenant),
        // auto-refreshed. Default tenant for now (oauth2-egress.md §4).
        if super::token::is_auth_code(&value) {
            return super::token::resolve_auth_code_token(
                &client.client_key,
                crate::constants::DEFAULT_TENANT,
                &value,
                &self.pool,
                self.vault.as_ref(),
            )
            .await
            .map(Some);
        }
        if let Some(secret) = value
            .get("secret")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            return Ok(Some(secret));
        }
        // Basic auth: credentials carry `username`/`password`; pre-encode the
        // `user:pass` combo so the injection branch just prefixes "Basic ".
        if let (Some(user), Some(pass)) = (
            value.get("username").and_then(Value::as_str),
            value.get("password").and_then(Value::as_str),
        ) {
            use base64::Engine as _;
            let combined = format!("{user}:{pass}");
            let encoded = base64::engine::general_purpose::STANDARD.encode(combined.as_bytes());
            return Ok(Some(encoded));
        }
        Ok(None)
    }

    /// Best-effort log write; failures are logged, never fail the call path
    /// twice (the outcome itself is already terminal for the caller).
    async fn write_log(
        &self,
        req: &EgressRequest,
        trace_id: Option<i64>,
        client: &ItgApiClient,
        outcome: Option<&CallOutcome>,
        latency_ms: i64,
        error_override: Option<&str>,
    ) -> Option<SnowflakeId> {
        let (status, http_status, tokens_in, tokens_out, model, error, summary) = match outcome {
            Some(o) => (
                if o.error.is_none() {
                    "success"
                } else {
                    "error"
                },
                Some(i64::from(o.status)),
                o.tokens_in,
                o.tokens_out,
                o.model.clone(),
                o.error.clone(),
                summarize(&o.body_text),
            ),
            None => (
                "error",
                None,
                None,
                None,
                None,
                error_override.map(str::to_string),
                None,
            ),
        };
        let id = crate::utils::id::new_snowflake_id();
        let placeholders: Vec<String> = (1..=13).map(crate::db::Driver::ph).collect();
        let sql = format!(
            "INSERT INTO itg_egress_log (id, trace_id, client_key, op, status, http_status, \
             latency_ms, tokens_in, tokens_out, model, error, response_summary, created_at) \
             VALUES ({})",
            placeholders.join(", ")
        );
        let result = sqlx::query(crate::db::safe_sql(&sql))
            .bind(*id)
            .bind(trace_id)
            .bind(client.client_key.as_str())
            .bind(req.op.as_str())
            .bind(status)
            .bind(http_status)
            .bind(latency_ms)
            .bind(tokens_in)
            .bind(tokens_out)
            .bind(model)
            .bind(error)
            .bind(summary)
            .bind(crate::utils::tz::now_utc())
            .execute(&self.pool)
            .await;
        if let Err(err) = result {
            tracing::error!(client = %req.client_key, error = %err, "egress log write failed");
            return None;
        }
        Some(id)
    }
}

/// `{var}` placeholder rendering from input (string form, URL-encoded).
/// Unknown vars are a config error at call time — better loud than silently
/// hitting the wrong path.
fn render_path(tpl: &str, input: &Value) -> AppResult<String> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err(AppError::BadRequest(format!(
                "path template '{tpl}' has an unclosed '{{'"
            )));
        };
        let var = &after[..end];
        let val = input
            .get(var)
            .map(value_to_path_string)
            .transpose()?
            .ok_or_else(|| {
                AppError::BadRequest(format!("path placeholder '{{{var}}}' missing in input"))
            })?;
        out.push_str(&percent_encode(&val));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Minimal percent-encoding for a path segment (RFC 3986 unreserved kept).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn value_to_path_string(v: &Value) -> AppResult<String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        Value::Bool(b) => Ok(b.to_string()),
        _ => Err(AppError::BadRequest(
            "path placeholder values must be scalar".into(),
        )),
    }
}

/// Render `{var}` placeholders in a scalar template from input (query values,
/// header values, string-interpolated body fields). Values are NOT
/// percent-encoded — callers decide (path encodes, query/headers let reqwest).
fn render_scalar(tpl: &str, input: &Value) -> AppResult<String> {
    let mut out = String::with_capacity(tpl.len());
    let mut rest = tpl;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err(AppError::BadRequest(format!(
                "template '{tpl}' has an unclosed '{{'"
            )));
        };
        let var = &after[..end];
        let val = input
            .get(var)
            .map(value_to_path_string)
            .transpose()?
            .ok_or_else(|| {
                AppError::BadRequest(format!("placeholder '{{{var}}}' missing in input"))
            })?;
        out.push_str(&val);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// JSON-aware body template: walk the op.body shape recursively, substituting
/// input values. A whole-string `{var}` embeds the raw value (type preserved,
/// e.g. `{"limit": {limit}}` → number); `{var}` inside a larger string is
/// interpolated as text (e.g. `{"q": "user:{q}"}`).
fn render_body(tpl: &Value, input: &Value) -> AppResult<Value> {
    match tpl {
        Value::String(s) => {
            if let Some(var) = s
                .strip_prefix('{')
                .and_then(|r| r.strip_suffix('}'))
                .filter(|var| {
                    !var.is_empty()
                        && var
                            .chars()
                            .all(|c| !c.is_whitespace() && c != '{' && c != '}')
                })
                && let Some(val) = input.get(var)
            {
                return Ok(val.clone());
            }
            Ok(Value::String(render_scalar(s, input)?))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), render_body(v, input)?);
            }
            Ok(Value::Object(out))
        }
        Value::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for v in arr {
                out.push(render_body(v, input)?);
            }
            Ok(Value::Array(out))
        }
        other => Ok(other.clone()),
    }
}

/// Build a multipart/form-data body from an op template:
/// `{"text": {field: scalar-tpl}, "files": {name: {filename, content_type, content(b64)}}}`
/// File bytes are base64 in the input and decoded here.
fn build_multipart(
    builder: reqwest::RequestBuilder,
    mp: &serde_json::Map<String, Value>,
    input: &Value,
) -> AppResult<reqwest::RequestBuilder> {
    use base64::Engine as _;
    let mut form = reqwest::multipart::Form::new();
    if let Some(text) = mp.get("text").and_then(Value::as_object) {
        for (key, val) in text {
            let rendered = match val {
                Value::String(s) => render_scalar(s, input)?,
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                other => {
                    return Err(AppError::BadRequest(format!(
                        "multipart text '{key}' must be scalar (got {other})"
                    )));
                }
            };
            form = form.text(key.clone(), rendered);
        }
    }
    if let Some(files) = mp.get("files").and_then(Value::as_object) {
        for (key, val) in files {
            let Some(fobj) = val.as_object() else {
                return Err(AppError::BadRequest(format!(
                    "multipart file '{key}' must be an object"
                )));
            };
            let tpl = |field: &str| -> AppResult<String> {
                let s = fobj.get(field).and_then(Value::as_str).ok_or_else(|| {
                    AppError::BadRequest(format!(
                        "multipart file '{key}' requires a string '{field}'"
                    ))
                })?;
                render_scalar(s, input)
            };
            let filename = tpl("filename")?;
            let content_type = tpl("content_type")?;
            let b64 = tpl("content")?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.as_bytes())
                .map_err(|e| {
                    AppError::BadRequest(format!(
                        "multipart file '{key}' content is not valid base64: {e}"
                    ))
                })?;
            let mut part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
            if !content_type.is_empty() {
                part = part.mime_str(&content_type).map_err(|e| {
                    AppError::BadRequest(format!("multipart file '{key}' bad content_type: {e}"))
                })?;
            }
            form = form.part(key.clone(), part);
        }
    }
    Ok(builder.multipart(form))
}

fn parse_body(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
}

fn summarize(text: &str) -> Option<String> {
    if text.is_empty() {
        None
    } else if text.len() <= 2000 {
        Some(text.to_string())
    } else {
        let mut s = text[..2000].to_string();
        s.push_str("…(truncated)");
        Some(s)
    }
}

/// Well-known usage fields (OpenAI-style `usage.prompt_tokens` /
/// Anthropic-style `usage.input_tokens` + top-level `model`).
fn usage_of(body: &str) -> (Option<i64>, Option<i64>, Option<String>) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return (None, None, None);
    };
    let usage = v.get("usage");
    let pick = |keys: &[&str]| -> Option<i64> {
        usage.and_then(|u| keys.iter().find_map(|k| u.get(*k).and_then(Value::as_i64)))
    };
    let model = v.get("model").and_then(Value::as_str).map(str::to_string);
    (
        pick(&["prompt_tokens", "input_tokens"]),
        pick(&["completion_tokens", "output_tokens"]),
        model,
    )
}

/// Apply an op `output` mapping: `{out_field: "$.src.path"}` — plain dot paths
/// against the response JSON (the §7.1 mapping DSL is envelope-specific; this
/// stays minimal).
fn apply_output_mapping(rules: &serde_json::Map<String, Value>, body: &Value) -> AppResult<Value> {
    let mut out = serde_json::Map::new();
    for (target, expr) in rules {
        let path = expr.as_str().unwrap_or_default();
        let src = path.strip_prefix("$.").unwrap_or(path);
        out.insert(target.clone(), walk_path(body, src));
    }
    Ok(Value::Object(out))
}

fn walk_path(mut v: &Value, dotted: &str) -> Value {
    for seg in dotted.split('.').filter(|s| !s.is_empty()) {
        v = match v {
            Value::Object(map) => map.get(seg).unwrap_or(&Value::Null),
            Value::Array(arr) => seg
                .parse::<usize>()
                .ok()
                .and_then(|i| arr.get(i))
                .unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    v.clone()
}

/// Query egress log rows for admin (filters optional).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn list_log(
    pool: &crate::db::Pool,
    trace_id: Option<SnowflakeId>,
    client_key: Option<&str>,
    limit: u64,
) -> AppResult<Vec<EgressLogRow>> {
    let mut clauses: Vec<String> = Vec::new();
    let mut bind_trace: Option<i64> = None;
    let mut bind_client: Option<String> = None;
    if let Some(t) = trace_id {
        clauses.push(format!("trace_id = {}", crate::db::Driver::ph(1)));
        bind_trace = Some(t.0);
    }
    if let Some(k) = client_key {
        clauses.push(format!(
            "client_key = {}",
            crate::db::Driver::ph(if bind_trace.is_some() { 2 } else { 1 })
        ));
        bind_client = Some(k.to_string());
    }
    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, trace_id, client_key, op, status, http_status, latency_ms, tokens_in, \
         tokens_out, model, error, response_summary, created_at FROM itg_egress_log{where_sql} \
         ORDER BY id DESC LIMIT {limit}"
    );
    let mut q = sqlx::query_as::<crate::db::pool::Db, EgressLogRow>(crate::db::safe_sql(&sql));
    if let Some(t) = bind_trace {
        q = q.bind(t);
    }
    if let Some(k) = bind_client {
        q = q.bind(k);
    }
    Ok(q.fetch_all(pool).await?)
}

/// Retention cleanup — cron handler body (`integration.egress_cleanup`).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn cleanup_old(pool: &crate::db::Pool, retention_days: u64) -> AppResult<u64> {
    let cutoff = crate::utils::tz::now_utc()
        - chrono::TimeDelta::try_days(retention_days.max(1) as i64).unwrap_or_default();
    let sql = format!(
        "DELETE FROM itg_egress_log WHERE created_at < {}",
        crate::db::Driver::ph(1)
    );
    let result = sqlx::query(crate::db::safe_sql(&sql))
        .bind(cutoff)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// Snapshot of egress stats per client (health API input).
///
/// # Errors
///
/// Returns `AppError` on SQL failure.
pub async fn stats_by_client(
    pool: &crate::db::Pool,
) -> AppResult<HashMap<String, (i64, i64, i64)>> {
    let sql = format!(
        "SELECT client_key, {total}, {errors}, {latency} FROM itg_egress_log \
         WHERE created_at > {recent} GROUP BY client_key",
        total = crate::db::Driver::cast_int("COUNT(*)"),
        errors = crate::db::Driver::cast_int("SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END)"),
        latency = crate::db::Driver::cast_int("COALESCE(AVG(latency_ms), 0)"),
        recent = crate::db::Driver::ph(1),
    );
    let since = crate::utils::tz::now_utc() - chrono::TimeDelta::try_hours(24).unwrap_or_default();
    let rows: Vec<(String, i64, i64, i64)> = sqlx::query_as(crate::db::safe_sql(&sql))
        .bind(since)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(k, total, errors, avg_latency)| (k, (total, errors, avg_latency)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn render_path_substitutes_scalars() {
        let input = json!({"id": 42, "name": "hello world"});
        let out = render_path("/items/{id}/detail/{name}", &input).expect("render");
        assert_eq!(out, "/items/42/detail/hello%20world");
    }

    #[test]
    fn render_path_rejects_missing_var() {
        let err = render_path("/items/{missing}", &json!({})).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn render_scalar_interpolates_without_encoding() {
        let input = json!({"q": "hello world", "page": 2});
        assert_eq!(render_scalar("{q}", &input).unwrap(), "hello world");
        assert_eq!(
            render_scalar("page={page}&x=1", &input).unwrap(),
            "page=2&x=1"
        );
        assert!(render_scalar("{missing}", &input).is_err());
    }

    #[test]
    fn render_body_embeds_raw_and_interpolates() {
        let input = json!({"q": "hi", "limit": 10, "user": "alice"});
        let mut query = serde_json::Map::new();
        query.insert("q".into(), json!("user:{user}"));
        query.insert("match".into(), json!("{q}"));
        let mut tpl = serde_json::Map::new();
        tpl.insert("query".into(), Value::Object(query));
        tpl.insert("limit".into(), json!("{limit}"));
        tpl.insert("tags".into(), json!(["a", "{q}"]));
        let tpl = Value::Object(tpl);
        let out = render_body(&tpl, &input).unwrap();
        assert_eq!(out["query"]["q"], "user:alice");
        assert_eq!(out["query"]["match"], "hi");
        assert_eq!(
            out["limit"],
            json!(10),
            "whole-string {{var}} keeps the raw type"
        );
        assert_eq!(out["tags"][1], "hi");
    }

    #[test]
    fn render_body_unknown_var_in_string_stays_literal_via_scalar_error() {
        // A whole-string {var} that isn't in input falls back to scalar render,
        // which errors loudly (config mistake) rather than sending a blank.
        assert!(render_body(&json!("{nope}"), &json!({})).is_err());
    }

    #[test]
    fn usage_extraction_openai_and_anthropic() {
        let (i, o, m) =
            usage_of(r#"{"model":"gpt-4o","usage":{"prompt_tokens":10,"completion_tokens":5}}"#);
        assert_eq!((i, o), (Some(10), Some(5)));
        assert_eq!(m.as_deref(), Some("gpt-4o"));
        let (i, o, _) = usage_of(r#"{"usage":{"input_tokens":7,"output_tokens":3}}"#);
        assert_eq!((i, o), (Some(7), Some(3)));
        assert_eq!(usage_of("not json"), (None, None, None));
    }

    #[test]
    fn output_mapping_walks_dot_paths() {
        let body = json!({"answer": "hi", "meta": {"model": "x"}});
        let mut rules = serde_json::Map::new();
        rules.insert("text".into(), json!("$.answer"));
        rules.insert("m".into(), json!("$.meta.model"));
        let out = apply_output_mapping(&rules, &body).expect("map");
        assert_eq!(out["text"], "hi");
        assert_eq!(out["m"], "x");
    }

    #[test]
    fn summarize_truncates_long_bodies() {
        let long = "x".repeat(3000);
        let s = summarize(&long).expect("some");
        assert!(s.len() < 2100);
        assert!(s.ends_with("…(truncated)"));
    }
}
