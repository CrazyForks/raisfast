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
use crate::llm::models::model::LlmModelType;
use crate::llm::relay::adaptor::RelayUsage;
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

/// Triggering user for log attribution (None = system-triggered).
fn caller_user_of(ctx: &crate::llm::service::ResolveCtx<'_>) -> Option<SnowflakeId> {
    ctx.caller.and_then(|c| c.user)
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
    caller_user: Option<SnowflakeId>,
    channel_id: Option<SnowflakeId>,
    key_index: Option<usize>,
    model: &'a str,
    elapsed_ms: Option<i32>,
    error: Option<&'a AppError>,
    /// Settled quota (internal billing §10.3; 0 when unbilled/failed).
    quota: crate::types::quota::Quota,
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
    #[allow(clippy::too_many_arguments)]
    pub async fn execute<T, F, Fut>(
        self: &Arc<Self>,
        ctx: &ResolveCtx<'_>,
        model: &str,
        source: LogSource,
        mut billing: Option<crate::llm::billing::InternalBilling>,
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
                            // 内部调用的 per-user 并发上限（§7.6）：身份由
                            // 消费方通过 ResolveCtx.caller 携带，系统触发
                            // （cron 等）为 None 不计数。
                            caller: ctx.caller,
                            body_bytes: 0,
                            deadline,
                        },
                        &mut retry,
                    )
                    .await
                {
                    Ok(triple) => triple,
                    Err(crate::llm::service::SlotError::NoRoute(err)) => {
                        self.refund_billing(ctx, billing.take()).await;
                        return Err(err);
                    }
                    Err(crate::llm::service::SlotError::Rejected {
                        status,
                        message,
                        retry_after_secs,
                    }) => {
                        self.refund_billing(ctx, billing.take()).await;
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
                        caller_user: caller_user_of(ctx),
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: None,
                        error: Some(&err),
                        quota: crate::types::quota::Quota(0),
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
                    let quota = self.settle_billing(ctx, billing.take()).await;
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        caller_user: caller_user_of(ctx),
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: None,
                        quota,
                    })
                    .await;
                    return Ok(value);
                }
                Err(ExecError::Local(err)) => {
                    // 上游已应答、闭包自身失败：用量已产生，照常结算（§9.3）。
                    let quota = self.settle_billing(ctx, billing.take()).await;
                    self.log_call(LogCall {
                        tenant: ctx.tenant,
                        source,
                        caller_user: caller_user_of(ctx),
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&err),
                        quota,
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
                        caller_user: caller_user_of(ctx),
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&err),
                        quota: crate::types::quota::Quota(0),
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
                        caller_user: caller_user_of(ctx),
                        channel_id: Some(channel.id),
                        key_index: Some(key_index),
                        model,
                        elapsed_ms: Some(elapsed.as_millis() as i32),
                        error: Some(&AppError::Internal(anyhow::anyhow!(pe.to_string()))),
                        quota: crate::types::quota::Quota(0),
                    })
                    .await;
                    if retryable && attempt < DEFAULT_RETRY_TIMES {
                        last_err = Some(AppError::Internal(anyhow::anyhow!(pe.to_string())));
                        continue;
                    }
                    // 终止：本次调用未产生用量 → 全额退款。
                    self.refund_billing(ctx, billing.take()).await;
                    return match Self::upstream_to_app_error(&pe) {
                        Err(err) => Err(err),
                        Ok(()) => unreachable!("upstream_to_app_error always errors"),
                    };
                }
            }
        }
        self.refund_billing(ctx, billing.take()).await;
        Err(last_err.unwrap_or_else(|| {
            AppError::Internal(anyhow::anyhow!("llm execute exhausted retries"))
        }))
    }

    /// 结算（成功/Local 臂）：钱包差额处理 + 返回应写日志的 quota。
    async fn settle_billing(
        &self,
        ctx: &ResolveCtx<'_>,
        billing: Option<crate::llm::billing::InternalBilling>,
    ) -> crate::types::quota::Quota {
        match billing {
            Some(b) => {
                let quota = b.quota_of_preview();
                if let Some(pool) = self.pool.as_ref() {
                    b.settle(pool, Some(ctx.tenant)).await;
                }
                quota
            }
            None => crate::types::quota::Quota(0),
        }
    }

    /// 全部失败：退还预扣（metered），free 无操作。
    async fn refund_billing(
        &self,
        ctx: &ResolveCtx<'_>,
        billing: Option<crate::llm::billing::InternalBilling>,
    ) {
        if let (Some(b), Some(pool)) = (billing, self.pool.as_ref()) {
            b.refund(pool, Some(ctx.tenant)).await;
        }
    }

    // ── modality facade (§10.1 consumer surface) ─────────────────────
    //
    // 目标形态：内部调用只指定"模态 + (可选)模型"，身份用 .as_user() 链上。
    // 默认模型（租户 options）、类型守门、渠道路由、failover、协议转换、
    // 预扣/结算、日志归因全部在内核侧完成。

    /// Facade 入口：`router.call(tenant, LogSource::Flow).as_user(uid).chat(..)`.
    pub fn call<'a>(self: &'a Arc<Self>, tenant: &'a str, source: LogSource) -> LlmCall<'a> {
        LlmCall {
            router: self,
            tenant,
            source,
            caller: None,
            pin_channel: None,
        }
    }

    /// 模型名解析链（§10.2）：显式指定 → 租户 options → 全局 options → 400。
    pub(crate) async fn resolve_default(
        &self,
        tenant: &str,
        explicit: Option<&str>,
        option_key: &str,
        modality: &str,
    ) -> AppResult<String> {
        if let Some(m) = explicit {
            return Ok(m.to_owned());
        }
        let Some(pool) = self.pool.as_ref() else {
            return Err(AppError::BadRequest(format!(
                "no {modality} model specified and no default configured"
            )));
        };
        for scope in [Some(tenant), None] {
            if let Some(row) = crate::models::options::find_by_key(pool, option_key, scope).await?
                && let Some(v) = row.value.as_str()
                && !v.trim().is_empty()
            {
                return Ok(v.trim().to_owned());
            }
        }
        Err(AppError::BadRequest(format!(
            "no {modality} model configured (set option {option_key})"
        )))
    }

    /// 目录类型守门（§5.4）：拼错/类型不符当场 400，不打上游。
    fn guard_model_type(
        &self,
        tenant: &str,
        model: &str,
        expected: &[LlmModelType],
        modality: &str,
    ) -> AppResult<()> {
        let Some(info) = self.model_info(tenant, model) else {
            return Err(AppError::BadRequest(format!("unknown model: {model}")));
        };
        if !expected.contains(&info.model_type) {
            return Err(AppError::BadRequest(format!(
                "model {model} is not a {modality} model: {}",
                info.model_type.as_str()
            )));
        }
        Ok(())
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
            user_id: entry.caller_user,
            channel_id: entry.channel_id,
            key_index: entry.key_index.map(|i| i as i32),
            model_name: entry.model.to_owned(),
            elapsed_ms: entry.elapsed_ms,
            error_message: entry.error.map(|e| e.to_string()),
            quota: entry.quota,
            ..NewLog::default()
        };
        if let Err(err) = crate::llm::models::log::insert_log(pool, log).await {
            tracing::warn!(%err, "llm log insert failed");
        }
    }
}

/// One consumer call under construction (see [`LlmRouter::call`]).
#[derive(Clone)]
pub struct LlmCall<'a> {
    router: &'a Arc<LlmRouter>,
    tenant: &'a str,
    source: LogSource,
    caller: Option<crate::llm::service::Caller>,
    pin_channel: Option<SnowflakeId>,
}

impl LlmCall<'_> {
    /// 归因到用户：per-user 并发上限（§7.6）+ 计费（§10.3）+ 日志归因。
    /// 省略 = 系统触发（cron/webhook），跳过计费与限额。
    pub fn as_user(mut self, user_id: SnowflakeId) -> Self {
        self.caller = Some(crate::llm::service::Caller::user(user_id));
        self
    }

    /// 钉死到指定渠道（design §10.1）：跳过 priority/加权路由，只在该渠道的
    /// key 池内轮转。渠道必须属于同租户、服务该模型、位于请求分组内，否则
    /// 直接 400（不做跨渠道 failover）。
    pub fn pin_channel(mut self, channel_id: SnowflakeId) -> Self {
        self.pin_channel = Some(channel_id);
        self
    }

    fn ctx(&self) -> ResolveCtx<'_> {
        ResolveCtx {
            tenant: self.tenant,
            group: None,
            pin_channel: self.pin_channel,
            caller: self.caller,
        }
    }

    /// 模型解析 + 类型守门 + 目录元数据。
    async fn prepare(
        &self,
        model: Option<&str>,
        default_key: &str,
        modality: &str,
        expected: &[LlmModelType],
    ) -> AppResult<(String, crate::llm::cache::ModelInfo)> {
        let model = self
            .router
            .resolve_default(self.tenant, model, default_key, modality)
            .await?;
        self.router
            .guard_model_type(self.tenant, &model, expected, modality)?;
        let info = self
            .router
            .model_info(self.tenant, &model)
            .ok_or_else(|| AppError::BadRequest(format!("unknown model: {model}")))?;
        Ok((model, (*info).clone()))
    }

    /// 租户策略 + 预扣（metered）/日限额检查（free）。无身份/无池 → 不计费。
    async fn billing(
        &self,
        info: &crate::llm::cache::ModelInfo,
        estimate: RelayUsage,
    ) -> AppResult<Option<crate::llm::billing::InternalBilling>> {
        let Some(user) = self.caller.and_then(|c| c.user) else {
            return Ok(None);
        };
        let Some(pool) = self.router.pool.as_ref() else {
            return Ok(None);
        };
        let mode = crate::llm::billing::resolve_policy(pool, self.tenant).await?;
        let currency = match &mode {
            crate::llm::billing::BillingMode::Metered { currency } => currency.clone(),
            crate::llm::billing::BillingMode::Free { .. } => String::new(),
        };
        let b = crate::llm::billing::InternalBilling::new(
            user,
            currency,
            info.pricing.clone(),
            mode,
            estimate,
        );
        let day = crate::utils::tz::now_utc().format("%Y-%m-%d").to_string();
        crate::llm::billing::preflight(pool, self.tenant, &day, &b).await?;
        Ok(Some(b))
    }

    /// chat 补全（非流式）。模型 None → 租户默认 chat 模型。
    pub async fn chat(
        self,
        model: Option<&str>,
        request: &ChatRequest<'_>,
    ) -> AppResult<ChatResponse> {
        let (model, info) = self
            .prepare(
                model,
                "llm.default_chat_model",
                "chat",
                &[LlmModelType::Chat, LlmModelType::Vlm],
            )
            .await?;
        let billing = self.billing(&info, chat_estimate(request, &info)).await?;
        let cell = billing.as_ref().map(|b| b.usage.clone());
        self.router
            .execute(&self.ctx(), &model, self.source, billing, |p, ep| {
                let cell = cell.clone();
                async move {
                    let resp = p
                        .chat(request, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)?;
                    if let (Some(cell), Some(u)) = (cell.as_ref(), resp.usage) {
                        store_usage(cell, u);
                    }
                    Ok(resp)
                }
            })
            .await
    }

    /// 流式 chat：增量事件透传给 `on_event`，返回完整聚合响应。
    pub async fn chat_stream(
        self,
        model: Option<&str>,
        request: &ChatRequest<'_>,
        on_event: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> AppResult<ChatResponse> {
        let (model, info) = self
            .prepare(
                model,
                "llm.default_chat_model",
                "chat",
                &[LlmModelType::Chat, LlmModelType::Vlm],
            )
            .await?;
        let billing = self.billing(&info, chat_estimate(request, &info)).await?;
        let cell = billing.as_ref().map(|b| b.usage.clone());
        // 回调装进 owned Mutex：&Mutex 是 Copy，可克隆进每次调用的 Future
        // 而不借用闭包环境（保持 FnMut，内核可在首事件前重试）。
        let on_event = std::sync::Mutex::new(on_event);
        self.router
            .execute(&self.ctx(), &model, self.source, billing, |p, ep| {
                let sink = cell.clone();
                let on_event = &on_event;
                async move {
                    let resp = p
                        .chat_stream(request, &ep.upstream_model, &mut |ev| {
                            if let (StreamEvent::Usage(u), Some(cell)) = (&ev, sink.as_ref()) {
                                store_usage(cell, *u);
                            }
                            if let Ok(mut f) = on_event.lock() {
                                f(ev);
                            }
                        })
                        .await
                        .map_err(ExecError::Upstream)?;
                    Ok(resp)
                }
            })
            .await
    }

    /// 文本向量（模型 None → 租户默认 embedding 模型）。
    pub async fn embed(self, model: Option<&str>, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
        let (model, info) = self
            .prepare(
                model,
                "llm.default_embedding_model",
                "embedding",
                &[LlmModelType::Embedding],
            )
            .await?;
        let chars: usize = texts.iter().map(|t| t.chars().count()).sum();
        let estimate = RelayUsage {
            prompt_tokens: (chars / 4).max(1) as i64,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.embed(texts, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 相关性重排（best-first）。
    pub async fn rerank(
        self,
        model: &str,
        query: &str,
        documents: &[&str],
    ) -> AppResult<Vec<raisfast_agent::provider::RerankResult>> {
        let (model, info) = self
            .prepare(Some(model), "", "rerank", &[LlmModelType::Rerank])
            .await?;
        let mut chars = query.chars().count();
        for d in documents {
            chars += d.chars().count();
        }
        let estimate = RelayUsage {
            prompt_tokens: (chars / 4).max(1) as i64,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.rerank(query, documents, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 文生图。
    pub async fn image(
        self,
        model: &str,
        request: &raisfast_agent::provider::ImageRequest,
    ) -> AppResult<Vec<raisfast_agent::provider::GeneratedImage>> {
        let (model, info) = self
            .prepare(Some(model), "", "image", &[LlmModelType::Image])
            .await?;
        let n = request.n.max(1) as i64;
        let estimate = RelayUsage {
            prompt_tokens: 1,
            completion_tokens: n,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.generate_image(request, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 语音转文字（按音频时长计费：文件大小估算秒数）。
    pub async fn transcribe(
        self,
        model: &str,
        audio: &raisfast_agent::provider::AudioInput<'_>,
    ) -> AppResult<raisfast_agent::provider::Transcription> {
        let (model, info) = self
            .prepare(Some(model), "", "transcription", &[LlmModelType::Asr])
            .await?;
        let secs = crate::llm::relay::billing::estimate_audio_secs(audio.data.len());
        let estimate = RelayUsage {
            prompt_tokens: secs,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.transcribe(audio, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 文生语音（按输入字符精确计费，input-side 结算语义 §9.3）。
    pub async fn speech(self, model: &str, text: &str, voice: &str) -> AppResult<Vec<u8>> {
        let (model, info) = self
            .prepare(Some(model), "", "speech", &[LlmModelType::Tts])
            .await?;
        let estimate = RelayUsage {
            prompt_tokens: text.chars().count().max(1) as i64,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.speech(text, voice, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 提交异步视频任务（按秒数估算预扣，提交即结算）。
    pub async fn video_submit(
        self,
        model: &str,
        request: &raisfast_agent::provider::VideoRequest,
    ) -> AppResult<raisfast_agent::provider::VideoTask> {
        let (model, info) = self
            .prepare(Some(model), "", "video", &[LlmModelType::Video])
            .await?;
        let seconds: i64 = request
            .seconds
            .as_deref()
            .and_then(|s| s.parse().ok())
            .filter(|v| *v > 0)
            .unwrap_or(12);
        let estimate = RelayUsage {
            prompt_tokens: 1,
            completion_tokens: seconds,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        };
        let billing = self.billing(&info, estimate).await?;
        self.router
            .execute(
                &self.ctx(),
                &model,
                self.source,
                billing,
                |p, ep| async move {
                    p.video_submit(request, &ep.upstream_model)
                        .await
                        .map_err(ExecError::Upstream)
                },
            )
            .await
    }

    /// 轮询视频任务（提交时已计费，不再扣）。
    pub async fn video_query(
        self,
        model: &str,
        task_id: &str,
    ) -> AppResult<raisfast_agent::provider::VideoTask> {
        let (model, _) = self
            .prepare(Some(model), "", "video", &[LlmModelType::Video])
            .await?;
        self.router
            .execute(&self.ctx(), &model, self.source, None, |p, ep| async move {
                p.video_query(task_id, &ep.upstream_model)
                    .await
                    .map_err(ExecError::Upstream)
            })
            .await
    }

    /// 拉取已完成视频内容。
    pub async fn video_content(self, model: &str, task_id: &str) -> AppResult<Vec<u8>> {
        let (model, _) = self
            .prepare(Some(model), "", "video", &[LlmModelType::Video])
            .await?;
        self.router
            .execute(&self.ctx(), &model, self.source, None, |p, ep| async move {
                p.video_content(task_id, &ep.upstream_model)
                    .await
                    .map_err(ExecError::Upstream)
            })
            .await
    }
}

/// chat 预扣估算：prompt = 消息字符 /4（下限 1），output = max_tokens →
/// params.max_output_tokens → 4096（与 §9.3 预扣链一致）。
fn chat_estimate(request: &ChatRequest<'_>, info: &crate::llm::cache::ModelInfo) -> RelayUsage {
    let prompt_chars: usize = request
        .messages
        .iter()
        .filter_map(|m| m.content.as_deref())
        .map(str::chars)
        .map(|c| c.count())
        .sum();
    let max_output = request
        .max_tokens
        .filter(|v| *v > 0)
        .or_else(|| {
            info.params
                .as_ref()
                .and_then(|p| p.get("max_output_tokens"))
                .and_then(serde_json::Value::as_i64)
                .filter(|v| *v > 0)
        })
        .unwrap_or(4096);
    RelayUsage {
        prompt_tokens: (prompt_chars / 4).max(1) as i64,
        completion_tokens: max_output,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }
}

/// 把 provider 返回的 TokenUsage 累进计费 cell（prompt 含 cache，§9.3）。
fn store_usage(
    cell: &std::sync::Mutex<Option<RelayUsage>>,
    u: raisfast_agent::messages::TokenUsage,
) {
    if let Ok(mut c) = cell.lock() {
        let usage = c.get_or_insert_with(RelayUsage::default);
        usage.prompt_tokens += u.input_tokens.unwrap_or(0) as i64;
        usage.completion_tokens += u.output_tokens.unwrap_or(0) as i64;
        usage.cache_read_tokens += u.cache_read.unwrap_or(0) as i64;
        usage.cache_write_tokens += u.cache_write.unwrap_or(0) as i64;
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
        /// Success with a big usage payload (billing math assertions).
        OkUsage,
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
                MockStep::OkUsage => MockStep::OkUsage,
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
                MockStep::OkUsage => Ok(ChatResponse {
                    text: Some("done".to_owned()),
                    tool_calls: vec![],
                    usage: Some(raisfast_agent::messages::TokenUsage {
                        input_tokens: Some(1_000_000),
                        output_tokens: Some(1_000_000),
                        cache_read: None,
                        cache_write: None,
                    }),
                }),
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
            caller: None,
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
                None,
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
                None,
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
                None,
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
            .execute(
                &ctx(),
                "mock-model",
                LogSource::Agent,
                None,
                move |_p, _ep| {
                    let counter = counter.clone();
                    async move {
                        counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Err::<ChatResponse, _>(ExecError::Local(AppError::BadRequest(
                            "db write failed".to_owned(),
                        )))
                    }
                },
            )
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

    // ── facade + 内部计费（§10.3，DB 路径）────────────────────────

    use crate::types::price::Price;

    async fn db_pool() -> crate::db::Pool {
        crate::test_pool!()
    }

    fn billed_cache() -> ChannelCache {
        let row = channel_row(1);
        let mut cache = ChannelCache::default();
        let cached = ChannelCache::from_row(&row);
        cache.channels.insert(cached.id, Arc::new(cached));
        cache.models.insert(
            ("default".to_owned(), "mock-model".to_owned()),
            Arc::new(crate::llm::cache::ModelInfo {
                name: "mock-model".to_owned(),
                model_type: LlmModelType::Chat,
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
        cache
    }

    fn seeded_router(pool: crate::db::Pool) -> Arc<crate::llm::service::LlmRouter> {
        let router =
            crate::llm::service::LlmRouter::from_cache_for_test_with_pool(billed_cache(), pool);
        router.providers.insert(
            (SnowflakeId(1), 0),
            Arc::new(MockProvider {
                script: std::sync::Mutex::new(vec![MockStep::OkUsage]),
            }) as Arc<dyn ModelProvider>,
        );
        router
    }

    async fn make_user(pool: &crate::db::Pool) -> SnowflakeId {
        let username = format!("facade-{}", crate::utils::id::new_id());
        let cmd = crate::commands::CreateUserCmd::new(
            username,
            crate::models::user::RegisteredVia::Email,
        );
        crate::models::user::create(pool, &cmd, None)
            .await
            .expect("user")
            .id
    }

    fn today() -> String {
        crate::utils::tz::now_utc().format("%Y-%m-%d").to_string()
    }

    async fn seed_option(pool: &crate::db::Pool, key: &str, value: serde_json::Value) {
        use crate::db::Driver;
        use crate::db::driver::DbDriver;
        let ph = |i: usize| Driver::ph(i);
        let sql = format!(
            "INSERT INTO options (id, option_key, value, type, group_name, label, autoload, sort_order, updated_at) \
             VALUES ({}, {}, {}, 'text', 'llm', 'llm', 1, 0, {})",
            ph(1),
            ph(2),
            ph(3),
            ph(4)
        );
        sqlx::query(crate::db::safe_sql(&sql))
            .bind(crate::utils::id::new_id())
            .bind(key)
            .bind(value.to_string())
            .bind(crate::utils::tz::now_utc())
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn facade_metered_holds_and_settles_wallet() -> AppResult<()> {
        let pool = db_pool().await;
        let router = seeded_router(pool.clone());
        let user = make_user(&pool).await;
        seed_option(&pool, "llm.billing.mode", serde_json::json!("metered")).await;
        seed_option(&pool, "llm.billing.currency", serde_json::json!("CNY")).await;

        // 钱包充值 10_000¢
        let w = crate::models::wallet::find_or_create(&pool, user, "CNY")
            .await
            .unwrap();
        crate::in_transaction!(pool, tx, {
            crate::models::wallet::apply_wallet_delta(
                &mut tx,
                w.id,
                w.version,
                1_000_000,
                w.balance.0,
            )
            .await
        })?;

        // usage = 1M in + 1M out ×$1/$1 → quota 2_000_000 → 200¢
        let resp = router
            .call("default", LogSource::Agent)
            .as_user(user)
            .chat(Some("mock-model"), &chat_req())
            .await
            .expect("metered call");
        assert_eq!(resp.text.as_deref(), Some("done"));

        let w2 = crate::models::wallet::find_by_user_and_currency(&pool, user, "CNY")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(w2.balance, Price(999_800), "10_000¢ - 200¢");

        let q: i64 = sqlx::query_scalar("SELECT quota FROM llm_logs ORDER BY id DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(q, 2_000_000, "真实结算值写入 llm_logs（计量与扣费分离）");
        Ok(())
    }

    #[tokio::test]
    async fn facade_free_daily_limit_blocks() {
        let pool = db_pool().await;
        let router = seeded_router(pool.clone());
        let user = make_user(&pool).await;
        seed_option(
            &pool,
            "llm.billing.free_daily_user_quota",
            serde_json::json!(100),
        )
        .await;

        // 当日已用 100 → 新调用（预估 ≥1）超限
        crate::llm::models::log::insert_log(
            &pool,
            NewLog {
                tenant_id: Some("default".to_owned()),
                user_id: Some(user),
                source: LogSource::Flow,
                model_name: "mock-model".to_owned(),
                quota: crate::types::quota::Quota(100),
                day: Some(today()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let err = router
            .call("default", LogSource::Flow)
            .as_user(user)
            .chat(Some("mock-model"), &chat_req())
            .await
            .expect_err("daily limit must block");
        assert!(matches!(err, AppError::TooManyRequests(_)), "{err}");
    }

    #[tokio::test]
    async fn facade_system_caller_skips_billing() {
        let pool = db_pool().await;
        let router = seeded_router(pool.clone());
        let user = make_user(&pool).await;
        seed_option(&pool, "llm.billing.mode", serde_json::json!("metered")).await;

        // 无 as_user → 系统触发：钱包不会被创建/扣减
        let resp = router
            .call("default", LogSource::Agent)
            .chat(Some("mock-model"), &chat_req())
            .await
            .expect("system call skips billing");
        assert_eq!(resp.text.as_deref(), Some("done"));
        let wallets = crate::models::wallet::find_by_user(&pool, user)
            .await
            .unwrap();
        assert!(wallets.is_empty(), "no wallet touched: {wallets:?}");
    }

    #[tokio::test]
    async fn facade_type_guard_and_missing_default() {
        // 无池路由：显式模型可用，None 默认 → 400
        let router = router_with_mock(vec![MockStep::Ok]);
        let resp = router
            .call("default", LogSource::Agent)
            .chat(Some("mock-model"), &chat_req())
            .await
            .expect("explicit model works without pool");
        assert_eq!(resp.text.as_deref(), Some("done"));
        let err = router
            .call("default", LogSource::Agent)
            .chat(None, &chat_req())
            .await
            .expect_err("no default without pool");
        assert!(matches!(err, AppError::BadRequest(_)), "{err}");
    }
}
