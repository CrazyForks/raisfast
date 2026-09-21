//! docreader parse engine — WeKnora's MIT docreader service over gRPC
//! (kb-parser-engines-design v4 E2). File bytes in (`ReadRequest.file_content`),
//! markdown + inline image bytes out (`ReadStream` frames); no object
//! storage involved [抄WK:docreader/proto/docreader.proto].

use std::time::Duration;

use crate::errors::app_error::{AppError, AppResult};

use super::{ParseOpts, ParseOutcome, ParsedImage};

pub struct DocreaderEngine {
    endpoint: String,
    timeout: Duration,
    /// Lazily-connected channel (probe failure recreates it on next call).
    channel: std::sync::OnceLock<tonic::transport::Channel>,
}

impl DocreaderEngine {
    /// From `RAISFAST_KB_DOCREADER_URL`; `None` when unconfigured (the
    /// engine then never registers).
    pub fn from_config(config: &crate::config::app::KbConfig) -> Option<Self> {
        let url = config.docreader_url.as_deref()?.trim().to_string();
        if url.is_empty() {
            return None;
        }
        Some(Self {
            endpoint: normalize_endpoint(&url),
            timeout: Duration::from_secs(config.docreader_timeout_secs),
            channel: std::sync::OnceLock::new(),
        })
    }

    pub fn new(endpoint: impl Into<String>, timeout_secs: u64) -> Self {
        Self {
            endpoint: normalize_endpoint(&endpoint.into()),
            timeout: Duration::from_secs(timeout_secs),
            channel: std::sync::OnceLock::new(),
        }
    }

    async fn channel(&self) -> AppResult<tonic::transport::Channel> {
        if let Some(ch) = self.channel.get() {
            return Ok(ch.clone());
        }
        let ch = tonic::transport::Channel::from_shared(self.endpoint.clone())
            .map_err(|e| AppError::ServiceUnavailable(format!("docreader endpoint: {e}")))?
            .connect()
            .await
            .map_err(|e| AppError::ServiceUnavailable(format!("docreader connect: {e}")))?;
        let _ = self.channel.set(ch.clone());
        Ok(ch)
    }
}

/// tonic needs a full URL; bare host:port gets an http:// scheme.
fn normalize_endpoint(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_string()
    } else {
        format!("http://{url}")
    }
}

/// File-type surface of the docreader service (§0 格式盘点)：PDF、office
/// 全家、epub/html、图片即文档。Simple text formats stay on builtin.
fn supported_type(mime: &str, filename: &str) -> bool {
    if mime.starts_with("image/") {
        return true;
    }
    const EXTS: &[&str] = &[
        "pdf", "doc", "docx", "odt", "rtf", "epub", "ppt", "pptx", "pps", "xls", "xlsx", "html",
        "mhtml",
    ];
    filename
        .rsplit('.')
        .next()
        .is_some_and(|ext| EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

#[async_trait::async_trait]
impl super::ParseEngine for DocreaderEngine {
    fn name(&self) -> &'static str {
        "docreader"
    }

    fn supports(&self, mime: &str, filename: &str) -> bool {
        supported_type(mime, filename)
    }

    async fn probe(&self) -> bool {
        // ListEngines as liveness probe (part of the docreader service).
        let Ok(channel) = self.channel().await else {
            return false;
        };
        let mut client = super::docreader_proto::DocReaderClient::new(channel);
        tokio::time::timeout(self.timeout, async {
            client
                .list_engines(super::docreader_proto::pb::ListEnginesRequest {
                    config_overrides: std::collections::HashMap::new(),
                })
                .await
                .map(|_| ())
        })
        .await
        .is_ok_and(|r| r.is_ok())
    }

    async fn parse(
        &self,
        _bytes: &[u8],
        mime: &str,
        filename: &str,
        opts: &ParseOpts,
    ) -> AppResult<ParseOutcome> {
        let channel = self.channel().await?;
        let mut client = super::docreader_proto::DocReaderClient::new(channel);
        let file_type = filename.rsplit('.').next().unwrap_or_default().to_string();
        let request = super::docreader_proto::pb::ReadRequest {
            file_content: _bytes.to_vec(),
            file_name: filename.to_string(),
            file_type,
            url: String::new(),
            title: String::new(),
            config: None,
            request_id: crate::utils::id::new_id().to_string(),
        };
        // ReadStream: meta frame first (markdown), then one frame per image
        // — bounded memory for large scanned PDFs [抄WK:proto ReadStream].
        let stream = tokio::time::timeout(self.timeout, client.read_stream(request))
            .await
            .map_err(|_| {
                AppError::ServiceUnavailable(format!(
                    "docreader parse timeout (> {}s)",
                    self.timeout.as_secs()
                ))
            })?
            .map_err(|e| AppError::ServiceUnavailable(format!("docreader rpc: {e}")))?
            .into_inner();

        let mut outcome = ParseOutcome {
            engine: "docreader".into(),
            ..Default::default()
        };
        let mut stream = stream;
        loop {
            let frame = tokio::time::timeout(self.timeout, stream.message())
                .await
                .map_err(|_| AppError::ServiceUnavailable("docreader stream timeout".into()))?
                .map_err(|e| AppError::ServiceUnavailable(format!("docreader stream: {e}")))?;
            let Some(frame) = frame else { break };
            match frame.payload {
                Some(super::docreader_proto::pb::read_stream_response::Payload::Meta(meta)) => {
                    if !meta.error.is_empty() {
                        return Err(AppError::ServiceUnavailable(format!(
                            "docreader: {}",
                            meta.error
                        )));
                    }
                    outcome.markdown = meta.markdown_content;
                    if let Some(pages) = meta.metadata.get("page_count")
                        && let Ok(pages) = pages.parse::<u32>()
                    {
                        outcome.pages = Some(pages);
                    }
                }
                Some(super::docreader_proto::pb::read_stream_response::Payload::Image(img))
                    if opts.extract_images && !img.image_data.is_empty() =>
                {
                    outcome.images.push(ParsedImage {
                        ref_name: if img.filename.is_empty() {
                            format!("image-{}.bin", outcome.images.len() + 1)
                        } else {
                            img.filename.clone()
                        },
                        mime_type: if img.mime_type.is_empty() {
                            "image/png".into()
                        } else {
                            img.mime_type.clone()
                        },
                        bytes: img.image_data,
                    });
                }
                // Guarded image arm above catches keeper images; this arm
                // covers the filtered-out ones (extract off / empty bytes).
                Some(super::docreader_proto::pb::read_stream_response::Payload::Image(_)) => {}
                None => {}
            }
        }
        if outcome.markdown.trim().is_empty() && outcome.images.is_empty() {
            return Err(AppError::ServiceUnavailable(
                "docreader: empty result (no markdown, no images)".into(),
            ));
        }
        let _ = mime;
        Ok(outcome)
    }
}
