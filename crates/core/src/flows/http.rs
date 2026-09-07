//! `http` node executor (dev-docs/workflow — n8n HTTP Request shape).
//!
//! Templates (C3.1) render against the pool, then one outbound request fires
//! through a shared client. SSRF: rendered hosts hitting localhost/private
//! ranges are rejected before any socket opens (same rule set as the plugin
//! host). Output fields: `status` / `json` (when parseable) / `body` /
//! `latency_ms`.

use std::sync::OnceLock;

use serde_json::{Map, Value, json};

use crate::errors::app_error::{AppError, AppResult};

use super::engine::{ExecOutcome, Pool};
use super::expr;
use super::graph::GraphNode;
use super::nodes::HttpConfig;

static HTTP: OnceLock<reqwest::Client> = OnceLock::new();

fn client() -> &'static reqwest::Client {
    HTTP.get_or_init(|| reqwest::Client::builder().build().unwrap_or_default())
}

/// Render a template to plain text (scalars stringify; whole-object refs
/// serialize as JSON — headers/query need text).
fn render_text(text: &str, pool: &Pool) -> AppResult<String> {
    match expr::resolve_text(text, pool)? {
        Value::String(s) => Ok(s),
        other => Ok(serde_json::to_string(&other).unwrap_or_default()),
    }
}

/// Host part of a URL string (strip scheme, userinfo, port, path, query).
fn host_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let without_user = authority.rsplit('@').next().unwrap_or(authority);
    // IPv6 brackets first (a ':' split would eat the address), then port.
    if let Some(inner) = without_user.strip_prefix('[') {
        return inner.split(']').next().unwrap_or(inner).to_string();
    }
    without_user
        .split(':')
        .next()
        .unwrap_or(without_user)
        .to_string()
}

/// Execute the `http` node.
///
/// # Errors
/// `BadRequest` on template render failures, non-http(s) schemes and
/// SSRF-blocked hosts; `Internal` on transport/timeout failures.
pub async fn run_http(node: &GraphNode, pool: &Pool) -> AppResult<ExecOutcome> {
    let cfg: HttpConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("http config: {e}")))?;

    let started = std::time::Instant::now();
    let url = render_text(cfg.url.trim(), pool)?;
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::BadRequest(format!(
            "http: 渲染后的 url 非法（须 http/https）: {url}"
        )));
    }
    let host = host_of(&url);
    if crate::plugins::permissions::is_private_host(&host) {
        return Err(AppError::BadRequest(format!(
            "http: SSRF 防护拦截内网地址 '{host}'"
        )));
    }

    let mut req = client().request(
        reqwest::Method::from_bytes(cfg.method.as_bytes())
            .map_err(|_| AppError::BadRequest(format!("http: method 非法 {}", cfg.method)))?,
        &url,
    );
    for h in &cfg.headers {
        req = req.header(h.key.trim(), render_text(&h.value, pool)?);
    }
    if !cfg.query.is_empty() {
        let mut pairs = Vec::with_capacity(cfg.query.len());
        for q in &cfg.query {
            pairs.push((q.key.trim().to_string(), render_text(&q.value, pool)?));
        }
        req = req.query(&pairs);
    }
    if let Some(body) = cfg.body.as_deref().filter(|b| !b.trim().is_empty()) {
        let rendered = render_text(body, pool)?;
        let has_ct = cfg
            .headers
            .iter()
            .any(|h| h.key.trim().eq_ignore_ascii_case("content-type"));
        req = if has_ct {
            req.body(rendered)
        } else {
            req.header("Content-Type", "application/json")
                .body(rendered)
        };
    }

    let timeout_ms = cfg.timeout_ms.filter(|t| *t > 0).unwrap_or(15_000);
    let resp = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms as u64),
        req.send(),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("http 超时 {timeout_ms}ms: {host}")))?
    .map_err(|e| AppError::Internal(anyhow::anyhow!("http 请求失败 ({host}): {e}")))?;

    let status = resp.status().as_u16();
    let body_text = resp
        .text()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("http 读取响应失败: {e}")))?;
    let latency_ms = started.elapsed().as_millis() as i64;

    let mut out = Map::new();
    out.insert("status".into(), json!(status));
    let parsed = serde_json::from_str::<Value>(&body_text).ok();
    if let Some(v) = parsed {
        out.insert("json".into(), v);
    }
    out.insert("body".into(), json!(body_text));
    out.insert("latency_ms".into(), json!(latency_ms));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: None,
        latency_ms: Some(latency_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;

    fn http_node(config: Value) -> GraphNode {
        GraphNode {
            id: "n1".into(),
            data: NodeData {
                kind: "http".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn ssrf_blocks_loopback_before_any_socket() {
        let err = run_http(
            &http_node(json!({"method": "GET", "url": "http://127.0.0.1:9898/admin/secret"})),
            &Pool::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
        assert!(err.to_string().contains("SSRF"), "{err}");
    }

    #[tokio::test]
    async fn ssrf_blocks_private_and_localhost_names() {
        for url in [
            "http://10.0.0.5/api",
            "http://192.168.1.1/",
            "http://localhost:3000/x",
            "http://169.254.169.254/latest/meta-data",
        ] {
            let err = run_http(
                &http_node(json!({"method": "GET", "url": url})),
                &Pool::new(),
            )
            .await
            .unwrap_err();
            assert!(err.to_string().contains("SSRF"), "{url} -> {err}");
        }
    }

    #[tokio::test]
    async fn template_render_failure_is_bad_request() {
        let mut pool = Pool::new();
        pool.insert("start".into(), Default::default());
        let err = run_http(
            &http_node(json!({"method": "GET", "url": "https://x.io/{{#start.nope#}}"})),
            &pool,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[test]
    fn host_extraction() {
        assert_eq!(host_of("https://api.x.io/v1?y=1"), "api.x.io");
        assert_eq!(host_of("http://user:pw@10.0.0.1:8080/a"), "10.0.0.1");
        assert_eq!(host_of("http://[::1]:9000/x"), "::1");
    }
}
