//! `/v1` relay endpoints (design §8, §4): the OpenAI-compatible public face.
//! Mounted at the application root (not `/api/v1`) — sk- auth, not JWT.

pub(crate) mod adaptor;
pub(crate) mod auth;
pub(crate) mod billing;

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::StreamExt;

use crate::AppState;
use crate::llm::models::log::{LogSource, NewLog};
use crate::llm::models::model::LlmModelType;
pub use crate::llm::relay::adaptor::shared_client;
use crate::llm::relay::adaptor::{RelayEndpoint, RelayUsage};
use crate::llm::service::{
    DEFAULT_RETRY_TIMES, LlmRouter, RELAY_TIER, ResolveCtx, RetryState, SlotError,
};
use crate::types::snowflake_id::SnowflakeId;

/// Resolve the sell-price multiplier for a Key's billing group from the
/// `llm_group_ratios` option (pricing.md §2). Unset/unknown group → 1.0.
/// Runtime-adjustable, so read per request.
async fn group_ratio_of(pool: &crate::db::Pool, group: &str) -> f64 {
    crate::llm::handler::read_group_ratios(pool)
        .await
        .get(group)
        .copied()
        .unwrap_or(1.0)
}

pub use auth::generate_sk;
pub use crate::types::quota::QUOTA_PER_USD;

/// Hash helper re-exported for the token self-service endpoints.
pub fn auth_hash(plain: &str) -> String {
    auth::hash_sk(plain)
}

/// Header builder re-exported for the channel test endpoint.
pub struct OpenaiHeaders;

impl OpenaiHeaders {
    /// Auth + channel header overrides.
    pub fn for_key(
        key: &str,
        header_override: Option<&serde_json::Value>,
    ) -> axum::http::HeaderMap {
        adaptor::OpenaiAdaptor::setup_headers(key, header_override)
    }
}

/// Register `/v1` routes on the application root.
pub fn routes() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(chat_completions),
        )
        .route("/v1/models", axum::routing::get(list_models))
}

/// Error body in the OpenAI-compatible wire shape (design §8.5): SDKs parse errors from this
/// envelope, bare JSON/text breaks them.
fn openai_error(status: StatusCode, typ: &str, message: String) -> Response {
    let body = serde_json::json!({
        "error": { "message": message, "type": typ, "code": null }
    });
    (status, Json(body)).into_response()
}

fn bearer_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned)
}

fn client_ip_of(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .unwrap_or_default()
        .to_owned()
}

async fn write_log(state: &AppState, log: NewLog) {
    if let Err(err) = crate::llm::models::log::insert_log(&state.pool, log).await {
        tracing::warn!(%err, "llm relay log insert failed");
    }
}

/// POST /v1/chat/completions — the full pipeline of design §4.
#[allow(clippy::too_many_lines)]
async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(bearer) = bearer_of(&headers) else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "missing bearer token".into(),
        );
    };
    let identity = match auth::authenticate(&state.pool, &bearer, &client_ip_of(&headers)).await {
        Ok(id) => id,
        Err(err) => {
            let status = match &err {
                crate::errors::app_error::AppError::Unauthorized => StatusCode::UNAUTHORIZED,
                crate::errors::app_error::AppError::Forbidden => StatusCode::FORBIDDEN,
                _ => StatusCode::UNAUTHORIZED,
            };
            return openai_error(status, "invalid_request_error", err.to_string());
        }
    };
    let token = identity.token;
    let tenant = token
        .tenant_id
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let (token_id, user_id) = auth::token_owner(&token);
    // Sell-price multiplier for the Key's billing group (pricing.md §2).
    let token_group = token
        .token_group
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let group_ratio = group_ratio_of(&state.pool, &token_group).await;

    let Some(model) = body
        .get("model")
        .and_then(|m| m.as_str())
        .map(str::to_owned)
    else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "missing model".into(),
        );
    };
    if !auth::model_allowed(&token, &model) {
        return openai_error(
            StatusCode::FORBIDDEN,
            "invalid_request_error",
            format!("model not allowed: {model}"),
        );
    }
    let Some(info) = state.llm_router.model_info(&tenant, &model) else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("unknown model: {model}"),
        );
    };
    if !matches!(info.model_type, LlmModelType::Chat | LlmModelType::Vlm) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("model is not a chat model: {model}"),
        );
    }
    let stream = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);

    // Pre-consume (§9.3) — rejection refunds nothing (no hold taken).
    let params_max_output = info
        .params
        .as_ref()
        .and_then(|p| p.get("max_output_tokens"))
        .and_then(serde_json::Value::as_i64);
    let estimate = billing::estimate_precharge(
        &info.pricing,
        info.model_type,
        &body,
        group_ratio,
        params_max_output,
    );
    let charge =
        match billing::pre_consume(&state.pool, token_id, token.unlimited_quota, estimate).await {
            Ok(c) => c,
            Err(_) => {
                return openai_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limit_error",
                    "insufficient quota".into(),
                );
            }
        };

    let router = state.llm_router.clone();
    let ctx = ResolveCtx {
        tenant: &tenant,
        group: None,
        pin_channel: None,
    };
    let mut retry = RetryState::default();
    let deadline = Instant::now() + Duration::from_secs(RELAY_TIER.max_wait_secs);
    let body_bytes = body.to_string().len();
    // §8.4 estimation source: prompt side of the no-usage fallback.
    let prompt_chars = billing::count_prompt_chars(&body);

    let mut attempts_log: Vec<String> = Vec::new();
    let mut last_error: Option<(u16, String)> = None;

    for attempt in 0..=DEFAULT_RETRY_TIMES {
        // Fresh snapshot per attempt: an arrears-banned key evicts its
        // channel from the routing table mid-request — a stale snapshot
        // would keep retrying the banned channel (design §6.2).
        let cache = router.cache_snapshot();
        let acquired = router
            .acquire_slot(
                &crate::llm::service::SlotRequest {
                    cache: &cache,
                    ctx: &ctx,
                    model: &model,
                    tier: RELAY_TIER,
                    caller: Some((token_id, user_id)),
                    body_bytes,
                    deadline,
                },
                &mut retry,
            )
            .await;
        let (channel, key_index, _permit) = match acquired {
            Ok(triple) => triple,
            Err(SlotError::NoRoute(err)) => {
                billing::refund_all(&state.pool, &charge).await;
                // When the route died mid-request (this request's failure
                // banned the only channel, or cooldowns evicted everything),
                // prefer the real upstream error over the routing-level
                // message — clients should see why it actually failed.
                let (status, message) = last_error.clone().unwrap_or((400, err.to_string()));
                let typ = if status == 429 {
                    "rate_limit_error"
                } else {
                    "api_error"
                };
                return openai_error(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
                    typ,
                    message,
                );
            }
            Err(SlotError::Rejected {
                status,
                message,
                retry_after_secs,
            }) => {
                billing::refund_all(&state.pool, &charge).await;
                let mut hdrs = HeaderMap::new();
                if let Some(secs) = retry_after_secs
                    && let Ok(v) = secs.to_string().parse()
                {
                    hdrs.insert("retry-after", v);
                }
                let typ = if status == 503 {
                    "server_error"
                } else {
                    "rate_limit_error"
                };
                let resp = openai_error(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS),
                    typ,
                    message,
                );
                return (
                    StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS),
                    hdrs,
                    resp,
                )
                    .into_response();
            }
        };

        let upstream_model = crate::llm::service::mapped_model(&channel, &model);
        let converted = OpenaiBody::convert(
            body.clone(),
            &upstream_model,
            channel.param_override.as_ref(),
            stream,
        );
        let url = adaptor_request_url(&channel.base_url);
        let client = shared_client();
        let resp = client
            .post(&url)
            .headers(crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
                &channel.keys[key_index].plain.clone().unwrap_or_default(),
                channel.header_override.as_ref(),
            ))
            .json(&converted)
            .send()
            .await;

        match resp {
            Ok(upstream) if upstream.status().is_success() => {
                let usage_cell = Arc::new(std::sync::Mutex::new(RelayUsage::default()));
                let chars_cell = Arc::new(std::sync::Mutex::new(0usize));
                if stream {
                    let inner = crate::llm::relay::adaptor::OpenaiAdaptor::handle_stream(
                        upstream,
                        usage_cell.clone(),
                        chars_cell.clone(),
                    );
                    // §7.6 槽生命周期：流终止（含客户端中断 drop）才释放槽
                    // 与在飞计数 —— permit 必须随流存活，不能随 handler 返回。
                    let settle_stream = SettleStream {
                        inner,
                        state: SettleCtx {
                            pool: state.pool.clone(),
                            charge: charge.clone(),
                            pricing: info.pricing.clone(),
                            usage: usage_cell,
                            content_chars: chars_cell,
                            prompt_chars,
                            group_ratio,
                            cost_mode: channel.cost_mode,
                            cost_discount: channel.cost_discount,
                            router: router.clone(),
                            tenant: tenant.clone(),
                            token_id: Some(token_id),
                            user_id: Some(user_id),
                            channel_id: channel.id,
                            key_index,
                            model: model.clone(),
                            _permit,
                        },
                        settled: false,
                    };
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header(header::CONTENT_TYPE, "text/event-stream")
                        .header(header::CACHE_CONTROL, "no-cache")
                        .body(Body::from_stream(settle_stream))
                        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
                }
                match crate::llm::relay::adaptor::OpenaiAdaptor::handle_response(upstream).await {
                    Ok((resp_body, usage)) => {
                        let usage = estimate_usage_if_missing(&body, &resp_body, usage);
                        let actual = billing::settle_quota(&info.pricing, &usage, group_ratio);
                        let cost = billing::cost_quota(
                            &info.pricing,
                            &usage,
                            channel.cost_mode,
                            channel.cost_discount,
                        );
                        billing::settle(&state.pool, &charge, actual).await;
                        router.report_success(channel.id, key_index);
                        router.record_latency(&tenant, &model, 1.0);
                        let log = NewLog {
                            tenant_id: Some(tenant.clone()),
                            user_id: Some(user_id),
                            token_id: Some(token_id),
                            source: LogSource::Relay,
                            channel_id: Some(channel.id),
                            key_index: Some(key_index as i32),
                            model_name: model.clone(),
                            is_stream: false,
                            prompt_tokens: usage.prompt_tokens as i32,
                            completion_tokens: usage.completion_tokens as i32,
                            cache_read_tokens: usage.cache_read_tokens as i32,
                            cache_write_tokens: usage.cache_write_tokens as i32,
                            quota: actual,
                            cost_quota: cost,
                            detail: Some(serde_json::json!({
                                "pre_consumed": charge.pre_consumed,
                                "group_ratio": group_ratio,
                                "cost_mode": channel.cost_mode.as_str(),
                                "cost_discount": channel.cost_discount,
                                "attempts": attempts_log.len() + 1,
                            })),
                            elapsed_ms: None,
                            status_code: Some(200),
                            error_message: None,
                            request_id: None,
                        };
                        write_log(&state, log).await;
                        return (StatusCode::OK, Json(resp_body)).into_response();
                    }
                    Err(err) => {
                        attempts_log.push(format!("ch{} parse: {err}", channel.id.0));
                        last_error = Some((502, err.to_string()));
                        // §7.3：解析失败不重试——上游响应体异常，重发大概率同果。
                        break;
                    }
                }
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                // §6.2 头信号：Retry-After / *-ratelimit-*-reset 是窗口判定
                // 与冷却时长在文案之外的另一半数据源（内部 execute 路径
                // ProviderError 不携带响应头，仅靠文案）。
                let retry_after =
                    crate::llm::relay::adaptor::parse_reset_deadline(upstream.headers());
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
                // §6.2 class 4)：400 是客户端自身的错误，key 无过错——豁免
                // 失败上报（不冷却不封禁不计连续失败）。其余照常上报：
                // 401-403 触发欠费封禁判定，超时类冷却分流（§7.3）。
                if status != 400 {
                    router.report_failure(
                        channel.id,
                        key_index,
                        &crate::llm::service::UpstreamFailure {
                            status: Some(status),
                            message: text.clone(),
                            retry_after,
                        },
                    );
                }
                last_error = Some((status, text));
                // §7.3 重试边界：400 确定性失败；408/504/524 超时类——上游
                // 可能已处理已计费，重试 = 双重计费。其余渠道侧错误换渠道重试。
                if matches!(status, 400 | 408 | 504 | 524) {
                    break;
                }
            }
            Err(err) => {
                attempts_log.push(format!(
                    "ch{} key{key_index}: transport {err}",
                    channel.id.0
                ));
                router.report_failure(
                    channel.id,
                    key_index,
                    &crate::llm::service::UpstreamFailure {
                        status: None,
                        message: err.to_string(),
                        retry_after: None,
                    },
                );
                last_error = Some((502, err.to_string()));
            }
        }
        if attempt < DEFAULT_RETRY_TIMES {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    billing::refund_all(&state.pool, &charge).await;
    let (status, message) = last_error.unwrap_or((502, "upstream failed".to_owned()));
    let status = StatusCode::from_u16(if (400..600).contains(&status) {
        status
    } else {
        502
    })
    .unwrap_or(StatusCode::BAD_GATEWAY);
    openai_error(status, "api_error", message)
}

/// GET /v1/models — token-visible models (design §8.1):
/// channel models ∩ active directory ∩ token allowlist.
async fn list_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(bearer) = bearer_of(&headers) else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "missing bearer token".into(),
        );
    };
    let identity = match auth::authenticate(&state.pool, &bearer, &client_ip_of(&headers)).await {
        Ok(id) => id,
        Err(err) => {
            return openai_error(
                StatusCode::UNAUTHORIZED,
                "invalid_request_error",
                err.to_string(),
            );
        }
    };
    let token = identity.token;
    let tenant = token
        .tenant_id
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let cache = state.llm_router.cache_snapshot();
    let mut names: Vec<String> = cache
        .channels
        .values()
        .filter(|ch| ch.tenant == tenant)
        .flat_map(|ch| ch.models.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    let data: Vec<serde_json::Value> = names
        .into_iter()
        .filter(|name| auth::model_allowed(&token, name))
        .filter(|name| cache.model_info(&tenant, name).is_some())
        .map(|id| serde_json::json!({ "id": id, "object": "model", "owned_by": "raisfast" }))
        .collect();
    Json(serde_json::json!({ "object": "list", "data": data })).into_response()
}

fn adaptor_request_url(base_url: &str) -> String {
    crate::llm::relay::adaptor::OpenaiAdaptor::request_url(base_url, RelayEndpoint::ChatCompletions)
}

/// body conversion helper namespace (keeps the handler readable).
struct OpenaiBody;

impl OpenaiBody {
    fn convert(
        body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
        stream: bool,
    ) -> serde_json::Value {
        match crate::llm::relay::adaptor::OpenaiAdaptor::convert_chat(
            body.clone(),
            upstream_model,
            param_override,
            stream,
        ) {
            Ok(converted) => converted,
            Err(_) => body,
        }
    }
}

/// §8.4 no-usage estimation fallback: upstreams that report no usage at
/// all (some compatible implementations) get billed by char estimation
/// (chars/4) with a warning — never a zero-cost free ride.
fn estimate_usage_if_missing(
    req: &serde_json::Value,
    resp: &serde_json::Value,
    usage: RelayUsage,
) -> RelayUsage {
    if usage.prompt_tokens != 0 || usage.completion_tokens != 0 {
        return usage;
    }
    let completion_chars: usize = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|ch| {
                    ch.get("message")
                        .and_then(|m| m.get("content"))
                        .and_then(|c| c.as_str())
                })
                .map(|s| s.chars().count())
                .sum()
        })
        .unwrap_or(0);
    let est = RelayUsage::estimate(billing::count_prompt_chars(req), completion_chars);
    tracing::warn!(
        prompt_tokens = est.prompt_tokens,
        completion_tokens = est.completion_tokens,
        "llm upstream returned no usage; billing by char estimation"
    );
    est
}

/// Settlement context carried by the streaming response.
struct SettleCtx {
    pool: crate::db::Pool,
    charge: billing::PreCharge,
    pricing: crate::llm::cache::Pricing,
    usage: Arc<std::sync::Mutex<RelayUsage>>,
    /// Delta-content chars forwarded (completion side of §8.4 estimation).
    content_chars: Arc<std::sync::Mutex<usize>>,
    /// Request prompt chars (prompt side of §8.4 estimation).
    prompt_chars: usize,
    /// Sell-price multiplier of the Key's billing group (pricing.md §2).
    group_ratio: f64,
    /// Upstream cost model of the channel that served the stream (§7).
    cost_mode: crate::llm::models::channel::LlmCostMode,
    cost_discount: f64,
    router: Arc<LlmRouter>,
    tenant: String,
    token_id: Option<SnowflakeId>,
    user_id: Option<SnowflakeId>,
    channel_id: SnowflakeId,
    key_index: usize,
    model: String,
    /// Held until the stream terminates (design §7.6 — releasing earlier
    /// would let the next request reach an upstream that still counts this
    /// streaming connection against its concurrency quota).
    _permit: crate::llm::service::SlotPermit,
}

impl SettleCtx {
    fn settle_now(&mut self) {
        let mut usage = self.usage.lock().map(|u| u.clone()).unwrap_or_default();
        if usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
            // §8.4: upstream sent no usage frame — estimate by forwarded
            // content chars instead of settling (and billing) zero.
            let chars = self.content_chars.lock().map(|c| *c).unwrap_or(0);
            usage = RelayUsage::estimate(self.prompt_chars, chars);
            tracing::warn!(
                prompt_tokens = usage.prompt_tokens,
                completion_tokens = usage.completion_tokens,
                "llm stream ended without usage; billing by char estimation"
            );
        }
        let actual = billing::settle_quota(&self.pricing, &usage, self.group_ratio);
        let cost = billing::cost_quota(
            &self.pricing,
            &usage,
            self.cost_mode,
            self.cost_discount,
        );
        let charge = self.charge.clone();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            billing::settle(&pool, &charge, actual).await;
        });
        self.router.report_success(self.channel_id, self.key_index);
        let log = NewLog {
            tenant_id: Some(self.tenant.clone()),
            user_id: self.user_id,
            token_id: self.token_id,
            source: LogSource::Relay,
            channel_id: Some(self.channel_id),
            key_index: Some(self.key_index as i32),
            model_name: self.model.clone(),
            is_stream: true,
            prompt_tokens: usage.prompt_tokens as i32,
            completion_tokens: usage.completion_tokens as i32,
            cache_read_tokens: usage.cache_read_tokens as i32,
            cache_write_tokens: usage.cache_write_tokens as i32,
            quota: actual,
            cost_quota: cost,
            detail: Some(serde_json::json!({
                "pre_consumed": self.charge.pre_consumed,
                "group_ratio": self.group_ratio,
                "cost_mode": self.cost_mode.as_str(),
                "cost_discount": self.cost_discount,
                "stream": true,
            })),
            elapsed_ms: None,
            status_code: Some(200),
            error_message: None,
            request_id: None,
        };
        let pool = self.pool.clone();
        tokio::spawn(async move {
            if let Err(err) = crate::llm::models::log::insert_log(&pool, log).await {
                tracing::warn!(%err, "llm stream settle log failed");
            }
        });
    }
}

/// Stream wrapper firing settlement exactly once on termination — normal
/// end, upstream error, or client disconnect (Drop; Drop can't await so the
/// settle spawns, design §8.4).
struct SettleStream {
    inner: std::pin::Pin<Box<dyn futures::Stream<Item = Result<Vec<u8>, std::io::Error>> + Send>>,
    state: SettleCtx,
    settled: bool,
}

impl futures::Stream for SettleStream {
    type Item = Result<Vec<u8>, std::io::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.poll_next_unpin(cx) {
            std::task::Poll::Ready(Some(Ok(bytes))) => std::task::Poll::Ready(Some(Ok(bytes))),
            std::task::Poll::Ready(Some(Err(e))) => {
                self.finish();
                std::task::Poll::Ready(Some(Err(e)))
            }
            std::task::Poll::Ready(None) => {
                self.finish();
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl SettleStream {
    fn finish(&mut self) {
        if !self.settled {
            self.settled = true;
            self.state.settle_now();
        }
    }
}

impl Drop for SettleStream {
    fn drop(&mut self) {
        self.finish();
    }
}
