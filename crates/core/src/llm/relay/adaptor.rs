//! Relay adaptors (design §8.2): OpenAI canonical format in, per-provider
//! conversion out. The openai-compatible adaptor is the default (deepseek/
//! moonshot/ollama/siliconflow/generic all ride it); anthropic-native
//! channels branch to `relay::anthropic::AnthropicAdaptor` by `provider`.

use std::pin::Pin;

use futures::Stream;
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};

use crate::errors::app_error::{AppError, AppResult};

/// Normalized usage (design §9.3 normalization contract:
/// `prompt_tokens` includes cache; OpenAI/DeepSeek are native, the anthropic
/// adaptor must sum its split fields).
#[derive(Debug, Clone, Default)]
pub struct RelayUsage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

impl RelayUsage {
    /// Parse an OpenAI-shaped usage object.
    pub fn from_openai(v: &serde_json::Value) -> Self {
        let get = |name: &str| v.get(name).and_then(|x| x.as_i64()).unwrap_or(0);
        let cache_read = v
            .get("prompt_tokens_details")
            .and_then(|d| d.get("cached_tokens"))
            .and_then(|x| x.as_i64())
            .or_else(|| v.get("prompt_cache_hit_tokens").and_then(|x| x.as_i64()))
            .unwrap_or(0);
        Self {
            prompt_tokens: get("prompt_tokens"),
            completion_tokens: get("completion_tokens"),
            cache_read_tokens: cache_read,
            cache_write_tokens: 0,
        }
    }

    /// Parse an Anthropic-shaped usage object (§9.3 normalization contract:
    /// `prompt_tokens` includes cache — anthropic reports input and the cache
    /// splits separately, so the splits are summed back into the prompt side).
    pub fn from_anthropic(v: &serde_json::Value) -> Self {
        let get = |name: &str| v.get(name).and_then(|x| x.as_i64()).unwrap_or(0);
        let cache_read = get("cache_read_input_tokens");
        let cache_write = get("cache_creation_input_tokens");
        Self {
            prompt_tokens: get("input_tokens") + cache_read + cache_write,
            completion_tokens: get("output_tokens"),
            cache_read_tokens: cache_read,
            cache_write_tokens: cache_write,
        }
    }

    /// Fallback estimation when the upstream reports no usage (design §8.4):
    /// chars / 4 per side.
    pub fn estimate(prompt_chars: usize, completion_chars: usize) -> Self {
        Self {
            prompt_tokens: (prompt_chars / 4) as i64,
            completion_tokens: (completion_chars / 4) as i64,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }
}

/// Upstream endpoint selector.
#[derive(Debug, Clone, Copy)]
pub enum RelayEndpoint {
    ChatCompletions,
    /// Reserved for the proxied models listing (P4).
    #[allow(dead_code)]
    Models,
    /// `/embeddings` — OpenAI-compatible vectorization.
    Embeddings,
    /// `/rerank` — Jina/Cohere-style relevance reranking.
    Rerank,
    /// `/images/generations` — OpenAI-compatible text-to-image.
    Images,
    /// `/audio/transcriptions` — speech-to-text (multipart).
    AudioTranscriptions,
    /// `/audio/translations` — speech-to-text translated to English (multipart).
    AudioTranslations,
    /// `/audio/speech` — text-to-speech (JSON in, binary audio out).
    AudioSpeech,
    /// `/videos` — async video generation (OpenAI Videos API shape). The
    /// `/{id}` and `/{id}/content` suffixes are appended by the caller.
    Videos,
}

/// One shared HTTP client for the whole module (design §7.6: per-host
/// connection reuse + h2 multiplexing; one client per provider would cause
/// handshake storms under load).
pub fn shared_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .unwrap_or_default()
    })
}

/// Apply channel header overrides onto the request headers.
pub fn apply_header_override(headers: &mut HeaderMap, override_hdr: Option<&serde_json::Value>) {
    if let Some(serde_json::Value::Object(map)) = override_hdr {
        for (k, v) in map {
            if let Some(vs) = v.as_str()
                && let (Ok(name), Ok(value)) =
                    (HeaderName::try_from(k.as_str()), HeaderValue::try_from(vs))
            {
                headers.insert(name, value);
            }
        }
    }
}

/// §6.2 reset-signal sources beyond message keywords: `Retry-After`
/// (delta-seconds / HTTP-date) plus `anthropic-ratelimit-*-reset` (RFC 3339)
/// and `x-ratelimit-reset*` headers. Returns the remaining window duration;
/// `None` when no parseable signal exists (callers keep transient handling).
/// Past deadlines yield `None` — an expired window is not a signal.
pub(crate) fn parse_reset_deadline(headers: &HeaderMap) -> Option<std::time::Duration> {
    const DAY_SECS: i64 = 24 * 60 * 60;
    let now = crate::utils::tz::now_utc();

    fn parse_value(v: &str, now: chrono::DateTime<chrono::Utc>) -> Option<i64> {
        let v = v.trim();
        if let Ok(n) = v.parse::<i64>() {
            // Bare number: relative seconds when small, unix epoch when
            // large (GitHub style) — epochs have exceeded DAY_SECS since
            // 2001-09-09, so the split is unambiguous.
            if n > DAY_SECS {
                let remaining = n - now.timestamp();
                return (remaining > 0).then_some(remaining);
            }
            return (n > 0).then_some(n);
        }
        // Retry-After HTTP-date (RFC 1123/2822) / anthropic ISO 8601.
        let parsed = chrono::DateTime::parse_from_rfc2822(v)
            .ok()
            .map(|t| t.with_timezone(&chrono::Utc))
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(v)
                    .ok()
                    .map(|t| t.with_timezone(&chrono::Utc))
            });
        if let Some(t) = parsed {
            let secs = (t - now).num_seconds();
            return (secs > 0).then_some(secs);
        }
        None
    }

    if let Some(secs) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_value(v, now))
    {
        return Some(std::time::Duration::from_secs(secs as u64));
    }
    // First parseable reset header wins (they share the deadline).
    for (name, value) in headers.iter() {
        let n = name.as_str();
        let is_reset = (n.starts_with("anthropic-ratelimit-") && n.ends_with("-reset"))
            || n.starts_with("x-ratelimit-reset");
        if is_reset && let Some(secs) = value.to_str().ok().and_then(|v| parse_value(v, now)) {
            return Some(std::time::Duration::from_secs(secs as u64));
        }
    }
    None
}

/// The OpenAI-compatible adaptor (near-passthrough).
pub struct OpenaiAdaptor;

impl OpenaiAdaptor {
    /// Upstream URL for an endpoint.
    pub fn request_url(base_url: &str, endpoint: RelayEndpoint) -> String {
        match endpoint {
            RelayEndpoint::ChatCompletions => format!("{base_url}/chat/completions"),
            RelayEndpoint::Models => format!("{base_url}/models"),
            RelayEndpoint::Embeddings => format!("{base_url}/embeddings"),
            RelayEndpoint::Rerank => format!("{base_url}/rerank"),
            RelayEndpoint::Images => format!("{base_url}/images/generations"),
            RelayEndpoint::AudioTranscriptions => format!("{base_url}/audio/transcriptions"),
            RelayEndpoint::AudioTranslations => format!("{base_url}/audio/translations"),
            RelayEndpoint::AudioSpeech => format!("{base_url}/audio/speech"),
            RelayEndpoint::Videos => format!("{base_url}/videos"),
        }
    }

    /// Auth headers + channel overrides.
    pub fn setup_headers(key: &str, header_override: Option<&serde_json::Value>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Ok(value) = HeaderValue::try_from(format!("Bearer {key}")) {
            headers.insert(AUTHORIZATION, value);
        }
        apply_header_override(&mut headers, header_override);
        headers
    }

    /// Convert a request body (design §8.3): upstream model rewrite →
    /// param_override shallow merge → stream_options.include_usage key-level
    /// merge (admin override wins).
    pub fn convert_chat(
        mut body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
        stream: bool,
    ) -> AppResult<serde_json::Value> {
        let Some(obj) = body.as_object_mut() else {
            return Err(AppError::BadRequest(
                "request body must be an object".to_owned(),
            ));
        };
        obj.insert(
            "model".to_owned(),
            serde_json::Value::String(upstream_model.to_owned()),
        );
        if let Some(serde_json::Value::Object(over)) = param_override {
            for (k, v) in over {
                obj.insert(k.clone(), v.clone());
            }
        }
        if stream {
            // §8.3 merge semantics: `include_usage` injection is KEY-LEVEL
            // (client's other stream_options keys survive); an explicit
            // `stream_options` in param_override replaces wholesale (admin
            // intent wins) — the shallow merge above already did that.
            let admin_forces = param_override
                .and_then(|p| p.get("stream_options"))
                .is_some();
            if !admin_forces {
                let mut so = obj
                    .get("stream_options")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                if !so.is_object() {
                    so = serde_json::json!({});
                }
                if let Some(so_obj) = so.as_object_mut() {
                    so_obj.insert("include_usage".to_owned(), serde_json::Value::Bool(true));
                }
                obj.insert("stream_options".to_owned(), so);
            }
        }
        Ok(body)
    }

    /// Convert a non-chat request body (embeddings / rerank): upstream model
    /// rewrite + param_override shallow merge. No stream handling — these
    /// modalities are always request/response JSON.
    pub fn convert_plain(
        mut body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
    ) -> AppResult<serde_json::Value> {
        let Some(obj) = body.as_object_mut() else {
            return Err(AppError::BadRequest(
                "request body must be an object".to_owned(),
            ));
        };
        obj.insert(
            "model".to_owned(),
            serde_json::Value::String(upstream_model.to_owned()),
        );
        if let Some(serde_json::Value::Object(over)) = param_override {
            for (k, v) in over {
                obj.insert(k.clone(), v.clone());
            }
        }
        Ok(body)
    }

    /// Non-stream response: full body + usage extraction.
    pub async fn handle_response(
        resp: reqwest::Response,
    ) -> AppResult<(serde_json::Value, RelayUsage)> {
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("read upstream: {e}")))?;
        let body: serde_json::Value = serde_json::from_str(&text).map_err(|_| {
            AppError::Internal(anyhow::anyhow!(
                "upstream non-JSON body: {}",
                &text[..text.len().min(200)]
            ))
        })?;
        if !status.is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "upstream {}: {}",
                status.as_u16(),
                text.chars().take(500).collect::<String>()
            )));
        }
        let usage = body
            .get("usage")
            .map(RelayUsage::from_openai)
            .unwrap_or_default();
        Ok((body, usage))
    }

    /// Stream response: an SSE byte-frame stream that forwards frames
    /// verbatim while accumulating usage. Callers attach settlement to the
    /// stream's termination (settle-on-complete, design §8.2/8.4).
    /// `content_chars_out` accumulates delta-content char counts — the
    /// completion side of the no-usage estimation fallback (§8.4).
    pub fn handle_stream(
        resp: reqwest::Response,
        usage_out: std::sync::Arc<std::sync::Mutex<RelayUsage>>,
        content_chars_out: std::sync::Arc<std::sync::Mutex<usize>>,
    ) -> Pin<Box<dyn Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>> {
        Box::pin(SseForwardStream::new(resp, usage_out, content_chars_out))
    }
}

/// Line-buffered SSE forwarding stream (design §8.4): forwards every raw
/// frame, parses `data:` payloads only to accumulate usage and content
/// chars.
struct SseForwardStream {
    upstream: Pin<Box<dyn Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>,
    buffer: String,
    usage: std::sync::Arc<std::sync::Mutex<RelayUsage>>,
    content_chars: std::sync::Arc<std::sync::Mutex<usize>>,
    done: bool,
}

impl SseForwardStream {
    fn new(
        resp: reqwest::Response,
        usage: std::sync::Arc<std::sync::Mutex<RelayUsage>>,
        content_chars: std::sync::Arc<std::sync::Mutex<usize>>,
    ) -> Self {
        use futures::StreamExt;
        Self {
            upstream: Box::pin(resp.bytes_stream().map(|r| r.map(|b| b.to_vec()))),
            buffer: String::new(),
            usage,
            content_chars,
            done: false,
        }
    }

    fn feed_line(&mut self, line: &str) {
        let data = line.strip_prefix("data:").map(str::trim);
        if let Some(data) = data {
            if data == "[DONE]" {
                return;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                if v.get("usage").is_some_and(|u| !u.is_null()) {
                    let parsed = RelayUsage::from_openai(v.get("usage").unwrap_or(&v));
                    if let Ok(mut u) = self.usage.lock() {
                        *u = parsed;
                    }
                }
                if let Some(choices) = v.get("choices").and_then(|c| c.as_array()) {
                    let delta_chars: usize = choices
                        .iter()
                        .filter_map(|c| {
                            c.get("delta")
                                .and_then(|d| d.get("content"))
                                .and_then(|s| s.as_str())
                        })
                        .map(|s| s.chars().count())
                        .sum();
                    if delta_chars > 0
                        && let Ok(mut total) = self.content_chars.lock()
                    {
                        *total += delta_chars;
                    }
                }
            }
        }
    }
}

impl Stream for SseForwardStream {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        use futures::StreamExt;
        if self.done {
            return std::task::Poll::Ready(None);
        }
        match self.upstream.poll_next_unpin(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                let text = String::from_utf8_lossy(&chunk).to_string();
                self.buffer.push_str(&text);
                let mut out = Vec::new();
                while let Some(pos) = self.buffer.find('\n') {
                    let line: String = self.buffer.drain(..=pos).collect();
                    let trimmed_end = line.trim_end_matches(['\n', '\r']);
                    self.feed_line(trimmed_end);
                    out.extend_from_slice(line.as_bytes());
                }
                if out.is_empty() {
                    cx.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                std::task::Poll::Ready(Some(Ok(out)))
            }
            std::task::Poll::Ready(Some(Err(e))) => {
                self.done = true;
                std::task::Poll::Ready(Some(Err(std::io::Error::other(e))))
            }
            std::task::Poll::Ready(None) => {
                self.done = true;
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(model: &str) -> serde_json::Value {
        serde_json::json!({ "model": model, "messages": [] })
    }

    #[test]
    fn model_rewritten_to_upstream_name() {
        let out = OpenaiAdaptor::convert_chat(body("public"), "upstream-x", None, false)
            .expect("convert");
        assert_eq!(out["model"], "upstream-x");
    }

    #[test]
    fn non_object_body_rejected() {
        assert!(OpenaiAdaptor::convert_chat(serde_json::json!([]), "m", None, false).is_err());
    }

    #[test]
    fn param_override_shallow_merges() {
        let over = serde_json::json!({ "temperature": 0.1 });
        let mut b = body("m");
        b["temperature"] = serde_json::json!(0.9);
        let out = OpenaiAdaptor::convert_chat(b, "m", Some(&over), false).expect("convert");
        assert_eq!(out["temperature"], 0.1);
    }

    #[test]
    fn non_stream_never_injects_stream_options() {
        let out = OpenaiAdaptor::convert_chat(body("m"), "m", None, false).expect("convert");
        assert!(out.get("stream_options").is_none());
    }

    #[test]
    fn include_usage_injection_is_key_level() {
        let mut b = body("m");
        b["stream"] = serde_json::json!(true);
        b["stream_options"] = serde_json::json!({ "x_custom": 42 });
        let out = OpenaiAdaptor::convert_chat(b, "m", None, true).expect("convert");
        assert_eq!(out["stream_options"]["include_usage"], true);
        assert_eq!(
            out["stream_options"]["x_custom"], 42,
            "client keys must survive (§8.3)"
        );
    }

    #[test]
    fn admin_stream_options_override_wins() {
        let over = serde_json::json!({ "stream_options": { "include_usage": false } });
        let mut b = body("m");
        b["stream"] = serde_json::json!(true);
        let out = OpenaiAdaptor::convert_chat(b, "m", Some(&over), true).expect("convert");
        assert_eq!(out["stream_options"]["include_usage"], false);
    }

    #[test]
    fn request_urls() {
        assert_eq!(
            OpenaiAdaptor::request_url("https://x/v1", RelayEndpoint::ChatCompletions),
            "https://x/v1/chat/completions"
        );
        assert_eq!(
            OpenaiAdaptor::request_url("https://x/v1", RelayEndpoint::Models),
            "https://x/v1/models"
        );
    }

    #[test]
    fn usage_from_openai_variants() {
        let openai = serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 40,
            "prompt_tokens_details": { "cached_tokens": 60 }
        });
        let u = RelayUsage::from_openai(&openai);
        assert_eq!((u.prompt_tokens, u.cache_read_tokens), (100, 60));

        let deepseek = serde_json::json!({
            "prompt_tokens": 100,
            "completion_tokens": 40,
            "prompt_cache_hit_tokens": 70
        });
        let u2 = RelayUsage::from_openai(&deepseek);
        assert_eq!(u2.cache_read_tokens, 70);

        let bare = serde_json::json!({ "prompt_tokens": 10, "completion_tokens": 5 });
        let u3 = RelayUsage::from_openai(&bare);
        assert_eq!((u3.prompt_tokens, u3.cache_read_tokens), (10, 0));
    }

    #[test]
    fn usage_from_anthropic_sums_cache_splits() {
        // §9.3: prompt side = input + cache_read + cache_write.
        let v = serde_json::json!({
            "input_tokens": 100,
            "output_tokens": 40,
            "cache_read_input_tokens": 60,
            "cache_creation_input_tokens": 10
        });
        let u = RelayUsage::from_anthropic(&v);
        assert_eq!(u.prompt_tokens, 170);
        assert_eq!(u.completion_tokens, 40);
        assert_eq!(u.cache_read_tokens, 60);
        assert_eq!(u.cache_write_tokens, 10);

        let bare = serde_json::json!({ "input_tokens": 10, "output_tokens": 5 });
        let u2 = RelayUsage::from_anthropic(&bare);
        assert_eq!((u2.prompt_tokens, u2.completion_tokens), (10, 5));
        assert_eq!((u2.cache_read_tokens, u2.cache_write_tokens), (0, 0));
    }

    #[test]
    fn usage_estimate_fallback() {
        let u = RelayUsage::estimate(400, 80);
        assert_eq!((u.prompt_tokens, u.completion_tokens), (100, 20));
    }

    #[test]
    fn header_override_applies() {
        let over = serde_json::json!({ "x-custom": "v", "not-a-string": 9 });
        let h = OpenaiAdaptor::setup_headers("k", Some(&over));
        assert_eq!(h.get("x-custom").unwrap(), "v");
        assert_eq!(h.get(reqwest::header::AUTHORIZATION).unwrap(), "Bearer k");
    }

    // ── reset-deadline header parsing（§6.2 头信号）─────────────────

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                HeaderName::try_from(*k).unwrap(),
                HeaderValue::try_from(*v).unwrap(),
            );
        }
        h
    }

    fn in_minutes(mins: i64) -> String {
        let t = crate::utils::tz::now_utc() + chrono::Duration::minutes(mins);
        t.with_timezone(&chrono::FixedOffset::east_opt(0).unwrap())
            .to_rfc2822()
    }

    #[test]
    fn retry_after_delta_seconds() {
        let h = hm(&[("retry-after", "18000")]);
        assert_eq!(
            parse_reset_deadline(&h),
            Some(std::time::Duration::from_secs(18000))
        );
    }

    #[test]
    fn retry_after_http_date() {
        let h = hm(&[("retry-after", &in_minutes(300))]); // +5h
        let d = parse_reset_deadline(&h).expect("parsed");
        let secs = d.as_secs() as i64;
        assert!((299 * 60..=300 * 60).contains(&secs), "secs: {secs}");
    }

    #[test]
    fn anthropic_reset_iso_header() {
        let iso = (crate::utils::tz::now_utc() + chrono::Duration::hours(5)).to_rfc3339();
        let h = hm(&[("anthropic-ratelimit-tokens-reset", &iso)]);
        let d = parse_reset_deadline(&h).expect("parsed");
        let secs = d.as_secs() as i64;
        assert!((4 * 3600..=5 * 3600).contains(&secs), "secs: {secs}");
    }

    #[test]
    fn x_ratelimit_epoch_vs_relative() {
        // 大数值 = unix epoch（GitHub 风格）；小数值 = 相对秒数。
        let epoch = crate::utils::tz::now_utc().timestamp() + 7200;
        let h = hm(&[("x-ratelimit-reset", &epoch.to_string())]);
        let d = parse_reset_deadline(&h).expect("epoch parsed");
        assert!((7100..=7200).contains(&(d.as_secs() as i64)));

        let h2 = hm(&[("x-ratelimit-reset-requests", "45")]);
        assert_eq!(
            parse_reset_deadline(&h2),
            Some(std::time::Duration::from_secs(45))
        );
    }

    #[test]
    fn garbage_and_past_deadlines_yield_none() {
        assert_eq!(parse_reset_deadline(&hm(&[("retry-after", "soon")])), None);
        assert_eq!(parse_reset_deadline(&hm(&[])), None);
        // 过期 deadline 不是窗口信号。
        assert_eq!(parse_reset_deadline(&hm(&[("retry-after", "0")])), None);
        assert_eq!(
            parse_reset_deadline(&hm(&[("retry-after", &in_minutes(-10))])),
            None
        );
        // 非数值/日期的 ratelimit 头忽略。
        assert_eq!(
            parse_reset_deadline(&hm(&[("x-ratelimit-reset", "unlimited")])),
            None
        );
    }
}
