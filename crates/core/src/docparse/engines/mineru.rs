//! MinerU parse engine — OpenDataLab MinerU 2.x HTTP API (`services/mineru`,
//! Docker :50053). Standalone per 方案A: fully independent of docreader; KB
//! parser rules route `pdf → mineru` once the engine is registered
//! (`RAISFAST_KB_MINERU_URL` set).
//!
//! Flow (verified e2e against mineru 4.0.2, 2026-09-18):
//! ① `POST /v1/uploads` → `PUT {id}/content` → `POST {id}/complete`
//! ② `POST /v1/parse/jobs` (markdown + middle_json) → poll `GET job`
//! ③ `GET /v1/files/{id}/content` → markdown / middle_json / zip(images)
//!
//! middle_json `pages[].page_idx` drives `<!-- page:N -->` anchor injection —
//! page mapping and the reader view depend on those anchors (same reason the
//! docreader PDF parser emits them).

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::errors::app_error::{AppError, AppResult};

use super::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};

pub struct MineruEngine {
    endpoint: String,
    timeout: Duration,
    http: reqwest::Client,
}

impl MineruEngine {
    /// From `RAISFAST_KB_MINERU_URL`; `None` when unconfigured (the engine
    /// then never registers).
    pub fn from_config(config: &crate::config::app::KbConfig) -> Option<Self> {
        let url = config.mineru_url.as_deref()?.trim();
        if url.is_empty() {
            return None;
        }
        Some(Self {
            endpoint: normalize_endpoint(url),
            timeout: Duration::from_secs(config.mineru_timeout_secs),
            http: reqwest::Client::new(),
        })
    }

    fn err(&self, context: &str, e: impl std::fmt::Display) -> AppError {
        AppError::ServiceUnavailable(format!("mineru {context}: {e}"))
    }

    /// Download one output file, returning raw bytes.
    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        let url = format!("{}/v1/files/{}/content", self.endpoint, file_id);
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| self.err("file content", e))?
            .error_for_status()
            .map_err(|e| self.err("file content", e))?;
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| self.err("file content read", e))
    }
}

fn normalize_endpoint(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Minimal middle_json projection — only what anchor injection needs.
#[derive(Deserialize)]
pub struct MiddleJson {
    pub pages: Vec<MiddlePage>,
}

#[derive(Deserialize)]
pub struct MiddlePage {
    pub page_idx: u32,
    #[serde(default)]
    blocks: Vec<MiddleBlock>,
}

#[derive(Deserialize)]
struct MiddleBlock {
    #[serde(default)]
    content: serde_json::Value,
}

impl MiddlePage {
    /// 从 blocks 的 content Value 中递归提取全部文本。
    pub fn first_text(&self) -> Option<String> {
        for block in &self.blocks {
            let text = extract_texts(&block.content).join("");
            let t = text.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
        None
    }
}

/// 递归提取 serde_json::Value 中所有文本（处理字符串和嵌套数组）。
fn extract_texts(v: &serde_json::Value) -> Vec<String> {
    match v {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(arr) => arr.iter().flat_map(extract_texts).collect(),
        serde_json::Value::Object(map) => map.get("content").map(extract_texts).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Inject `<!-- page:N -->` anchors into the markdown, guided by each page's
/// first text snippet (located in reading order; pages whose snippet is not
/// found are skipped — best-effort, like the docreader patch this mirrors).
/// Returns the anchored markdown and the total page count.
/// Inject `<!-- page:N -->` anchors into the markdown.
///
/// Two-pass algorithm for comprehensive coverage:
/// ① Text match — locate each page's first text snippet in the markdown.
/// ② Proportional fallback — unmatched pages get an estimated position
///    based on `page_idx / total_pages * md_len`.
///
/// Every page gets an anchor; `page_of` attributes each chunk to the nearest
/// preceding anchor. Returns the anchored markdown and the total page count.
pub(crate) fn inject_page_anchors(markdown: &str, pages: &[MiddlePage]) -> (String, u32) {
    let total = pages.len() as u32;
    let md_bytes = markdown.len();

    // Pass ①: text-matched positions (byte offsets in original markdown).
    let mut text_matched: Vec<(u32, usize)> = Vec::new(); // (page_no, byte_pos)
    let mut search_from = 0usize;
    for page in pages {
        let Some(needle) = page.first_text() else {
            continue;
        };
        let needle: String = needle.chars().take(48).collect();
        if let Some(rel) = markdown[search_from.min(markdown.len())..].find(&needle) {
            let pos = search_from + rel;
            text_matched.push((page.page_idx + 1, pos));
            search_from = pos + needle.len();
        }
    }

    // Pass ②: proportional fallback for unmatched pages.
    if text_matched.len() < total as usize {
        let matched_set: std::collections::HashSet<u32> =
            text_matched.iter().map(|(p, _)| *p).collect();
        let mut filled: Vec<(u32, usize)> = Vec::new();
        for page_no in 1..=total {
            if matched_set.contains(&page_no) {
                continue;
            }
            let est = (page_no as usize * md_bytes)
                .checked_div(total as usize)
                .unwrap_or(0);
            filled.push((page_no, est));
        }
        text_matched.extend(filled);
        text_matched.sort_by_key(|(_, pos)| *pos);
    }

    // Inject anchors from the end to preserve positions.
    let mut sorted: Vec<(u32, usize)> = text_matched;
    sorted.sort_by_key(|&(_, pos)| std::cmp::Reverse(pos));
    let mut out = markdown.to_string();
    for &(page_no, pos) in &sorted {
        let anchor = format!(
            "<!-- page:{} -->

",
            page_no
        );
        let pos = pos.min(out.len());
        out.insert_str(pos, &anchor);
    }
    (out, total)
}

/// flash 层专属格式（mineru/filetypes.py FLASH_ONLY_PARSE_EXTENSIONS）：
/// Office 全家 + HTML + CSV/TSV + EPUB + OFD。
fn flash_only(filename: &str) -> bool {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "doc"
            | "docx"
            | "ppt"
            | "pptx"
            | "xls"
            | "xlsx"
            | "rtf"
            | "odt"
            | "ods"
            | "odp"
            | "html"
            | "htm"
            | "shtml"
            | "csv"
            | "tsv"
            | "epub"
            | "ofd"
    )
}

fn zip_mime(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "application/octet-stream"
    }
}

#[async_trait::async_trait]
impl ParseEngine for MineruEngine {
    fn name(&self) -> &'static str {
        "mineru"
    }

    fn supports(&self, mime: &str, filename: &str) -> bool {
        if mime == "application/pdf" {
            return true;
        }
        filename
            .rsplit('.')
            .next()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
    }

    async fn probe(&self) -> bool {
        let url = format!("{}/v1/health", self.endpoint);
        matches!(
            self.http.get(&url).timeout(Duration::from_secs(5)).send().await,
            Ok(resp) if resp.status().is_success()
        )
    }

    async fn parse(
        &self,
        bytes: &[u8],
        _mime: &str,
        filename: &str,
        opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        // ① create upload → ② content → ③ complete
        let create = self
            .http
            .post(format!("{}/v1/uploads", self.endpoint))
            .timeout(Duration::from_secs(30))
            .json(&json!({
                "filename": filename,
                "bytes": bytes.len(),
                "mime_type": "application/pdf",
                "purpose": "parse",
            }))
            .send()
            .await
            .map_err(|e| self.err("create upload", e))?
            .error_for_status()
            .map_err(|e| self.err("create upload", e))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| self.err("create upload decode", e))?;
        let upload_id = create["id"]
            .as_str()
            .ok_or_else(|| self.err("create upload", "missing upload id"))?
            .to_string();

        self.http
            .put(format!(
                "{}/v1/uploads/{}/content",
                self.endpoint, upload_id
            ))
            .timeout(Duration::from_secs(120))
            .header("Content-Type", "application/octet-stream")
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| self.err("upload content", e))?
            .error_for_status()
            .map_err(|e| self.err("upload content", e))?;

        self.http
            .post(format!(
                "{}/v1/uploads/{}/complete",
                self.endpoint, upload_id
            ))
            .timeout(Duration::from_secs(30))
            .json(&json!({}))
            .send()
            .await
            .map_err(|e| self.err("complete upload", e))?
            .error_for_status()
            .map_err(|e| self.err("complete upload", e))?;

        // Resolve the stored file id (upload response carries `file.id`).
        let upload = self
            .http
            .get(format!("{}/v1/uploads/{}", self.endpoint, upload_id))
            .timeout(Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| self.err("upload status", e))?
            .error_for_status()
            .map_err(|e| self.err("upload status", e))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| self.err("upload status decode", e))?;
        let file_id = upload["file"]["id"]
            .as_str()
            .ok_or_else(|| self.err("upload status", "missing file id"))?
            .to_string();

        // ④ parse job
        let job = self
            .http
            .post(format!("{}/v1/parse/jobs", self.endpoint))
            .timeout(Duration::from_secs(30))
            .json(&json!({
                "files": [{ "source": { "type": "file_id", "file_id": file_id } }],
                "output_formats": ["markdown", "middle_json"],
                // Office/HTML/EPUB/OFD 是 flash 层专属（mineru/filetypes.py
                // FLASH_ONLY_PARSE_EXTENSIONS）——standard 层解析这些格式会失败。
                "tier": if flash_only(filename) { "flash" } else { "standard" },
            }))
            .send()
            .await
            .map_err(|e| self.err("create job", e))?
            .error_for_status()
            .map_err(|e| self.err("create job", e))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| self.err("create job decode", e))?;
        let job_id = job["job_id"]
            .as_str()
            .or_else(|| job["id"].as_str())
            .ok_or_else(|| self.err("create job", "missing job id"))?
            .to_string();

        // ⑤ poll until the file completes or the budget runs out.
        let deadline = tokio::time::Instant::now() + self.timeout;
        let output_files: serde_json::Value = loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::ServiceUnavailable(format!(
                    "mineru parse timeout (> {}s)",
                    self.timeout.as_secs()
                )));
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
            let job = self
                .http
                .get(format!("{}/v1/parse/jobs/{}", self.endpoint, job_id))
                .timeout(Duration::from_secs(30))
                .send()
                .await
                .map_err(|e| self.err("job status", e))?
                .error_for_status()
                .map_err(|e| self.err("job status", e))?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| self.err("job status decode", e))?;
            let file = &job["files"][0];
            match file["status"].as_str().unwrap_or_default() {
                "completed" => break file["output_files"].clone(),
                "failed" => {
                    let msg = file["error"].to_string();
                    return Err(AppError::ServiceUnavailable(format!(
                        "mineru parse failed: {msg}"
                    )));
                }
                _ => {}
            }
        };

        // ⑥ download markdown + middle_json (+ zip images when enabled).
        let md_id = output_files["markdown"]["file_id"]
            .as_str()
            .ok_or_else(|| self.err("job result", "missing markdown output"))?;
        let markdown_raw = String::from_utf8_lossy(&self.download(md_id).await?).into_owned();
        let middle_id = output_files["middle_json"]["file_id"].as_str();
        let middle: Option<MiddleJson> = match middle_id {
            Some(mid) => match self.download(mid).await {
                Ok(raw) => serde_json::from_slice(&raw).ok(),
                Err(e) => {
                    tracing::warn!(error = %e, "mineru middle_json download failed — pages unknown");
                    None
                }
            },
            None => None,
        };
        let pages = middle.as_ref().map(|m| m.pages.len() as u32);

        // ⑦ images from the zip bundle (recognition-enabled KBs only).
        let mut images: Vec<ParsedImage> = Vec::new();
        if opts.extract_images
            && let Some(zip_id) = output_files["zip"]["file_id"].as_str()
        {
            let zip_bytes = self.download(zip_id).await?;
            let cursor = std::io::Cursor::new(zip_bytes);
            let mut archive =
                zip::ZipArchive::new(cursor).map_err(|e| self.err("images zip", e))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i).map_err(|e| self.err("images zip", e))?;
                let Some(name) = entry
                    .enclosed_name()
                    .map(|p| p.to_string_lossy().to_string())
                else {
                    continue;
                };
                if name.ends_with('/') {
                    continue;
                }
                let mut buf = Vec::with_capacity(entry.size() as usize);
                std::io::copy(&mut entry, &mut buf).map_err(|e| self.err("images zip read", e))?;
                images.push(ParsedImage {
                    mime_type: zip_mime(&name).to_string(),
                    ref_name: name,
                    bytes: buf,
                });
            }
        }

        // ⑧ page anchors (middle_json pages drive the mapping).
        let markdown = match &middle {
            Some(m) => inject_page_anchors(&markdown_raw, &m.pages).0,
            None => markdown_raw,
        };

        Ok(ParseOutcome {
            markdown,
            images,
            engine: "mineru".into(),
            pages,
            scanned_pages: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_inject_before_page_starts() {
        let md = "第一页开头内容\n\n第二页开头内容\n\n第三页开头内容";
        let pages = vec![
            MiddlePage {
                page_idx: 0,
                blocks: vec![MiddleBlock {
                    content: serde_json::json!([{"type": "text", "content": "第一页开头内容"}]),
                }],
            },
            MiddlePage {
                page_idx: 1,
                blocks: vec![MiddleBlock {
                    content: serde_json::json!([{"type": "text", "content": "第二页开头内容"}]),
                }],
            },
            MiddlePage {
                page_idx: 2,
                blocks: vec![MiddleBlock {
                    content: serde_json::json!([{"type": "text", "content": "第三页开头内容"}]),
                }],
            },
        ];
        let (out, total) = inject_page_anchors(md, &pages);
        assert_eq!(total, 3);
        assert!(
            out.starts_with("<!-- page:1 -->"),
            "page 1 anchors at start"
        );
        let p2 = out.find("<!-- page:2 -->").unwrap();
        let p3 = out.find("<!-- page:3 -->").unwrap();
        let s2 = out.find("第二页开头内容").unwrap();
        let s3 = out.find("第三页开头内容").unwrap();
        assert!(p2 < s2 && p3 < s3, "each anchor must precede its page text");
        assert!(out.find("第一页开头内容").unwrap() > p1_pos(&out));
    }

    fn p1_pos(out: &str) -> usize {
        out.find("<!-- page:1 -->").unwrap() + 1
    }

    /// 回归（2026-09-18）：`doc_title`/表格块的 content 是嵌套数组而非
    /// 字符串——旧模型解码整份 middle_json 失败，页锚点一个都注不进去，
    /// 阅读视图全空。嵌套形态必须可解码且产出正确锚点。
    #[test]
    fn nested_content_blocks_decode_and_anchor() {
        let raw = r#"{"pages":[
            {"page_idx":0,"blocks":[
                {"type":"doc_title","content":[{"type":"text","content":"Solana: A new architecture"}]},
                {"type":"text","content":[{"type":"text","content":"正文第一页"}]}
            ]},
            {"page_idx":1,"blocks":[
                {"type":"text","content":[{"type":"text","content":"正文第二页开头"}]}
            ]}
        ]}"#;
        let middle: MiddleJson = serde_json::from_str(raw).expect("nested content must decode");
        let md = "Solana: A new architecture\n\n正文第一页\n\n正文第二页开头";
        let (out, total) = inject_page_anchors(md, &middle.pages);
        assert_eq!(total, 2);
        assert_eq!(
            out.matches("<!-- page:").count(),
            2,
            "both pages anchored: {out}"
        );
    }

    #[test]
    fn pages_missing_needle_are_skipped() {
        let md = "只有这一段";
        let pages = vec![
            MiddlePage {
                page_idx: 0,
                blocks: vec![MiddleBlock {
                    content: serde_json::json!([{"type": "text", "content": "只有这一段"}]),
                }],
            },
            MiddlePage {
                page_idx: 1,
                blocks: vec![MiddleBlock {
                    content: serde_json::json!([{"type": "text", "content": "这段不在markdown里"}]),
                }],
            },
        ];
        let (out, total) = inject_page_anchors(md, &pages);
        assert_eq!(total, 2, "page count stays truthful");
        assert!(out.contains("<!-- page:1 -->"));
        assert!(
            out.contains("<!-- page:2 -->"),
            "proportional fallback now covers every page"
        );
    }

    #[tokio::test]
    async fn live_parse_when_service_up() {
        // Gated e2e: runs only when the local MinerU container is reachable.
        let config = crate::config::app::KbConfig {
            mineru_url: Some("http://localhost:50053".into()),
            mineru_timeout_secs: 1800,
            ..Default::default()
        };
        let Some(engine) = MineruEngine::from_config(&config) else {
            return;
        };
        if !engine.probe().await {
            println!("mineru not reachable — skipping live parse test");
            return;
        }
        let pdf = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../third/rust-genai/tests/data/small.pdf"
        ))
        .unwrap();
        let opts = ParseOpts {
            extract_images: false,
        };
        let outcome = engine
            .parse(&pdf, "application/pdf", "small.pdf", &opts)
            .await
            .unwrap();
        assert_eq!(outcome.engine, "mineru");
        assert!(
            outcome.markdown.contains("quantum tourism"),
            "markdown must carry the parsed text: {}",
            outcome.markdown
        );
        assert_eq!(outcome.pages, Some(1));
    }
}

#[cfg(test)]
mod jieti_verify {
    use super::*;

    /// 真实 17MB 文档锚点覆盖验证（需 /tmp/jieti.* 产物文件存在）。
    #[test]
    fn verify_real_jieti_anchor_coverage() {
        let Ok(md) = std::fs::read_to_string("/tmp/jieti.md") else {
            println!("SKIP: /tmp/jieti.md not found");
            return;
        };
        let Ok(raw) = std::fs::read_to_string("/tmp/jieti.middle.json") else {
            println!("SKIP: /tmp/jieti.middle.json not found");
            return;
        };
        let middle: MiddleJson = serde_json::from_str(&raw).unwrap();
        println!("pages: {}", middle.pages.len());

        let (out, total) = inject_page_anchors(&md, &middle.pages);
        let count = out.matches("<!-- page:").count();
        println!("anchors: {count} / {total} pages");
        assert_eq!(total, 197);
        assert!(count >= 150, "coverage must be ≥75%: {count}/197");
        assert!(out.contains("<!-- page:1 -->"));
        assert!(out.contains("<!-- page:197 -->"));
    }
}
