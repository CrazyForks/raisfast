//! MinerU Cloud parse engine — mineru.net API v4
//! [抄WK:docparser/mineru_cloud_converter.go].
//!
//! Flow: `POST /file-urls/batch` (presigned PUT url) → upload → poll
//! `GET /extract-results/batch/{batch_id}` → inline markdown, or
//! `full_zip_url` (zip: least-nested .md + referenced images).
//!
//! Declared deltas vs WK: no SSRF policy layer (endpoints are admin-set env,
//! not user input); page anchors are unavailable in this API (reader page
//! mapping is empty for docs parsed here) — WK has the same limitation.

use std::time::Duration;

use serde_json::json;

use crate::errors::app_error::{AppError, AppResult};

use super::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};

const DEFAULT_BASE_URL: &str = "https://mineru.net/api/v4";
const POLL_INTERVAL_SECS: u64 = 3;
const CLOUD_TIMEOUT_SECS: u64 = 600;

pub struct MineruCloudEngine {
    api_key: String,
    base_url: String,
    timeout: Duration,
    http: reqwest::Client,
}

impl MineruCloudEngine {
    /// From `RAISFAST_KB_MINERU_CLOUD_API_KEY`; `None` when unconfigured.
    pub fn from_config(config: &crate::config::app::KbConfig) -> Option<Self> {
        let key = config.mineru_cloud_api_key.as_deref()?.trim();
        if key.is_empty() {
            return None;
        }
        Some(Self {
            api_key: key.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(CLOUD_TIMEOUT_SECS),
            http: reqwest::Client::new(),
        })
    }

    fn err(&self, context: &str, e: impl std::fmt::Display) -> AppError {
        AppError::ServiceUnavailable(format!("mineru_cloud {context}: {e}"))
    }

    /// Apply for presigned upload urls: returns `(batch_id, file_url)`.
    async fn apply_upload_urls(&self, filename: &str) -> AppResult<(String, String)> {
        let _ext = filename.rsplit('.').next().unwrap_or("pdf");
        let payload = json!({
            "files": [{
                "name": filename,
                "data_id": crate::utils::id::new_id().to_string(),
            }],
            "model_version": "pipeline",
            "is_ocr": true,
            "enable_formula": true,
            "enable_table": true,
            "language": "ch",
        });
        let resp = self
            .http
            .post(format!("{}/file-urls/batch", self.base_url))
            .timeout(Duration::from_secs(30))
            .bearer_auth(&self.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| self.err("apply upload urls", e))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| self.err("apply upload urls", e))?;
        if !status.is_success() {
            return Err(self.err(
                "apply upload urls",
                format!("HTTP {}: {body}", status.as_u16()),
            ));
        }
        let v: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| self.err("apply upload urls decode", e))?;
        if v["code"].as_i64().unwrap_or(-1) != 0 {
            return Err(self.err(
                "apply upload urls",
                format!("API error: {}", v["msg"].as_str().unwrap_or("unknown")),
            ));
        }
        let batch_id = v["data"]["batch_id"]
            .as_str()
            .ok_or_else(|| self.err("apply upload urls", "missing batch_id"))?
            .to_string();
        let file_url = v["data"]["file_urls"][0]
            .as_str()
            .ok_or_else(|| self.err("apply upload urls", "no file_urls"))?
            .to_string();
        Ok((batch_id, file_url))
    }

    /// Poll the batch until done/failed; returns the first extract item.
    async fn poll_batch(&self, batch_id: &str) -> AppResult<serde_json::Value> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::ServiceUnavailable(format!(
                    "mineru_cloud parse timeout (> {}s)",
                    self.timeout.as_secs()
                )));
            }
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            let resp = self
                .http
                .get(format!(
                    "{}/extract-results/batch/{}",
                    self.base_url, batch_id
                ))
                .timeout(Duration::from_secs(30))
                .bearer_auth(&self.api_key)
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "mineru_cloud poll failed — retrying");
                    continue;
                }
            };
            if !resp.status().is_success() {
                tracing::warn!(status = %resp.status(), "mineru_cloud poll non-success — retrying");
                continue;
            }
            let v: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "mineru_cloud poll decode failed — retrying");
                    continue;
                }
            };
            if v["code"].as_i64().unwrap_or(-1) != 0 {
                return Err(self.err(
                    "poll",
                    format!("code={} msg={}", v["code"], v["msg"].as_str().unwrap_or("")),
                ));
            }
            let items = match v["data"]["extract_result"].as_array() {
                Some(a) if !a.is_empty() => a.clone(),
                _ => continue, // not yet queued — keep polling
            };
            let state = items[0]["state"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase();
            match state.as_str() {
                "failed" => {
                    let msg = items[0]["err_msg"].as_str().unwrap_or("unknown");
                    return Err(AppError::ServiceUnavailable(format!(
                        "mineru_cloud task failed: {msg}"
                    )));
                }
                "done" => return Ok(items[0].clone()),
                _ => {} // waiting / running — keep polling
            }
        }
    }
}

/// Extract (markdown, images) from a result zip: the least-nested `.md` file
/// plus every local image it references [照抄 WK downloadAndExtractZip].
/// Referenced image entry: `(path in markdown, bytes)`.
type ZipImage = (String, Vec<u8>);

fn extract_zip_md_images(zip_bytes: &[u8]) -> AppResult<(String, Vec<ZipImage>)> {
    let cursor = std::io::Cursor::new(zip_bytes.to_vec());
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("zip open: {e}")))?;
    let mut md_files: Vec<String> = Vec::new();
    let mut entries: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Some(name) = archive
            .by_index(i)
            .ok()
            .and_then(|f| Some(f.enclosed_name().map(|p| p.to_string_lossy().to_string()))?)
        {
            if name.ends_with(".md") {
                md_files.push(name.clone());
            }
            entries.push(name);
        }
    }
    if md_files.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!("no .md file in zip")));
    }
    md_files.sort_by_key(|n| n.matches('/').count());
    let md_name = md_files[0].clone();
    let md_dir = md_name
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
        .unwrap_or_default();
    let md_text = {
        let mut f = archive
            .by_name(&md_name)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read {md_name}: {e}")))?;
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut f, &mut buf)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read utf8: {e}")))?;
        buf
    };
    // referenced local images (skip http/data refs), deduped, in ref order.
    let mut images: Vec<(String, Vec<u8>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for caps in markdown_image_refs(&md_text) {
        let ipath = caps;
        if ipath.starts_with("http://")
            || ipath.starts_with("https://")
            || ipath.starts_with("data:")
        {
            continue;
        }
        if !seen.insert(ipath.clone()) {
            continue;
        }
        let full = format!("{md_dir}{ipath}");
        if let Ok(mut f) = archive.by_name(&full) {
            let mut buf = Vec::with_capacity(f.size() as usize);
            std::io::Read::read_to_end(&mut f, &mut buf)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("zip read image: {e}")))?;
            images.push((ipath, buf));
        }
    }
    let _ = entries;
    Ok((md_text, images))
}

/// The local image refs (`![...](path)`) of a markdown document, in order.
fn markdown_image_refs(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = md;
    while let Some(start) = rest.find("](") {
        let Some(close) = rest[start + 2..].find(')') else {
            break;
        };
        let path = rest[start + 2..start + 2 + close].to_string();
        if !path.starts_with("http") && !path.starts_with("data:") {
            out.push(path);
        }
        rest = &rest[start + 2 + close..];
    }
    out
}

#[async_trait::async_trait]
impl ParseEngine for MineruCloudEngine {
    fn name(&self) -> &'static str {
        "mineru_cloud"
    }

    fn supports(&self, mime: &str, filename: &str) -> bool {
        if mime == "application/pdf" {
            return true;
        }
        matches!(
            filename
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str(),
            "jpg" | "jpeg" | "png" | "bmp" | "tiff" | "doc" | "docx" | "ppt" | "pptx"
        )
    }

    /// Ping: an empty-file batch apply — validates the API key cheaply.
    async fn probe(&self) -> bool {
        let resp = self
            .http
            .post(format!("{}/file-urls/batch", self.base_url))
            .timeout(Duration::from_secs(10))
            .bearer_auth(&self.api_key)
            .json(&json!({ "files": [], "model_version": "pipeline" }))
            .send()
            .await;
        matches!(resp, Ok(r) if r.status().is_success())
    }

    async fn parse(
        &self,
        bytes: &[u8],
        _mime: &str,
        filename: &str,
        _opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        let (batch_id, file_url) = self.apply_upload_urls(filename).await?;
        let put = self
            .http
            .put(&file_url)
            .timeout(Duration::from_secs(120))
            .body(bytes.to_vec())
            .send()
            .await
            .map_err(|e| self.err("file upload", e))?;
        if !put.status().is_success() {
            return Err(self.err("file upload", format!("status {}", put.status())));
        }

        let item = self.poll_batch(&batch_id).await?;

        // Prefer inline markdown; fall back to the full result zip.
        let inline = ["markdown", "content", "text"]
            .iter()
            .find_map(|k| item[k].as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty());
        let (markdown, images) = if let Some(md) = inline {
            (md, Vec::new())
        } else if let Some(zip_url) = item["full_zip_url"].as_str() {
            super::validate_external_url(zip_url)?;
            let zip_bytes = self
                .http
                .get(zip_url)
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(|e| self.err("download zip", e))?
                .error_for_status()
                .map_err(|e| self.err("download zip", e))?
                .bytes()
                .await
                .map_err(|e| self.err("download zip read", e))?
                .to_vec();
            let (md, imgs) = extract_zip_md_images(&zip_bytes)?;
            let images = imgs
                .into_iter()
                .map(|(path, bytes)| ParsedImage {
                    ref_name: path,
                    mime_type: "image/*".into(),
                    bytes,
                })
                .collect();
            (md, images)
        } else {
            return Err(self.err(
                "job result",
                "state=done but no markdown/content or full_zip_url",
            ));
        };

        Ok(ParseOutcome {
            markdown,
            images,
            engine: "mineru_cloud".into(),
            pages: None, // cloud API exposes no page mapping on this flow
            scanned_pages: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn from_config_requires_key() {
        let mut cfg = crate::config::app::KbConfig::default();
        assert!(MineruCloudEngine::from_config(&cfg).is_none());
        cfg.mineru_cloud_api_key = Some("k".into());
        assert!(MineruCloudEngine::from_config(&cfg).is_some());
    }

    /// zip 产物抽取：最浅层 .md + 其引用的本地图片。
    #[test]
    fn extract_zip_picks_shallowest_md_and_referenced_images() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = Default::default();
            w.start_file("nested/deep.md", opts).unwrap();
            w.write_all(b"# deep").unwrap();
            w.start_file("out.md", opts).unwrap();
            w.write_all("# 标题\n\n![图](images/a.png)\n\n![外链](https://x/y.png)".as_bytes())
                .unwrap();
            w.start_file("images/a.png", opts).unwrap();
            w.write_all(b"PNGDATA").unwrap();
            w.finish().unwrap();
        }
        let (md, images) = extract_zip_md_images(&buf.into_inner()).unwrap();
        assert!(md.contains("# 标题"), "must pick the shallowest md: {md}");
        assert_eq!(images.len(), 1, "external refs must be skipped");
        assert_eq!(images[0].0, "images/a.png");
        assert_eq!(images[0].1, b"PNGDATA");
    }
}
