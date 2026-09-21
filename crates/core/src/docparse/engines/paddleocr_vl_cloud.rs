//! PaddleOCR-VL Cloud parse engine — AI Studio hosted API
//! [抄WK:docparser/paddleocr_vl_cloud_converter.go].
//!
//! Flow: `POST {base}` (multipart: model + optionalPayload + file) → poll
//! `GET {base}/{job_id}` → `resultUrl.jsonUrl` JSONL → per-page markdown +
//! image URLs (downloaded and attached to the refs that use them).
//!
//! Declared deltas vs WK: no SSRF policy layer (admin-set env); no HTML
//! table normalization (v1); probe validates token presence only (the API
//! has no lightweight health endpoint — same as WK).

use std::time::Duration;

use serde::Deserialize;

use crate::errors::app_error::{AppError, AppResult};

use super::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};

const DEFAULT_BASE_URL: &str = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";
const POLL_INTERVAL_SECS: u64 = 5;
const CLOUD_TIMEOUT_SECS: u64 = 600;

pub struct PaddleOcrVlCloudEngine {
    base_url: String,
    token: String,
    timeout: Duration,
    http: reqwest::Client,
}

impl PaddleOcrVlCloudEngine {
    /// From `RAISFAST_KB_PADDLEOCR_VL_CLOUD_TOKEN`; `None` when unconfigured.
    pub fn from_config(config: &crate::config::app::KbConfig) -> Option<Self> {
        let token = config.paddleocr_vl_cloud_token.as_deref()?.trim();
        if token.is_empty() {
            return None;
        }
        Some(Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            token: token.to_string(),
            timeout: Duration::from_secs(CLOUD_TIMEOUT_SECS),
            http: reqwest::Client::new(),
        })
    }

    fn err(&self, context: &str, e: impl std::fmt::Display) -> AppError {
        AppError::ServiceUnavailable(format!("paddleocr_vl_cloud {context}: {e}"))
    }

    async fn submit_job(&self, filename: &str, bytes: &[u8]) -> AppResult<String> {
        let optional = super::paddleocr_vl::recognition_params();
        let file_name = if filename.is_empty() {
            "document.pdf".to_string()
        } else {
            filename.to_string()
        };
        let form = self
            .http
            .post(&self.base_url)
            .timeout(Duration::from_secs(60))
            .header("Authorization", format!("bearer {}", self.token))
            .multipart(reqwest::multipart::Form::new().part(
                "file",
                reqwest::multipart::Part::bytes(bytes.to_vec()).file_name(file_name),
            ))
            .query(&[
                ("model", "PaddleOCR-VL"),
                ("optionalPayload", optional.to_string().as_str()),
            ])
            .send()
            .await
            .map_err(|e| self.err("submit job", e))?;
        let status = form.status();
        let body = form
            .text()
            .await
            .map_err(|e| self.err("submit job read", e))?;
        if !status.is_success() {
            return Err(self.err("submit job", format!("HTTP {}: {body}", status.as_u16())));
        }
        let v: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| self.err("submit job decode", e))?;
        let job_id = v["data"]["jobId"]
            .as_str()
            .ok_or_else(|| self.err("submit job", format!("no jobId: {body}")))?
            .to_string();
        Ok(job_id)
    }

    /// Poll the job; on success returns the result JSONL url.
    async fn poll_job(&self, job_id: &str) -> AppResult<String> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(AppError::ServiceUnavailable(format!(
                    "paddleocr_vl_cloud parse timeout (> {}s)",
                    self.timeout.as_secs()
                )));
            }
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            let resp = self
                .http
                .get(format!("{}/{}", self.base_url, job_id))
                .timeout(Duration::from_secs(30))
                .send()
                .await;
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(error = %e, "paddleocr_vl_cloud poll failed — retrying");
                    continue;
                }
            };
            if !resp.status().is_success() {
                let status = resp.status();
                let _ = resp.text().await;
                tracing::warn!(status = %status, "paddleocr_vl_cloud poll non-success — retrying");
                continue;
            }
            let v: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(error = %e, "paddleocr_vl_cloud poll decode failed — retrying");
                    continue;
                }
            };
            match v["data"]["status"].as_str().unwrap_or_default() {
                "done" | "success" | "completed" => {
                    let url = v["data"]["resultUrl"]["jsonUrl"]
                        .as_str()
                        .or_else(|| v["data"]["resultUrl"]["url"].as_str())
                        .ok_or_else(|| self.err("poll", "done but no resultUrl.jsonUrl"))?;
                    return Ok(url.to_string());
                }
                "failed" => {
                    let msg = v["data"]["errorMsg"].as_str().unwrap_or("unknown");
                    return Err(AppError::ServiceUnavailable(format!(
                        "paddleocr_vl_cloud task failed: {msg}"
                    )));
                }
                _ => {} // queued / running
            }
        }
    }
}

/// One JSONL line of the result file.
#[derive(Deserialize)]
struct ResultLine {
    #[serde(default)]
    result: Option<ResultLinePayload>,
}

#[derive(Deserialize)]
struct ResultLinePayload {
    #[serde(default, rename = "layoutParsingResults")]
    layout_parsing_results: Vec<ResultLinePage>,
}

#[derive(Deserialize)]
struct ResultLinePage {
    markdown: ResultLineMarkdown,
}

#[derive(Deserialize)]
struct ResultLineMarkdown {
    #[serde(default)]
    text: String,
    #[serde(default)]
    images: std::collections::BTreeMap<String, String>,
}

/// Parse the result JSONL: per-page markdown texts + image path → URL map
/// [照抄 WK fetchResults].
fn parse_jsonl(data: &str) -> (Vec<String>, std::collections::BTreeMap<String, String>) {
    let mut texts: Vec<String> = Vec::new();
    let mut images: std::collections::BTreeMap<String, String> = Default::default();
    for line in data.trim().split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(parsed) = serde_json::from_str::<ResultLine>(line) else {
            tracing::warn!("paddleocr_vl_cloud: skipping malformed jsonl line");
            continue;
        };
        let Some(result) = parsed.result else {
            continue;
        };
        for page in result.layout_parsing_results {
            let t = page.markdown.text.trim().to_string();
            if !t.is_empty() {
                texts.push(t);
            }
            for (path, url) in page.markdown.images {
                images.entry(path).or_insert(url);
            }
        }
    }
    (texts, images)
}

#[async_trait::async_trait]
impl ParseEngine for PaddleOcrVlCloudEngine {
    fn name(&self) -> &'static str {
        "paddleocr_vl_cloud"
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
            "jpg" | "jpeg" | "png" | "bmp" | "tiff"
        )
    }

    /// The API has no lightweight health endpoint — token presence is the
    /// only cheap check [照抄 WK PingPaddleOCRVLCloud].
    async fn probe(&self) -> bool {
        !self.token.trim().is_empty()
    }

    async fn parse(
        &self,
        bytes: &[u8],
        _mime: &str,
        filename: &str,
        _opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        let job_id = self.submit_job(filename, bytes).await?;
        let jsonl_url = self.poll_job(&job_id).await?;
        super::validate_external_url(&jsonl_url)?;
        let resp = self
            .http
            .get(&jsonl_url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| self.err("download jsonl", e))?
            .error_for_status()
            .map_err(|e| self.err("download jsonl", e))?;
        let data = resp
            .text()
            .await
            .map_err(|e| self.err("download jsonl read", e))?;
        let (texts, image_urls) = parse_jsonl(&data);
        let markdown = texts.join("\n\n");
        let pages = u32::try_from(texts.len()).unwrap_or(0);

        // Download only the image urls the markdown actually references.
        let mut images: Vec<ParsedImage> = Vec::new();
        for (path, url) in &image_urls {
            if !markdown.contains(path.as_str()) {
                continue;
            }
            super::validate_external_url(url)?;
            if let Ok(img) = self
                .http
                .get(url)
                .timeout(Duration::from_secs(60))
                .send()
                .await
                .and_then(|r| r.error_for_status())
                && let Ok(b) = img.bytes().await
            {
                images.push(ParsedImage {
                    ref_name: path.clone(),
                    mime_type: "image/*".into(),
                    bytes: b.to_vec(),
                });
            }
        }

        Ok(ParseOutcome {
            markdown,
            images,
            engine: "paddleocr_vl_cloud".into(),
            pages: (pages > 0).then_some(pages),
            scanned_pages: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_requires_token() {
        let mut cfg = crate::config::app::KbConfig::default();
        assert!(PaddleOcrVlCloudEngine::from_config(&cfg).is_none());
        cfg.paddleocr_vl_cloud_token = Some("t".into());
        assert!(PaddleOcrVlCloudEngine::from_config(&cfg).is_some());
    }

    #[test]
    fn jsonl_pages_merge_and_images_dedup() {
        let data = concat!(
            r#"{"result":{"layoutParsingResults":[{"markdown":{"text":"第一页","images":{"imgs/a.png":"http://x/a.png"}}}]}}"#,
            "\n",
            r#"{"result":{"layoutParsingResults":[{"markdown":{"text":"第二页","images":{"imgs/a.png":"http://x/a.png"}}}]}}"#,
            "\n",
            "not json\n",
        );
        let (texts, images) = parse_jsonl(data);
        assert_eq!(texts, vec!["第一页", "第二页"]);
        assert_eq!(images.len(), 1);
    }
}
