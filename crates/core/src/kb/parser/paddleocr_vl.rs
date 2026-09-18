//! PaddleOCR-VL parse engine — self-hosted PaddleX serving pipeline
//! [抄WK:docparser/paddleocr_vl_converter.go].
//!
//! Flow: `POST {endpoint}/layout-parsing` (base64 file + recognition params)
//! → synchronous per-page results (markdown text + inline base64 images).
//!
//! Declared deltas vs WK: no SSRF policy layer (admin-set env); HTML tables
//! are NOT normalized to markdown tables (v1 — WK's html_table_normalizer
//! port is deferred); `useSeal=true` / `useChart=false` WK defaults kept.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::errors::app_error::{AppError, AppResult};

use super::{ParseEngine, ParseOpts, ParseOutcome, ParsedImage};

/// 大型扫描 PDF 的同步解析预算 [抄WK:paddleOCRVLTimeout=1000s].
const PARSE_TIMEOUT_SECS: u64 = 1000;

pub struct PaddleOcrVlEngine {
    endpoint: String,
    http: reqwest::Client,
}

impl PaddleOcrVlEngine {
    /// From `RAISFAST_KB_PADDLEOCR_VL_ENDPOINT`; `None` when unconfigured.
    pub fn from_config(config: &crate::config::app::KbConfig) -> Option<Self> {
        let endpoint = config
            .paddleocr_vl_endpoint
            .as_deref()?
            .trim()
            .trim_end_matches('/');
        if endpoint.is_empty() {
            return None;
        }
        Some(Self {
            endpoint: endpoint.to_string(),
            http: reqwest::Client::new(),
        })
    }

    fn err(&self, context: &str, e: impl std::fmt::Display) -> AppError {
        AppError::ServiceUnavailable(format!("paddleocr_vl {context}: {e}"))
    }
}

/// Recognition / page-restructuring parameters shared by the self-hosted and
/// cloud request bodies [照抄 WK paddleOCRVLRecognitionParams] — keeping both
/// identical reproduces cross-page table merging, heading reconstruction and
/// header/footer stripping.
pub fn recognition_params() -> serde_json::Value {
    json!({
        "markdownIgnoreLabels": ["header", "header_image", "footer", "footer_image",
                                 "number", "footnote", "aside_text"],
        "useDocOrientationClassify": false,
        "useDocUnwarping": false,
        "useLayoutDetection": true,
        "useChartRecognition": false,
        "useSealRecognition": true,
        "useOcrForImageBlock": false,
        "mergeTables": true,
        "relevelTitles": true,
        "restructurePages": true,
        "layoutShapeMode": "auto",
        "promptLabel": "ocr",
        "layoutNms": true,
        "repetitionPenalty": 1,
        "temperature": 0,
        "topP": 1,
        "minPixels": 147384,
        "maxPixels": 2822400,
    })
}

/// 0 = PDF, 1 = image（含 TIFF）[照抄 WK fileTypeCode].
fn file_type_code(filename: &str) -> u8 {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if ext == "pdf" { 0 } else { 1 }
}

/// Decode a `data:image/<ext>;base64,<payload>` URI.
fn decode_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    let (header, payload) = uri.split_once(",")?;
    let ext = header
        .strip_prefix("data:image/")?
        .trim_end_matches(";base64");
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    Some((ext.to_string(), bytes))
}

#[derive(Deserialize)]
struct LayoutResponse {
    error_code: i64,
    #[serde(default)]
    error_msg: String,
    result: Option<LayoutResult>,
}

#[derive(Deserialize)]
struct LayoutResult {
    #[serde(default, rename = "layoutParsingResults")]
    layout_parsing_results: Vec<LayoutParsingResult>,
}

#[derive(Deserialize)]
struct LayoutParsingResult {
    markdown: LayoutMarkdown,
}

#[derive(Deserialize)]
struct LayoutMarkdown {
    #[serde(default)]
    text: String,
    #[serde(default)]
    images: std::collections::BTreeMap<String, String>,
}

#[async_trait::async_trait]
impl ParseEngine for PaddleOcrVlEngine {
    fn name(&self) -> &'static str {
        "paddleocr_vl"
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

    /// The pipeline only exposes POST /layout-parsing — any HTTP response
    /// (even an error status) proves the service is there [照抄 WK Ping].
    async fn probe(&self) -> bool {
        self.http
            .get(&self.endpoint)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }

    async fn parse(
        &self,
        bytes: &[u8],
        _mime: &str,
        filename: &str,
        _opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        use base64::Engine as _;
        let mut payload = recognition_params();
        payload["file"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        payload["fileType"] = json!(file_type_code(filename));
        payload["visualize"] = json!(false);

        let resp = self
            .http
            .post(format!("{}/layout-parsing", self.endpoint))
            .timeout(Duration::from_secs(PARSE_TIMEOUT_SECS))
            .json(&payload)
            .send()
            .await
            .map_err(|e| self.err("layout-parsing", e))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| self.err("layout-parsing read", e))?;
        if !status.is_success() {
            return Err(self.err(
                "layout-parsing",
                format!("HTTP {}: {body}", status.as_u16()),
            ));
        }
        let result: LayoutResponse =
            serde_json::from_str(&body).map_err(|e| self.err("layout-parsing decode", e))?;
        if result.error_code != 0 {
            return Err(self.err(
                "layout-parsing",
                format!("error {}: {}", result.error_code, result.error_msg),
            ));
        }
        let Some(result) = result.result else {
            return Err(self.err("layout-parsing", "response has no result"));
        };

        // Merge per-page markdown + image dicts into one document.
        let mut texts: Vec<String> = Vec::new();
        let mut images: std::collections::BTreeMap<String, String> = Default::default();
        for page in result.layout_parsing_results {
            let t = page.markdown.text.trim().to_string();
            if !t.is_empty() {
                texts.push(t);
            }
            for (path, data) in page.markdown.images {
                images.entry(path).or_insert(data);
            }
        }
        let markdown = texts.join("\n\n");
        let mut out_images: Vec<ParsedImage> = Vec::new();
        for (path, data_uri) in images {
            if let Some((ext, bytes)) = decode_data_uri(&data_uri) {
                out_images.push(ParsedImage {
                    ref_name: path,
                    mime_type: format!("image/{ext}"),
                    bytes,
                });
            }
        }
        let pages = u32::try_from(texts.len()).unwrap_or(0);

        Ok(ParseOutcome {
            markdown,
            images: out_images,
            engine: "paddleocr_vl".into(),
            pages: (pages > 0).then_some(pages),
            scanned_pages: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_requires_endpoint() {
        let mut cfg = crate::config::app::KbConfig::default();
        assert!(PaddleOcrVlEngine::from_config(&cfg).is_none());
        cfg.paddleocr_vl_endpoint = Some("http://127.0.0.1:8080/".into());
        let engine = PaddleOcrVlEngine::from_config(&cfg).unwrap();
        assert_eq!(engine.endpoint, "http://127.0.0.1:8080");
    }

    #[test]
    fn file_type_code_matches_wk() {
        assert_eq!(file_type_code("a.pdf"), 0);
        assert_eq!(file_type_code("scan.tiff"), 1);
        assert_eq!(file_type_code("noext"), 1);
    }
}
