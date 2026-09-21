//! `docparse` node executor (docparse-node.md §2).
//!
//! Parses one document into markdown through the shared docparse engine
//! registry, mirroring `llm` node shape: a per-call runtime injected by
//! `FlowsExec` (test doubles stay possible), output fields flattened into the
//! node namespace for downstream `{{#id.field#}}` refs.
//!
//! Input is a ValueExpr resolving to a file reference — either a storage key
//! or an external `https://` URL (SSRF-validated, no redirects). Embedded
//! images are persisted to storage and their markdown refs rewritten, exactly
//! like `docparse::conversion::convert_inner` (one shared convention).

use std::sync::Arc;
use std::time::Instant;

use serde_json::{Map, Value, json};

use crate::docparse::{ParseOpts, ParserRegistry};
use crate::errors::app_error::{AppError, AppResult};
use crate::storage::Storage;

use super::engine::{ExecOutcome, Pool};
use super::graph::GraphNode;
use super::nodes::DocParseConfig;

/// Per-run docparse runtime (registry + storage + global default engine).
/// Built from the process-wide [`crate::docparse::shared`] host; tests inject
/// their own so no global init is required.
#[derive(Clone)]
pub struct DocParseRuntime {
    pub parsers: Arc<ParserRegistry>,
    pub storage: Arc<dyn Storage>,
    pub global_engine: Option<String>,
    /// Per-run tenant (usage attribution; the node itself is tenant-agnostic).
    pub tenant: String,
}

impl DocParseRuntime {
    /// Build from the shared host; `None` when the host was never installed
    /// (unit tests / boot before `build_app_state`).
    #[must_use]
    pub fn from_shared(tenant: String) -> Option<Self> {
        crate::docparse::shared().map(|h| Self {
            parsers: h.parsers.clone(),
            storage: h.storage.clone(),
            global_engine: h.global_engine.clone(),
            tenant,
        })
    }
}

/// Fetch input bytes + a filename (drives mime sniffing) from a reference:
/// `https://` URL → SSRF-checked download; anything else → storage key.
async fn load_input(runtime: &DocParseRuntime, reference: &str) -> AppResult<(Vec<u8>, String)> {
    let r = reference.trim();
    if r.is_empty() {
        return Err(AppError::BadRequest("docparse: input 为空".into()));
    }
    if r.starts_with("https://") || r.starts_with("http://") {
        crate::docparse::validate_external_url(r)?;
        // Redirects disabled: a 302 to a private host must not bypass the
        // SSRF check above (`validate_external_url` only sees the first hop).
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("docparse http client: {e}")))?;
        let resp = client
            .get(r)
            .send()
            .await
            .map_err(|e| AppError::BadRequest(format!("docparse: 下载失败 {e}")))?;
        if !resp.status().is_success() {
            return Err(AppError::BadRequest(format!(
                "docparse: 下载失败 HTTP {}",
                resp.status()
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| AppError::BadRequest(format!("docparse: 读取响应失败 {e}")))?;
        if bytes.len() > crate::docparse::conversion::MAX_INPUT_BYTES {
            return Err(AppError::BadRequest(format!(
                "docparse: 文件过大 {} bytes (max {})",
                bytes.len(),
                crate::docparse::conversion::MAX_INPUT_BYTES
            )));
        }
        let filename = r
            .rsplit('/')
            .next()
            .unwrap_or("document")
            .split(['?', '#'])
            .next()
            .unwrap_or("document")
            .to_string();
        return Ok((bytes.to_vec(), filename));
    }
    // Storage key (reads are bounded by the object already persisted).
    let bytes = runtime.storage.get(r).await?;
    if bytes.is_empty() {
        return Err(AppError::BadRequest(format!(
            "docparse: 存储对象为空或不存在: {r}"
        )));
    }
    let filename = r.rsplit('/').next().unwrap_or("document").to_string();
    Ok((bytes, filename))
}

/// Execute the `docparse` node against the variable pool.
///
/// # Errors
/// `BadRequest` on missing/invalid input refs, unknown engine, or a parse
/// failure (fail-fast — the engine's blind retry never helps authoring bugs);
/// `Internal` on transient transport/storage failures.
pub async fn run_docparse(
    runtime: &DocParseRuntime,
    node: &GraphNode,
    pool: &Pool,
) -> AppResult<ExecOutcome> {
    let cfg: DocParseConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("docparse config: {e}")))?;

    // Whole-string ref keeps its typed value; a file ref must be a string.
    let raw = super::engine::resolve(&cfg.input, pool)?;
    let reference = raw.as_str().map(str::to_string).ok_or_else(|| {
        AppError::BadRequest(
            "docparse: input 须解析为文件引用字符串（存储 key 或 https URL）".into(),
        )
    })?;
    let (bytes, filename) = load_input(runtime, &reference).await?;

    // Engine routing: node override → global default → builtin; an unavailable
    // engine warns and degrades to builtin (same semantics as KB routing).
    let requested = cfg
        .engine
        .clone()
        .or_else(|| runtime.global_engine.clone())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "builtin".to_string());
    let mut warnings: Vec<String> = Vec::new();
    let engine = match runtime.parsers.get(requested.trim()) {
        Some(e) if e.probe().await => e,
        _ => {
            warnings.push(format!(
                "engine '{}' unavailable → falling back to builtin",
                requested
            ));
            runtime
                .parsers
                .get("builtin")
                .ok_or_else(|| AppError::Internal(anyhow::anyhow!("builtin engine missing")))?
        }
    };

    let mime = mime_guess::from_path(&filename).first_or_octet_stream();
    let started = Instant::now();
    let parsed = engine
        .parse(
            &bytes,
            mime.essence_str(),
            &filename,
            &ParseOpts {
                extract_images: cfg.extract_images,
            },
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("docparse engine '{}': {e}", engine.name())))?;
    let latency_ms = started.elapsed().as_millis() as i64;

    // Persist extracted images + rewrite markdown refs (shared naming shape with
    // conversion::convert_inner so both surfaces produce portable markdown).
    let mut markdown = parsed.markdown;
    let mut images: Vec<Value> = Vec::new();
    let job_id = crate::utils::id::new_id().to_string();
    for img in &parsed.images {
        let key = format!(
            "parse/flows/{job_id}/images/{}",
            img.ref_name.trim_start_matches("./")
        );
        runtime
            .storage
            .put(&key, &img.bytes, &img.mime_type)
            .await?;
        let url = runtime
            .storage
            .url(&key)
            .await
            .unwrap_or_else(|_| format!("/{key}"));
        markdown = markdown.replace(&format!("]({})", img.ref_name), &format!("]({url})"));
        images.push(json!({ "name": img.ref_name, "key": key, "url": url }));
    }

    let mut out = Map::new();
    out.insert("markdown".into(), json!(markdown));
    out.insert("images".into(), Value::Array(images));
    out.insert("engine".into(), json!(engine.name()));
    out.insert("pages".into(), json!(parsed.pages));
    out.insert("warnings".into(), json!(warnings));
    Ok(ExecOutcome {
        output: Value::Object(out),
        usage: None,
        latency_ms: Some(latency_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docparse::{ParseEngine, ParseOutcome, ParsedImage};
    use crate::flows::graph::NodeData;
    use serde_json::json;

    struct FakeEngine;

    #[async_trait::async_trait]
    impl ParseEngine for FakeEngine {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn supports(&self, _mime: &str, _filename: &str) -> bool {
            true
        }
        async fn probe(&self) -> bool {
            true
        }
        async fn parse(
            &self,
            _bytes: &[u8],
            _mime: &str,
            _filename: &str,
            _opts: &ParseOpts,
        ) -> AppResult<ParseOutcome> {
            Ok(ParseOutcome {
                markdown: "# DOCPARSE_TEST\n\n![pic](images/pic.png)".into(),
                images: vec![ParsedImage {
                    ref_name: "images/pic.png".into(),
                    mime_type: "image/png".into(),
                    bytes: b"PNGDATA".to_vec(),
                }],
                engine: "fake".into(),
                pages: Some(3),
                scanned_pages: Vec::new(),
            })
        }
    }

    fn runtime(dir: &str, global: Option<&str>) -> (DocParseRuntime, Arc<dyn Storage>) {
        let storage: Arc<dyn Storage> = Arc::new(
            crate::storage::local::LocalStorage::new(dir, "/uploads").expect("local storage"),
        );
        let parsers = Arc::new(ParserRegistry::new(vec![Arc::new(FakeEngine)]));
        (
            DocParseRuntime {
                parsers,
                storage: storage.clone(),
                global_engine: global.map(str::to_string),
                tenant: "default".into(),
            },
            storage,
        )
    }

    fn node(config: Value) -> GraphNode {
        GraphNode {
            id: "n1".into(),
            data: NodeData {
                kind: "docparse".into(),
                version: 1,
                title: String::new(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    fn pool_with(pairs: &[(&str, &str, Value)]) -> Pool {
        let mut pool = Pool::new();
        for (ns, name, v) in pairs {
            pool.entry((*ns).to_string())
                .or_default()
                .insert((*name).to_string(), v.clone());
        }
        pool
    }

    #[tokio::test]
    async fn parses_storage_key_and_rewrites_images() {
        let dir = format!("/tmp/docparse-node-test/{}", crate::utils::id::new_id());
        let (rt, storage) = runtime(&dir, Some("fake"));
        storage
            .put("kb/doc.txt", b"hello", "text/plain")
            .await
            .unwrap();
        let pool = pool_with(&[("start", "file", json!("kb/doc.txt"))]);
        let node = node(json!({"input": {"ref": ["start", "file"]}, "extract_images": true}));
        let out = run_docparse(&rt, &node, &pool).await.unwrap();
        assert_eq!(out.output["engine"], "fake");
        assert_eq!(out.output["pages"], 3);
        assert_eq!(out.output["images"].as_array().unwrap().len(), 1);
        let md = out.output["markdown"].as_str().unwrap();
        assert!(md.contains("DOCPARSE_TEST"));
        assert!(
            !md.contains("](images/pic.png)"),
            "image ref must be rewritten"
        );
    }

    #[tokio::test]
    async fn non_string_input_is_bad_request() {
        let dir = format!("/tmp/docparse-node-test/{}", crate::utils::id::new_id());
        let (rt, _s) = runtime(&dir, None);
        let pool = pool_with(&[("start", "n", json!(42))]);
        let node = node(json!({"input": {"ref": ["start", "n"]}}));
        let err = run_docparse(&rt, &node, &pool).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }

    #[tokio::test]
    async fn unknown_engine_falls_back_to_builtin_with_warning() {
        let dir = format!("/tmp/docparse-node-test/{}", crate::utils::id::new_id());
        let (rt, storage) = runtime(&dir, Some("ghost"));
        storage.put("a.txt", b"hi", "text/plain").await.unwrap();
        let pool = pool_with(&[("start", "file", json!("a.txt"))]);
        let node = node(json!({"input": {"ref": ["start", "file"]}}));
        let out = run_docparse(&rt, &node, &pool).await.unwrap();
        assert_eq!(out.output["engine"], "builtin");
        assert!(!out.output["warnings"].as_array().unwrap().is_empty());
    }
}
