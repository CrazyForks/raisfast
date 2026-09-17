//! Builtin engine — the anydoc pipeline as one engine (E1): markdown
//! passthrough / anydoc conversion / scanned-PDF placeholder degradation,
//! plus embedded-asset extraction for non-PDF document formats. Behavior
//! is identical to the pre-registry pipeline (zero-regression E1
//! acceptance).

use super::{ParseOpts, ParseOutcome, ParsedImage};
use crate::errors::app_error::{AppError, AppResult};

pub struct BuiltinEngine;

#[async_trait::async_trait]
impl super::ParseEngine for BuiltinEngine {
    fn name(&self) -> &'static str {
        "builtin"
    }

    fn supports(&self, mime: &str, filename: &str) -> bool {
        // The builtin takes everything except pure image files (no
        // rasterization ability — image-as-document needs a service engine;
        // kb-parser-engines-design §0).
        !(mime.starts_with("image/") || looks_like_image_ext(filename))
    }

    async fn probe(&self) -> bool {
        true // in-process, always available
    }

    async fn parse(
        &self,
        bytes: &[u8],
        mime: &str,
        filename: &str,
        opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        parse_builtin(bytes, mime, filename, opts)
    }
}

fn looks_like_image_ext(filename: &str) -> bool {
    filename.rsplit('.').next().is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif"
        )
    })
}

/// The pre-registry parse pipeline, verbatim (service.rs `parse_document` +
/// the embedded-asset pass previously done at registration time). Asset
/// images get synthetic ref names — the anydoc String API renders them as
/// alt text only, so they stay doc-level (no chunk association); engine
/// paths keep real refs [抄WK:collectAssets 命名形态, 位置保留=等 anydoc
/// 上游开放项].
pub fn parse_builtin(
    bytes: &[u8],
    mime: &str,
    filename: &str,
    opts: &ParseOpts,
) -> AppResult<ParseOutcome> {
    let is_markdown = mime == "text/markdown" || mime == "text/plain" || filename.ends_with(".md");
    if is_markdown {
        let md = String::from_utf8(bytes.to_vec())
            .map_err(|e| AppError::BadRequest(format!("invalid utf-8 markdown: {e}")))?;
        return Ok(ParseOutcome {
            markdown: md,
            engine: "builtin".into(),
            ..Default::default()
        });
    }
    match anydoc::to_markdown_bytes(bytes, None) {
        Ok(md) => Ok(ParseOutcome {
            markdown: md,
            images: embedded_assets(bytes, opts.extract_images),
            engine: "builtin".into(),
            ..Default::default()
        }),
        Err(anydoc::ConvertError::NeedsOcr { pages, page_count }) => {
            let markdown = crate::kb::service::extract_pdf_skip_ocr(bytes, &pages, page_count)?;
            Ok(ParseOutcome {
                markdown,
                engine: "builtin".into(),
                pages: Some(page_count),
                scanned_pages: pages,
                ..Default::default()
            })
        }
        Err(e) => Err(AppError::BadRequest(format!("document parse failed: {e}"))),
    }
}

/// Embedded image assets of non-PDF document formats via anydoc's
/// Document API (PDF has no document model — declared limitation).
fn embedded_assets(bytes: &[u8], extract: bool) -> Vec<ParsedImage> {
    if !extract {
        return Vec::new();
    }
    match anydoc::to_document(bytes, None) {
        Ok(doc) => doc
            .assets
            .into_iter()
            .enumerate()
            .filter(|(_, a)| a.media_type.starts_with("image/") && !a.bytes.is_empty())
            .map(|(i, a)| {
                let ext = a.media_type.rsplit('/').next().unwrap_or("bin");
                ParsedImage {
                    ref_name: format!("images/image-{}.{ext}", i + 1),
                    mime_type: a.media_type,
                    bytes: a.bytes,
                }
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}
