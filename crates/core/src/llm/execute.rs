//! Internal-consumer execution kernel (design §10.1): selection → slot →
//! guarded provider → closure → failure classification → retry — consumers
//! carry zero retry code.
//!
//! - `SideEffectGuard` decides the retry boundary in the kernel: it proxies
//!   `chat_stream` and flags the first `on_event` (callback-style streaming
//!   fires the first side effect with the first token). Retrying after that
//!   would duplicate partial output, so the kernel only retries while the
//!   flag is clear.
//! - `ExecError` splits closure failures: `Upstream` (classified by status +
//!   side-effect flag) vs `Local` (never retried). Defined in core so
//!   `ProviderError` stays untouched (engine crate 不动).

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use raisfast_agent::provider::ModelProvider;
use raisfast_agent::{ChatRequest, ChatResponse, ProviderError, StreamEvent};

use crate::errors::app_error::{AppError, AppResult};
use crate::llm::models::log::{LogSource, NewLog};
use crate::llm::service::{
    DEFAULT_RETRY_TIMES, INTERNAL_TIER, LlmRouter, ResolveCtx, ResolvedEndpoint, UpstreamFailure,
};
use crate::types::snowflake_id::SnowflakeId;

/// Closure error dichotomy (design §10.1).
pub enum ExecError {
    /// Upstream failure — the kernel classifies (status ∧ side-effect flag).
    Upstream(ProviderError),
    /// Closure's own failure (DB write etc.) — never retried.
    Local(AppError),
}

impl From<ProviderError> for ExecError {
    fn from(e: ProviderError) -> Self {
        ExecError::Upstream(e)
    }
}

impl From<AppError> for ExecError {
    fn from(e: AppError) -> Self {
        ExecError::Local(e)
    }
}

fn provider_status(err: &ProviderError) -> Option<u16> {
    match err {
        ProviderError::Http { status, .. } => Some(*status),
        _ => None,
    }
}

/// `ModelProvider` wrapper marking the side-effect boundary: the first
/// streamed event flips the flag; the kernel refuses to retry afterwards.
/// Created per `execute` call, never cached (flag is per-request).
pub struct SideEffectGuard {
    inner: Arc<dyn ModelProvider>,
    effects: Arc<AtomicBool>,
}

impl SideEffectGuard {
    /// Wrap a bare provider.
    pub fn new(inner: Arc<dyn ModelProvider>) -> Self {
        Self {
            inner,
            effects: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Whether any side effect (first stream event) has been observed.
    pub fn side_effects(&self) -> bool {
        self.effects.load(Ordering::Acquire)
    }
}

#[async_trait::async_trait]
impl ModelProvider for SideEffectGuard {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn chat(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
    ) -> Result<ChatResponse, ProviderError> {
        self.inner.chat(request, model).await
    }

    async fn embed(&self, texts: &[&str], model: &str) -> Result<Vec<Vec<f32>>, ProviderError> {
        self.inner.embed(texts, model).await
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest<'_>,
        model: &str,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Result<ChatResponse, ProviderError> {
        let effects = self.effects.clone();
        self.inner
            .chat_stream(request, model, &mut move |ev: StreamEvent| {
                effects.store(true, Ordering::Release);
                on_event(ev);
            })
            .await
    }

    async fn rerank(
        &self,
        query: &str,
        documents: &[&str],
        model: &str,
    ) -> Result<Vec<raisfast_agent::provider::RerankResult>, ProviderError> {
        self.inner.rerank(query, documents, model).await
    }

    async fn generate_image(
        &self,
        request: &raisfast_agent::provider::ImageRequest,
        model: &str,
    ) -> Result<Vec<raisfast_agent::provider::GeneratedImage>, ProviderError> {
        self.inner.generate_image(request, model).await
    }

    async fn transcribe(
        &self,
        audio: &raisfast_agent::provider::AudioInput<'_>,
        model: &str,
    ) -> Result<raisfast_agent::provider::Transcription, ProviderError> {
        self.inner.transcribe(audio, model).await
    }

    async fn speech(&self, text: &str, voice: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        self.inner.speech(text, voice, model).await
    }

    async fn video_submit(
        &self,
        request: &raisfast_agent::provider::VideoRequest,
        model: &str,
    ) -> Result<raisfast_agent::provider::VideoTask, ProviderError> {
        self.inner.video_submit(request, model).await
    }

    async fn video_query(
        &self,
        task_id: &str,
        model: &str,
    ) -> Result<raisfast_agent::provider::VideoTask, ProviderError> {
        self.inner.video_query(task_id, model).await
    }

    async fn video_content(&self, task_id: &str, model: &str) -> Result<Vec<u8>, ProviderError> {
        self.inner.video_content(task_id, model).await
    }
}

/// Log entry passed to [`LlmRouter::log_call`] (keeps the arg list flat).
struct LogCall<'a> {
    tenant: &'a str,
    source: LogSource,
    channel_id: Option<SnowflakeId>,
    key_index: Option<usize>,
    model: &'a str,
    elapsed_ms: Option<i32>,
    error: Option<&'a AppError>,
}

impl LlmRouter {
    /// Resolve an endpoint to a cached bare provider (design §10.2) by the
    /// channel `provider` field (§8.2: the channel picks the protocol):
    /// openai-compatible family (openai/deepseek/moonshot/ollama/siliconflow/
    /// generic/**gemini** — its registry preset is the OpenAI-compat surface)
    /// → `OpenAiCompatProvider`; native anthropic → `AnthropicProvider`
    /// (§10.2 P4 item: chat unlocks internal consumption; its non-chat
    /// modalities report unsupported and the kernel skips such channels).
    /// The guard wraps per call; the bare provider is cached by
    /// `(channel_id, key_index)`.
    pub fn provider_for(&self, ep: &ResolvedEndpoint) -> AppResult<Arc<dyn ModelProvider>> {
        let entry = self
            .providers
            .entry((ep.channel_id, ep.key_index))
            .or_insert_with(|| match ep.provider.as_str() {
                "anthropic" => Arc::new(crate::llm::provider_anthropic::AnthropicProvider::new(
                    ep.base_url.clone(),
                    Some(ep.api_key.clone()),
                    ep.param_override.clone(),
                    ep.header_override.clone(),
                )) as Arc<dyn ModelProvider>,
                _ => Arc::new(raisfast_agent::provider::openai::OpenAiCompatProvider::new(
                    ep.base_url.clone(),
                    Some(ep.api_key.clone()),
                )) as Arc<dyn ModelProvider>,
            })
            .clone();
        Ok(entry)
    }

    /// Unified execution kernel (design §10.1). The closure receives the
    /// guarded provider plus the resolved endpoint (model metadata rides
    /// along); everything else — slot, queue, retry, failover, logging — is
    /// kernel-internal.
    pub async fn execute<T, F, Fut>(
        self: &Arc<Self>,
        ctx: &ResolveCtx<'_>,
        model: &str,
        source: LogSource,
        run: F,
    ) -> AppResult<T>
    where
        F: FnMut(Arc<dyn ModelProvider>, ResolvedEndpoint) -> Fut,
        Fut: Future<Output = Result<T, ExecError>>,
    {
        let cache = self.cache.read().expect("llm cache lock").clone();
        let info = cache
            .model_info(ctx.tenant, model)
            .ok_or_else(|| AppError::BadRequest(format!("unknown model: {model}")))?;

        let mut retry = crate::llm::service::RetryState::default();
        let deadline = Instant::now() + std::time::Duration::from_secs(INTERNAL_TIER.max_wait_secs);
        let mut run = run;
        let mut last_err: Option<AppError> = None;

        for attempt in 0..=DEFAULT_RETRY_TIMES {
            // Fresh snapshot per attempt — an arrears-banned key must evict
            // its channel from selection mid-request (design §6.2).
            let cache = self.cache.read().expect("llm cache lock").clone();
            let (channel, key_index, _permit) = {
                match self
                    .acquire_slot(
                        &crate::llm::service::SlotRequest {
                            cache: &cache,
                            ctx,
                            model,
                            tier: INTERNAL_TIER,
                            caller: None,
                            body_bytes: 0,
                            deadline,
                        },
                        &mut retry,
                    )
                    .await
                {
                    Ok(triple) => triple,
                    Err(crate::llm::service::SlotError::NoRoute(err)) => return Err(err),
                    Err(crate::llm::service::SlotError::Rejected {
                        status,
                        message,
                        retry_after_secs,
                    }) => {
                        return Err(AppError::BadRequest(format!(
                            "llm busy (HTTP {status}{hint}): {message}",
                            hint = retry_after_secs
                                .map(|s| format!(", retry after {s}s"))
                                .unwrap_or_default()
                        )));
                    }
                }
            };

            let endpoint = ResolvedEndpoint {
                channel_id: channel.id,
                key_index,
                base_url: channel.base_url.clone(),
                api_key: channel.keys[key_index].plain.clone().unwrap_or_default(),
                provider: channel.provider.clone(),
                upstream_model: crate::llm::service::mapped_model(&channel, model),
                model: info.clone(),
                param_override: channel.param_override.clone(),
                header_override: channel.header_override.clone(),
            };
            let provider = match self.provider_for(&endpoint) {
                Ok(p) => p,
                Err(err) => {
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: None,
                        error: Some(&err),
                    })
                    .await;
                    return Err(err);
                }
            };
            let guard = Arc::new(SideEffectGuard::new(provider));
            let started = Instant::now();

            let outcome = run(guard.clone() as Arc<dyn ModelProvider>, endpoint).await;
            let elapsed = started.elapsed();
            let _ = attempt;

            match outcome {
                Ok(value) => {
                    self.report_success(channel.id, key_index);
                    self.record_latency(ctx.tenant, model, elapsed.as_secs_f64());
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: None,
                    })
                    .await;
                    return Ok(value);
                }
                Err(ExecError::Local(err)) => {
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&err),
                    })
                    .await;
                    return Err(err);
                }
                Err(ExecError::Upstream(ProviderError::Config(msg))) => {
                    // Endpoint cannot serve this request (modality unsupported
                    // on this provider / channel misconfig): deterministic for
                    // the endpoint, not an upstream health signal — mark the
                    // key tried and fail over WITHOUT a failure report (same
                    // semantics as the relay's anthropic guard, design §7.3).
                    retry.tried.insert((channel.id, key_index));
                    let err = AppError::BadRequest(msg.clone());
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&err),
                    })
                    .await;
                    last_err = Some(err);
                    continue;
                }
                Err(ExecError::Upstream(pe)) => {
                    let failure = UpstreamFailure {
                        status: provider_status(&pe),
                        message: pe.to_string(),
                        retry_after: None,
                    };
                    // §6.2 class 4): a plain 400 is the caller's fault — the
                    // key is innocent, no cooldown, no ban judgment. Gateway
                    // flakes disguised as 400s are upstream infra issues —
                    // they count toward failure judgment [照抄 claw-code
                    // `is_retryable_400`].
                    let flake_400 = matches!(&pe, ProviderError::Http { status: 400, .. })
                        && Self::upstream_retryable(&pe);
                    if failure.status != Some(400) || flake_400 {
                        self.report_failure(channel.id, key_index, &failure);
                    }
                    let retryable = Self::upstream_retryable(&pe) && !guard.side_effects();
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&AppError::Internal(anyhow::anyhow!(pe.to_string()))),
                    })
                    .await;
                    if retryable && attempt < DEFAULT_RETRY_TIMES {
                        last_err = Some(AppError::Internal(anyhow::anyhow!(pe.to_string())));
                        continue;
                    }
                    return match Self::upstream_to_app_error(&pe) {
                        Err(err) => Err(err),
                        Ok(()) => unreachable!("upstream_to_app_error always errors"),
                    };
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            AppError::Internal(anyhow::anyhow!("llm execute exhausted retries"))
        }))
    }

    fn upstream_retryable(pe: &ProviderError) -> bool {
        Self::upstream_retryable_impl(pe)
    }

    /// §7.3 retry conditions: channel-side errors (401-403 arrears, 404,
    /// 409+, 429, 5xx) and transport errors retry on another channel;
    /// 400 (deterministic client error) and the timeout class 408/504/524
    /// (upstream may already have processed and billed — retrying means
    /// double billing) never retry. Exception [照抄 claw-code
    /// `is_retryable_400`]: a 400 whose body carries gateway-flake markers
    /// ("no parseable body" / "connection reset" / "broken pipe" / "empty
    /// reply from server") is a transient network blip, retryable.
    /// (split out for unit testing)
    fn upstream_retryable_impl(pe: &ProviderError) -> bool {
        match pe {
            ProviderError::Http { status, body } => {
                if *status == 400 {
                    return Self::gateway_flake_body(body);
                }
                !matches!(status, 400 | 408 | 504 | 524)
            }
            ProviderError::Transport(_) => true,
            ProviderError::Parse(_) => false,
            ProviderError::Config(_) => false,
        }
    }

    fn gateway_flake_body(body: &str) -> bool {
        let lowered = body.to_ascii_lowercase();
        lowered.contains("no parseable body")
            || lowered.contains("connection reset")
            || lowered.contains("broken pipe")
            || lowered.contains("empty reply from server")
    }

    fn upstream_to_app_error(pe: &ProviderError) -> AppResult<()> {
        Err(match pe {
            ProviderError::Http { status, body } => {
                AppError::BadRequest(format!("upstream {status}: {body}"))
            }
            other => AppError::Internal(anyhow::anyhow!(other.to_string())),
        })
    }

    async fn log_call(&self, entry: LogCall<'_>) {
        let Some(pool) = self.pool.as_ref() else {
            return;
        };
        let log = NewLog {
            tenant_id: Some(entry.tenant.to_owned()),
            source: entry.source,
            channel_id: entry.channel_id,
            key_index: entry.key_index.map(|i| i as i32),
            model_name: entry.model.to_owned(),
            elapsed_ms: entry.elapsed_ms,
            error_message: entry.error.map(|e| e.to_string()),
            ..NewLog::default()
        };
        if let Err(err) = crate::llm::models::log::insert_log(pool, log).await {
            tracing::warn!(%err, "llm log insert failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::cache::ChannelCache;
    use crate::llm::models::channel::{
        LlmChannel, LlmChannelStatus, LlmKeyEntry, LlmKeyMode, LlmKeyStatus,
    };
    use crate::llm::service::ResolveCtx;
    use raisfast_agent::messages::{ChatMessage, ChatRole};

    /// Scripted provider: pops queued outcomes per call.
    struct MockProvider {
        script: std::sync::Mutex<Vec<MockStep>>,
    }

    enum MockStep {
        Ok,
        StreamThenBreak {
            events_before_break: usize,
        },
        Http(u16),
        Transport,
        /// Provider cannot serve this request (modality unsupported).
        Config,
    }

    impl Clone for MockStep {
        fn clone(&self) -> Self {
            match self {
                MockStep::Ok => MockStep::Ok,
                MockStep::StreamThenBreak {
                    events_before_break,
                } => MockStep::StreamThenBreak {
                    events_before_break: *events_before_break,
                },
                MockStep::Http(s) => MockStep::Http(*s),
                MockStep::Transport => MockStep::Transport,
                MockStep::Config => MockStep::Config,
            }
        }
    }

    #[async_trait::async_trait]
    impl ModelProvider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }

        async fn chat(
            &self,
            _request: &ChatRequest<'_>,
            _model: &str,
        ) -> Result<ChatResponse, ProviderError> {
            let mut script = self.script.lock().expect("script");
            match script.remove(0) {
                MockStep::Ok => Ok(ChatResponse::text_only("done")),
                MockStep::StreamThenBreak { .. } => Ok(ChatResponse::text_only("done")),
                MockStep::Http(status) => Err(ProviderError::Http {
                    status,
                    body: "boom".to_owned(),
                }),
                MockStep::Transport => Err(ProviderError::Transport("net down".to_owned())),
                MockStep::Config => Err(ProviderError::Config(
                    "provider mock does not support chat".to_owned(),
                )),
            }
        }

        async fn chat_stream(
            &self,
            _request: &ChatRequest<'_>,
            _model: &str,
            on_event: &mut (dyn FnMut(StreamEvent) + Send),
        ) -> Result<ChatResponse, ProviderError> {
            let step = {
                let mut script = self.script.lock().expect("script");
                script.remove(0)
            };
            match step {
                MockStep::StreamThenBreak {
                    events_before_break,
                } => {
                    for _ in 0..events_before_break {
                        on_event(StreamEvent::TextDelta {
                            delta: "x".to_owned(),
                        });
                    }
                    Err(ProviderError::Transport("stream broke mid-way".to_owned()))
                }
                MockStep::Ok => {
                    on_event(StreamEvent::TextDelta {
                        delta: "full".to_owned(),
                    });
                    on_event(StreamEvent::Final);
                    Ok(ChatResponse::text_only("full"))
                }
                other => {
                    // Non-stream steps reuse chat semantics.
                    let _ = on_event;
                    match other {
                        MockStep::Http(status) => Err(ProviderError::Http {
                            status,
                            body: "boom".to_owned(),
                        }),
                        MockStep::Transport => Err(ProviderError::Transport("net down".to_owned())),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    fn channel_row(id: i64) -> LlmChannel {
        LlmChannel {
            id: SnowflakeId(id),
            tenant_id: Some("default".to_owned()),
            name: format!("ch{id}"),
            provider: "openai".to_owned(),
            base_url: "https://mock.test/v1".to_owned(),
            api_keys: serde_json::to_value(vec![LlmKeyEntry {
                key: "plain".to_owned(),
                status: LlmKeyStatus::Active,
                disabled_reason: None,
                disabled_at: None,
                max_concurrency: None,
            }])
            .unwrap(),
            key_mode: LlmKeyMode::Polling,
            status: LlmChannelStatus::Enabled,
            models: "mock-model".to_owned(),
            model_mapping: None,
            priority: 0,
            weight: 0,
            channel_groups: "default".to_owned(),
            auto_ban: true,
            param_override: None,
            header_override: None,
            config: None,
            used_quota: 0,
            cost_mode: crate::llm::models::channel::LlmCostMode::Usage,
            cost_discount: 1.0,
            monthly_cost: None,
            test_model: None,
            test_time: None,
            response_time: None,
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        }
    }

    fn router_with_mock(steps: Vec<MockStep>) -> std::sync::Arc<crate::llm::service::LlmRouter> {
        router_with_mocks(vec![(1, 1, steps)])
    }

    /// Multi-channel harness: `(id, priority, script)` per channel — channel
    /// 1 defaults to a higher priority so it is attempted first.
    fn router_with_mocks(
        channels: Vec<(i64, i64, Vec<MockStep>)>,
    ) -> std::sync::Arc<crate::llm::service::LlmRouter> {
        let mut cache = ChannelCache::default();
        for (id, priority, _) in &channels {
            let mut row = channel_row(*id);
            row.priority = *priority;
            let cached = ChannelCache::from_row(&row);
            cache
                .channels
                .insert(cached.id, std::sync::Arc::new(cached));
        }
        cache.models.insert(
            ("default".to_owned(), "mock-model".to_owned()),
            std::sync::Arc::new(crate::llm::cache::ModelInfo {
                name: "mock-model".to_owned(),
                model_type: crate::llm::models::model::LlmModelType::Chat,
                pricing: crate::llm::cache::Pricing {
                    price_mode: crate::llm::models::model::LlmPriceMode::Token,
                    input_price: 1.0,
                    output_price: 1.0,
                    cache_read_price: None,
                    cache_write_price: None,
                    call_price: None,
                },
                params: None,
            }),
        );
        cache.rebuild_routes();
        let router = crate::llm::service::LlmRouter::from_cache_for_test(cache);
        // Seed the provider cache with the mock (provider_for hits cache
        // first), so execute() never constructs OpenAiCompatProvider.
        for (id, _, steps) in channels {
            router.providers.insert(
                (SnowflakeId(id), 0),
                std::sync::Arc::new(MockProvider {
                    script: std::sync::Mutex::new(steps),
                }) as std::sync::Arc<dyn ModelProvider>,
            );
        }
        router
    }

    fn ctx() -> ResolveCtx<'static> {
        ResolveCtx {
            tenant: "default",
            group: None,
            pin_channel: None,
        }
    }

    fn leaked_messages() -> &'static [ChatMessage] {
        Box::leak(Box::new(vec![ChatMessage {
            role: ChatRole::User,
            content: Some("hi".to_owned()),
            tool_calls: None,
            tool_call_id: None,
        }]))
    }

    fn chat_req() -> ChatRequest<'static> {
        ChatRequest {
            messages: leaked_messages(),
            tools: None,
            temperature: None,
            max_tokens: None,
            stop: None,
        }
    }

    async fn run_chat(
        router: &std::sync::Arc<crate::llm::service::LlmRouter>,
    ) -> AppResult<ChatResponse> {
        let _req = chat_req();
        router
            .execute(
                &ctx(),
                "mock-model",
                LogSource::Agent,
                move |p, _ep| async move {
                    p.chat(&chat_req(), "mock-model")
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    #[tokio::test]
    async fn success_on_first_attempt() {
        let router = router_with_mock(vec![MockStep::Ok]);
        let out = run_chat(&router).await.expect("ok");
        assert_eq!(out.text.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn transient_500_retries_then_succeeds() {
        let router = router_with_mock(vec![MockStep::Http(500), MockStep::Ok]);
        let out = run_chat(&router).await.expect("recovered");
        assert_eq!(out.text.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn transport_error_retries() {
        let router = router_with_mock(vec![MockStep::Transport, MockStep::Ok]);
        let out = run_chat(&router).await.expect("recovered");
        assert_eq!(out.text.as_deref(), Some("done"));
    }

    #[tokio::test]
    async fn client_4xx_never_retries() {
        let router = router_with_mock(vec![MockStep::Http(400)]);
        let err = run_chat(&router).await.expect_err("4xx is fatal");
        // 4xx maps to BadRequest (§7.3).
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn exhaustion_after_retries_errors_out() {
        let router = router_with_mock(vec![
            MockStep::Http(500),
            MockStep::Http(500),
            MockStep::Http(500),
        ]);
        assert!(run_chat(&router).await.is_err());
    }

    #[tokio::test]
    async fn no_retry_after_first_stream_event() {
        // Stream emits one event then breaks: side effects started → the
        // kernel must NOT retry (duplicate output risk, design §10.1).
        let router = router_with_mock(vec![
            MockStep::StreamThenBreak {
                events_before_break: 1,
            },
            MockStep::Ok, // would succeed if (wrongly) retried
        ]);
        let _req = chat_req();
        let err = router
            .execute(
                &ctx(),
                "mock-model",
                LogSource::Agent,
                move |p, _ep| async move {
                    p.chat_stream(&chat_req(), "mock-model", &mut |_ev| {})
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
            .expect_err("must not retry after side effects");
        assert!(!matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn retry_allowed_when_stream_breaks_before_first_event() {
        let router = router_with_mock(vec![
            MockStep::StreamThenBreak {
                events_before_break: 0,
            },
            MockStep::Ok,
        ]);
        let _req = chat_req();
        let out = router
            .execute(
                &ctx(),
                "mock-model",
                LogSource::Agent,
                move |p, _ep| async move {
                    p.chat_stream(&chat_req(), "mock-model", &mut |_ev| {})
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
            .expect("pre-first-event failure is retryable");
        assert_eq!(out.text.as_deref(), Some("full"));
    }

    #[tokio::test]
    async fn local_error_never_retries() {
        let router = router_with_mock(vec![MockStep::Ok, MockStep::Ok]);
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        let err = router
            .execute(&ctx(), "mock-model", LogSource::Agent, move |_p, _ep| {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Err::<ChatResponse, _>(ExecError::Local(AppError::BadRequest(
                        "db write failed".to_owned(),
                    )))
                }
            })
            .await
            .expect_err("local errors surface immediately");
        assert!(matches!(err, AppError::BadRequest(_)));
        assert_eq!(
            attempts.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "Local must short-circuit all retries"
        );
    }

    #[tokio::test]
    async fn side_effect_guard_flag_semantics() {
        let guard = SideEffectGuard::new(std::sync::Arc::new(MockProvider {
            script: std::sync::Mutex::new(vec![MockStep::Ok]),
        }));
        assert!(!guard.side_effects());
        let _req = chat_req();
        let _ = guard.chat_stream(&chat_req(), "m", &mut |_| {}).await;
        assert!(
            guard.side_effects(),
            "any stream event flips the side-effect flag"
        );
    }

    #[test]
    fn upstream_retryable_classification() {
        let http = |s: u16| ProviderError::Http {
            status: s,
            body: String::new(),
        };
        // 渠道侧错误 → 换渠道重试（§7.3）。
        assert!(LlmRouter::upstream_retryable_impl(&http(429)));
        assert!(
            LlmRouter::upstream_retryable_impl(&http(401)),
            "arrears failover"
        );
        assert!(
            LlmRouter::upstream_retryable_impl(&http(402)),
            "arrears failover"
        );
        assert!(
            LlmRouter::upstream_retryable_impl(&http(403)),
            "arrears failover"
        );
        assert!(
            LlmRouter::upstream_retryable_impl(&http(404)),
            "channel coverage differs"
        );
        assert!(LlmRouter::upstream_retryable_impl(&http(500)));
        assert!(LlmRouter::upstream_retryable_impl(&http(503)));
        assert!(LlmRouter::upstream_retryable_impl(
            &ProviderError::Transport("t".to_owned())
        ));
        // 400 = 确定性失败（除非是网关抖动 [照抄 claw-code]）；408/504/524 =
        // 上游可能已计费的超时类。
        assert!(!LlmRouter::upstream_retryable_impl(&http(400)));
        assert!(!LlmRouter::upstream_retryable_impl(&ProviderError::Http {
            status: 400,
            body: "invalid model".to_owned(),
        }));
        // 网关抖动伪装的 400 是瞬时网络故障，可重试。
        for marker in [
            "no parseable body",
            "HTTP 400 from backend (connection reset)",
            "broken pipe while reading",
            "empty reply from server",
        ] {
            assert!(
                LlmRouter::upstream_retryable_impl(&ProviderError::Http {
                    status: 400,
                    body: marker.to_owned(),
                }),
                "flake marker should retry: {marker}"
            );
        }
        assert!(!LlmRouter::upstream_retryable_impl(&http(408)));
        assert!(!LlmRouter::upstream_retryable_impl(&http(504)));
        assert!(!LlmRouter::upstream_retryable_impl(&http(524)));
        assert!(!LlmRouter::upstream_retryable_impl(&ProviderError::Parse(
            "p".to_owned()
        )));
        assert!(!LlmRouter::upstream_retryable_impl(&ProviderError::Config(
            "c".to_owned()
        )));
    }

    // ── Config → Skip 换渠道（§10.2 六模态解锁的内核语义）─────────

    #[tokio::test]
    async fn config_error_skips_channel_without_failure_report() {
        // 高优先级渠道不支持该模态（如 anthropic 渠道被请求 embed）→
        // 跳过且不计失败，低优先级渠道兜底成功。
        let router = router_with_mocks(vec![
            (1, 1, vec![MockStep::Config]),
            (2, 0, vec![MockStep::Ok]),
        ]);
        let out = run_chat(&router).await.expect("fails over to channel 2");
        assert_eq!(out.text.as_deref(), Some("done"));
        assert!(
            router.cooldown_snapshot().is_empty(),
            "skip must not report failure: {:?}",
            router.cooldown_snapshot()
        );
    }

    #[tokio::test]
    async fn all_config_errors_surface_bad_request() {
        let router = router_with_mocks(vec![
            (1, 1, vec![MockStep::Config]),
            (2, 0, vec![MockStep::Config]),
        ]);
        let err = run_chat(&router).await.expect_err("all channels skip");
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }
}
