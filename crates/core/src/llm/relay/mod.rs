//! `/v1` relay endpoints (design §8, §4): the OpenAI-compatible public face.
//! Mounted at the application root (not `/api/v1`) — sk- auth, not JWT.

pub(crate) mod adaptor;
pub(crate) mod anthropic;
pub(crate) mod auth;
pub(crate) mod billing;
pub(crate) mod tasks;

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

pub use crate::types::quota::QUOTA_PER_USD;
pub use auth::generate_sk;

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
    // OpenAI allows 25MB audio uploads — lift the 2MB default body limit
    // for the multipart STT routes only.
    const AUDIO_BODY_LIMIT: usize = 25 * 1024 * 1024;
    axum::Router::new()
        .route(
            "/v1/chat/completions",
            axum::routing::post(chat_completions),
        )
        .route("/v1/messages", axum::routing::post(messages))
        .route("/v1/models", axum::routing::get(list_models))
        .route("/v1/embeddings", axum::routing::post(embeddings))
        .route("/v1/rerank", axum::routing::post(rerank))
        .route(
            "/v1/images/generations",
            axum::routing::post(images_generations),
        )
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(audio_transcriptions)
                .layer(axum::extract::DefaultBodyLimit::max(AUDIO_BODY_LIMIT)),
        )
        .route(
            "/v1/audio/translations",
            axum::routing::post(audio_translations)
                .layer(axum::extract::DefaultBodyLimit::max(AUDIO_BODY_LIMIT)),
        )
        .route("/v1/audio/speech", axum::routing::post(audio_speech))
        .route("/v1/videos", axum::routing::post(tasks::submit_video))
        .route("/v1/videos/{id}", axum::routing::get(tasks::get_video))
        .route(
            "/v1/videos/{id}/content",
            axum::routing::get(tasks::get_video_content),
        )
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

/// Client key: `x-api-key` (Anthropic SDK) with Bearer fallback.
fn api_key_of(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok())
        && !v.is_empty()
    {
        return Some(v.to_owned());
    }
    bearer_of(headers)
}

/// Error body in the Anthropic wire shape (SDKs parse `error.type`).
fn anthropic_error(status: StatusCode, typ: &str, message: String) -> Response {
    let body = serde_json::json!({
        "type": "error",
        "error": { "type": typ, "message": message },
    });
    (status, Json(body)).into_response()
}

/// HTTP status → anthropic error `type` vocabulary.
fn anthropic_error_type(status: u16) -> &'static str {
    match status {
        400 => "invalid_request_error",
        401 => "authentication_error",
        403 => "permission_error",
        404 => "not_found_error",
        413 => "request_too_large",
        429 => "rate_limit_error",
        503 => "overloaded_error",
        _ => "api_error",
    }
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
        // Protocol selection rides the channel `provider` (design §8.2):
        // anthropic-native channels speak /v1/messages, everything else is
        // OpenAI-compatible. The model name never picks the adaptor.
        let native_anthropic = channel.provider == "anthropic";
        let converted = if native_anthropic {
            OpenaiBody::convert_anthropic(
                body.clone(),
                &upstream_model,
                channel.param_override.as_ref(),
                stream,
            )
        } else {
            OpenaiBody::convert(
                body.clone(),
                &upstream_model,
                channel.param_override.as_ref(),
                stream,
            )
        };
        let url = if native_anthropic {
            anthropic::AnthropicAdaptor::request_url(&channel.base_url)
        } else {
            adaptor_request_url(&channel.base_url)
        };
        let upstream_key = &channel.keys[key_index].plain.clone().unwrap_or_default();
        let upstream_headers = if native_anthropic {
            anthropic::AnthropicAdaptor::setup_headers(
                upstream_key,
                channel.header_override.as_ref(),
            )
        } else {
            crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
                upstream_key,
                channel.header_override.as_ref(),
            )
        };
        let client = shared_client();
        let resp = client
            .post(&url)
            .headers(upstream_headers)
            .json(&converted)
            .send()
            .await;

        match resp {
            Ok(upstream) if upstream.status().is_success() => {
                let usage_cell = Arc::new(std::sync::Mutex::new(RelayUsage::default()));
                let chars_cell = Arc::new(std::sync::Mutex::new(0usize));
                if stream {
                    let inner = if native_anthropic {
                        anthropic::AnthropicAdaptor::handle_stream(
                            upstream,
                            usage_cell.clone(),
                            chars_cell.clone(),
                            &model,
                        )
                    } else {
                        crate::llm::relay::adaptor::OpenaiAdaptor::handle_stream(
                            upstream,
                            usage_cell.clone(),
                            chars_cell.clone(),
                        )
                    };
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
                let handled = if native_anthropic {
                    anthropic::AnthropicAdaptor::handle_response(upstream, &model).await
                } else {
                    crate::llm::relay::adaptor::OpenaiAdaptor::handle_response(upstream).await
                };
                match handled {
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
                            day: None,
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

/// POST /v1/messages — the Anthropic-native inbound face (design §8.2 P4):
/// Anthropic SDK / Claude Code clients hit the same routing + billing
/// pipeline as /v1/chat/completions. openai-compatible channels get the
/// request converted both ways; anthropic-native channels are a
/// near-passthrough (model rewrite + param_override). Errors always wear
/// the anthropic envelope. [照抄 new-api claude relay 双向 convert]
#[allow(clippy::too_many_lines)]
async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let Some(key) = api_key_of(&headers) else {
        return anthropic_error(
            StatusCode::UNAUTHORIZED,
            "authentication_error",
            "missing api key".into(),
        );
    };
    let identity = match auth::authenticate(&state.pool, &key, &client_ip_of(&headers)).await {
        Ok(id) => id,
        Err(err) => {
            let (status, typ) = match &err {
                crate::errors::app_error::AppError::Forbidden => {
                    (StatusCode::FORBIDDEN, "permission_error")
                }
                _ => (StatusCode::UNAUTHORIZED, "authentication_error"),
            };
            return anthropic_error(status, typ, err.to_string());
        }
    };
    let token = identity.token;
    let tenant = token
        .tenant_id
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let (token_id, user_id) = auth::token_owner(&token);
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
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "missing model".into(),
        );
    };
    if !auth::model_allowed(&token, &model) {
        return anthropic_error(
            StatusCode::FORBIDDEN,
            "permission_error",
            format!("model not allowed: {model}"),
        );
    }
    let Some(info) = state.llm_router.model_info(&tenant, &model) else {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("unknown model: {model}"),
        );
    };
    if !matches!(info.model_type, LlmModelType::Chat | LlmModelType::Vlm) {
        return anthropic_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("model is not a chat model: {model}"),
        );
    }
    let stream = body
        .get("stream")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    // Canonical openai body: the estimation + conversion source for
    // openai-compatible channels (anthropic channels forward the original).
    let openai_body = match anthropic::AnthropicAdaptor::to_openai_body(body.clone()) {
        Ok(b) => b,
        Err(err) => {
            return anthropic_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                err.to_string(),
            );
        }
    };

    // Pre-consume (§9.3) on the converted shape so the max_tokens chain and
    // prompt estimation match what the upstream will actually see.
    let params_max_output = info
        .params
        .as_ref()
        .and_then(|p| p.get("max_output_tokens"))
        .and_then(serde_json::Value::as_i64);
    let estimate = billing::estimate_precharge(
        &info.pricing,
        info.model_type,
        &openai_body,
        group_ratio,
        params_max_output,
    );
    let charge =
        match billing::pre_consume(&state.pool, token_id, token.unlimited_quota, estimate).await {
            Ok(c) => c,
            Err(_) => {
                return anthropic_error(
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
    let prompt_chars = billing::count_prompt_chars(&openai_body);
    let prompt_est = (prompt_chars / 4).max(1) as i64;
    let mut attempts_log: Vec<String> = Vec::new();
    let mut last_error: Option<(u16, String)> = None;

    for attempt in 0..=DEFAULT_RETRY_TIMES {
        // Fresh snapshot per attempt (design §6.2 — see chat pipeline).
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
                let (status, message) = last_error.clone().unwrap_or((400, err.to_string()));
                return anthropic_error(
                    StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
                    anthropic_error_type(status),
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
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
                let resp = anthropic_error(code, anthropic_error_type(status), message);
                return (code, hdrs, resp).into_response();
            }
        };

        let upstream_model = crate::llm::service::mapped_model(&channel, &model);
        let native_anthropic = channel.provider == "anthropic";
        let (url, upstream_headers) = if native_anthropic {
            (
                anthropic::AnthropicAdaptor::request_url(&channel.base_url),
                anthropic::AnthropicAdaptor::setup_headers(
                    &channel.keys[key_index].plain.clone().unwrap_or_default(),
                    channel.header_override.as_ref(),
                ),
            )
        } else {
            (
                adaptor_request_url(&channel.base_url),
                crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
                    &channel.keys[key_index].plain.clone().unwrap_or_default(),
                    channel.header_override.as_ref(),
                ),
            )
        };
        let fwd_body = if native_anthropic {
            match anthropic::AnthropicAdaptor::convert_native(
                body.clone(),
                &upstream_model,
                channel.param_override.as_ref(),
                stream,
            ) {
                Ok(b) => b,
                Err(err) => {
                    billing::refund_all(&state.pool, &charge).await;
                    return anthropic_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_request_error",
                        err.to_string(),
                    );
                }
            }
        } else {
            OpenaiBody::convert(
                openai_body.clone(),
                &upstream_model,
                channel.param_override.as_ref(),
                stream,
            )
        };
        let client = shared_client();
        let resp = client
            .post(&url)
            .headers(upstream_headers)
            .json(&fwd_body)
            .send()
            .await;

        match resp {
            Ok(upstream) if upstream.status().is_success() => {
                let usage_cell = Arc::new(std::sync::Mutex::new(RelayUsage::default()));
                let chars_cell = Arc::new(std::sync::Mutex::new(0usize));
                if stream {
                    let inner = if native_anthropic {
                        anthropic::AnthropicAdaptor::handle_passthrough_stream(
                            upstream,
                            usage_cell.clone(),
                            chars_cell.clone(),
                        )
                    } else {
                        anthropic::AnthropicAdaptor::handle_openai_stream(
                            upstream,
                            usage_cell.clone(),
                            chars_cell.clone(),
                            &model,
                            prompt_est,
                        )
                    };
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
                let handled = if native_anthropic {
                    anthropic::AnthropicAdaptor::read_message_response(upstream).await
                } else {
                    crate::llm::relay::adaptor::OpenaiAdaptor::handle_response(upstream).await
                };
                match handled {
                    Ok((upstream_body, parsed)) => {
                        // openai channels convert back to a claude message;
                        // anthropic channels pass the body through.
                        let (resp_body, usage) = if native_anthropic {
                            let usage = if parsed.prompt_tokens == 0
                                && parsed.completion_tokens == 0
                            {
                                let completion_chars =
                                    anthropic::AnthropicAdaptor::message_text_chars(&upstream_body);
                                RelayUsage::estimate(prompt_chars, completion_chars)
                            } else {
                                parsed
                            };
                            (upstream_body, usage)
                        } else {
                            let usage =
                                estimate_usage_if_missing(&openai_body, &upstream_body, parsed);
                            (
                                anthropic::AnthropicAdaptor::completion_to_message(
                                    &upstream_body,
                                    &model,
                                ),
                                usage,
                            )
                        };
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
                                "anthropic_face": true,
                                "attempts": attempts_log.len() + 1,
                            })),
                            elapsed_ms: None,
                            status_code: Some(200),
                            error_message: None,
                            request_id: None,
                            day: None,
                        };
                        write_log(&state, log).await;
                        return (StatusCode::OK, Json(resp_body)).into_response();
                    }
                    Err(err) => {
                        attempts_log.push(format!("ch{} parse: {err}", channel.id.0));
                        last_error = Some((502, err.to_string()));
                        break;
                    }
                }
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                let retry_after =
                    crate::llm::relay::adaptor::parse_reset_deadline(upstream.headers());
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
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
    anthropic_error(status, anthropic_error_type(status.as_u16()), message)
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

// ── non-chat modalities: /v1/embeddings + /v1/rerank ──────────────
// Shared non-streaming pipeline (chat §4 minus the streaming branch):
// the retry/settle/log loop is identical, only the URL, model-type guard
// and usage extraction vary per modality.

/// Per-modality relay parameters for the shared non-streaming pipeline.
struct JsonRelaySpec {
    endpoint: RelayEndpoint,
    /// Guard on the directory model_type (chat models are rejected here).
    type_ok: fn(&LlmModelType) -> bool,
    /// Extract RelayUsage from a successful upstream response body.
    usage_of: fn(&serde_json::Value) -> RelayUsage,
    /// §8.4 no-usage fallback: estimate from the request text payload.
    estimate_of: fn(&serde_json::Value) -> RelayUsage,
}

const EMBEDDINGS_SPEC: JsonRelaySpec = JsonRelaySpec {
    endpoint: RelayEndpoint::Embeddings,
    type_ok: |t| matches!(t, LlmModelType::Embedding),
    usage_of: |v| {
        v.get("usage")
            .map(crate::llm::relay::adaptor::RelayUsage::from_openai)
            .unwrap_or_default()
    },
    estimate_of: |req| RelayUsage::estimate(billing::count_prompt_chars(req), 0),
};

const RERANK_SPEC: JsonRelaySpec = JsonRelaySpec {
    endpoint: RelayEndpoint::Rerank,
    type_ok: |t| matches!(t, LlmModelType::Rerank),
    usage_of: |v| {
        // Jina/Cohere shape: usage.total_tokens (prompt_tokens optional).
        let prompt = v
            .get("usage")
            .and_then(|u| {
                u.get("prompt_tokens")
                    .and_then(|x| x.as_i64())
                    .or_else(|| u.get("total_tokens").and_then(|x| x.as_i64()))
            })
            .unwrap_or(0);
        RelayUsage {
            prompt_tokens: prompt,
            completion_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    },
    estimate_of: |req| RelayUsage::estimate(billing::count_prompt_chars(req), 0),
};

const IMAGES_SPEC: JsonRelaySpec = JsonRelaySpec {
    endpoint: RelayEndpoint::Images,
    type_ok: |t| matches!(t, LlmModelType::Image),
    usage_of: |v| {
        // Generated image count from the response `data` array; images
        // responses carry no usage object.
        let images = v
            .get("data")
            .and_then(|d| d.as_array())
            .map(|a| a.len())
            .unwrap_or(0) as i64;
        RelayUsage {
            prompt_tokens: 1,
            completion_tokens: images,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    },
    estimate_of: |req| {
        let n = req
            .get("n")
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .unwrap_or(1);
        RelayUsage {
            prompt_tokens: 1,
            completion_tokens: n,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    },
};

/// POST /v1/embeddings — OpenAI-compatible vectorization relay.
async fn embeddings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    relay_json(state, headers, body, &EMBEDDINGS_SPEC).await
}

/// POST /v1/rerank — Jina/Cohere-style reranking relay.
async fn rerank(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    relay_json(state, headers, body, &RERANK_SPEC).await
}

/// POST /v1/images/generations — OpenAI-compatible text-to-image relay.
/// Billing convention (token mode): `prompt_tokens` = 1 (input_price =
/// per-prompt), `completion_tokens` = generated image count (output_price
/// = per-image); `per_call` mode bills the flat call price per request.
async fn images_generations(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Response {
    relay_json(state, headers, body, &IMAGES_SPEC).await
}

/// Shared non-streaming relay pipeline (chat §4 minus streaming): auth →
/// model guard → pre-consume → slot/retry loop → settle → log.
#[allow(clippy::too_many_lines)]
async fn relay_json(
    state: AppState,
    headers: HeaderMap,
    body: serde_json::Value,
    spec: &JsonRelaySpec,
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
    if !(spec.type_ok)(&info.model_type) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!(
                "model {} is not valid for this endpoint: {}",
                model,
                info.model_type.as_str()
            ),
        );
    }

    let estimate =
        billing::estimate_precharge(&info.pricing, info.model_type, &body, group_ratio, None);
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
    let mut attempts_log: Vec<String> = Vec::new();
    let mut last_error: Option<(u16, String)> = None;

    for attempt in 0..=DEFAULT_RETRY_TIMES {
        // Fresh snapshot per attempt (design §6.2 — see chat pipeline).
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
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
                let typ = if status == 503 {
                    "server_error"
                } else {
                    "rate_limit_error"
                };
                let resp = openai_error(code, typ, message);
                return (code, hdrs, resp).into_response();
            }
        };

        let upstream_model = crate::llm::service::mapped_model(&channel, &model);
        // anthropic-native channels speak /v1/messages only — skip them for
        // OpenAI-shaped modalities (config mismatch is not an upstream
        // health signal, so no failure report; retry picks another channel).
        if channel.provider == "anthropic" {
            attempts_log.push(format!(
                "ch{} skip: anthropic serves chat only",
                channel.id.0
            ));
            last_error = Some((
                400,
                "provider 'anthropic' does not support this endpoint".to_owned(),
            ));
            continue;
        }
        let converted = match crate::llm::relay::adaptor::OpenaiAdaptor::convert_plain(
            body.clone(),
            &upstream_model,
            channel.param_override.as_ref(),
        ) {
            Ok(converted) => converted,
            Err(err) => {
                return openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    err.to_string(),
                );
            }
        };
        let url = crate::llm::relay::adaptor::OpenaiAdaptor::request_url(
            &channel.base_url,
            spec.endpoint,
        );
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
                match crate::llm::relay::adaptor::OpenaiAdaptor::handle_response(upstream).await {
                    Ok((resp_body, parsed)) => {
                        let mut usage = (spec.usage_of)(&resp_body);
                        if parsed.prompt_tokens > 0 || parsed.completion_tokens > 0 {
                            usage = parsed;
                        }
                        if usage.prompt_tokens == 0 && usage.completion_tokens == 0 {
                            // §8.4 no-usage fallback — estimate from the
                            // request payload instead of billing zero.
                            usage = (spec.estimate_of)(&body);
                            tracing::warn!(
                                prompt_tokens = usage.prompt_tokens,
                                "llm upstream returned no usage; billing by char estimation"
                            );
                        }
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
                            day: None,
                        };
                        write_log(&state, log).await;
                        return (StatusCode::OK, Json(resp_body)).into_response();
                    }
                    Err(err) => {
                        attempts_log.push(format!("ch{} parse: {err}", channel.id.0));
                        last_error = Some((502, err.to_string()));
                        break;
                    }
                }
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                let retry_after =
                    crate::llm::relay::adaptor::parse_reset_deadline(upstream.headers());
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
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

// ── audio modalities: /v1/audio/* ────────────────────────────────

/// POST /v1/audio/transcriptions — speech-to-text (multipart).
async fn audio_transcriptions(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> Response {
    audio_stt(
        state,
        headers,
        &mut multipart,
        RelayEndpoint::AudioTranscriptions,
    )
    .await
}

/// POST /v1/audio/translations — speech-to-text translated to English.
async fn audio_translations(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: axum::extract::Multipart,
) -> Response {
    audio_stt(
        state,
        headers,
        &mut multipart,
        RelayEndpoint::AudioTranslations,
    )
    .await
}

/// One buffered multipart field (rebuilt into a reqwest Form per attempt).
struct SttField {
    name: String,
    filename: Option<String>,
    mime: Option<String>,
    bytes: Vec<u8>,
}

/// Shared STT pipeline: multipart in → JSON out; billed by audio duration
/// (seconds × input_price; `per_call` = flat). Duration comes from the
/// upstream `duration` field (verbose_json) or a file-size estimate.
#[allow(clippy::too_many_lines)]
async fn audio_stt(
    state: AppState,
    headers: HeaderMap,
    multipart: &mut axum::extract::Multipart,
    endpoint: RelayEndpoint,
) -> Response {
    // Buffer the multipart once (25MB cap — DefaultBodyLimit guards it).
    let mut fields: Vec<SttField> = Vec::new();
    let mut model = String::new();
    let mut file_bytes_total = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(err) => {
                return openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    format!("invalid multipart body: {err}"),
                );
            }
        };
        let name = field.name().unwrap_or_default().to_owned();
        let filename = field.file_name().map(str::to_owned);
        let mime = field.content_type().map(str::to_owned);
        let bytes = field.bytes().await.unwrap_or_default().to_vec();
        if name == "model"
            && filename.is_none()
            && let Ok(m) = std::str::from_utf8(&bytes)
        {
            model = m.trim().to_owned();
        }
        file_bytes_total += bytes.len();
        fields.push(SttField {
            name,
            filename,
            mime,
            bytes,
        });
    }

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
    let (token_id, user_id) = auth::token_owner(&token);
    let token_group = token
        .token_group
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let group_ratio = group_ratio_of(&state.pool, &token_group).await;

    if model.is_empty() {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "missing model".into(),
        );
    }
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
    if !matches!(info.model_type, LlmModelType::Asr) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!(
                "model {} is not an asr model: {}",
                model,
                info.model_type.as_str()
            ),
        );
    }

    // Pre-charge: per-call flat, else duration estimate from file size.
    let est_secs = billing::estimate_audio_secs(file_bytes_total);
    let estimate = match info.pricing.price_mode {
        crate::llm::models::model::LlmPriceMode::PerCall => {
            billing::settle_quota(&info.pricing, &RelayUsage::default(), group_ratio)
        }
        _ => billing::flat_token_quota(info.pricing.input_price, est_secs, group_ratio),
    };
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
    let mut attempts_log: Vec<String> = Vec::new();
    let mut last_error: Option<(u16, String)> = None;

    for attempt in 0..=DEFAULT_RETRY_TIMES {
        let cache = router.cache_snapshot();
        let acquired = router
            .acquire_slot(
                &crate::llm::service::SlotRequest {
                    cache: &cache,
                    ctx: &ctx,
                    model: &model,
                    tier: RELAY_TIER,
                    caller: Some((token_id, user_id)),
                    body_bytes: file_bytes_total,
                    deadline,
                },
                &mut retry,
            )
            .await;
        let (channel, key_index, _permit) = match acquired {
            Ok(triple) => triple,
            Err(SlotError::NoRoute(err)) => {
                billing::refund_all(&state.pool, &charge).await;
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
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
                let typ = if status == 503 {
                    "server_error"
                } else {
                    "rate_limit_error"
                };
                let resp = openai_error(code, typ, message);
                return (code, hdrs, resp).into_response();
            }
        };

        let upstream_model = crate::llm::service::mapped_model(&channel, &model);
        // anthropic-native channels serve chat only (see relay_json guard).
        if channel.provider == "anthropic" {
            attempts_log.push(format!(
                "ch{} skip: anthropic serves chat only",
                channel.id.0
            ));
            last_error = Some((
                400,
                "provider 'anthropic' does not support this endpoint".to_owned(),
            ));
            continue;
        }
        // Rebuild the multipart form per attempt: model rewritten, other
        // fields (file, language, prompt, ...) passed through untouched.
        let mut form = reqwest::multipart::Form::new();
        for f in &fields {
            let mut part = reqwest::multipart::Part::bytes(f.bytes.clone());
            if let Some(fname) = f.filename.clone().or_else(|| Some(f.name.clone())) {
                part = part.file_name(fname);
            }
            let part = match &f.mime {
                Some(mime) => part
                    .mime_str(mime)
                    .unwrap_or_else(|_| reqwest::multipart::Part::bytes(f.bytes.clone())),
                None => part,
            };
            let part = if f.name == "model" {
                reqwest::multipart::Part::text(upstream_model.clone())
            } else {
                part
            };
            form = form.part(f.name.clone(), part);
        }
        let url =
            crate::llm::relay::adaptor::OpenaiAdaptor::request_url(&channel.base_url, endpoint);
        let client = shared_client();
        let resp = client
            .post(&url)
            .headers(crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
                &channel.keys[key_index].plain.clone().unwrap_or_default(),
                channel.header_override.as_ref(),
            ))
            .multipart(form)
            .send()
            .await;

        match resp {
            Ok(upstream) if upstream.status().is_success() => {
                match crate::llm::relay::adaptor::OpenaiAdaptor::handle_response(upstream).await {
                    Ok((resp_body, _)) => {
                        // Duration from verbose_json `duration`, else the
                        // file-size estimate (§8.4 spirit: never free).
                        let secs = resp_body
                            .get("duration")
                            .and_then(|d| d.as_f64())
                            .map(|d| d.ceil() as i64)
                            .filter(|d| *d > 0)
                            .unwrap_or(est_secs);
                        let usage = RelayUsage {
                            prompt_tokens: secs,
                            completion_tokens: 0,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                        };
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
                            completion_tokens: 0,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                            quota: actual,
                            cost_quota: cost,
                            detail: Some(serde_json::json!({
                                "pre_consumed": charge.pre_consumed,
                                "group_ratio": group_ratio,
                                "cost_mode": channel.cost_mode.as_str(),
                                "cost_discount": channel.cost_discount,
                                "audio_secs": secs,
                                "duration_estimated": !resp_body
                                    .get("duration")
                                    .is_some_and(|d| d.as_f64().is_some_and(|d| d > 0.0)),
                                "attempts": attempts_log.len() + 1,
                            })),
                            elapsed_ms: None,
                            status_code: Some(200),
                            error_message: None,
                            request_id: None,
                            day: None,
                        };
                        write_log(&state, log).await;
                        return (StatusCode::OK, Json(resp_body)).into_response();
                    }
                    Err(err) => {
                        attempts_log.push(format!("ch{} parse: {err}", channel.id.0));
                        last_error = Some((502, err.to_string()));
                        break;
                    }
                }
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                let retry_after =
                    crate::llm::relay::adaptor::parse_reset_deadline(upstream.headers());
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
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

/// POST /v1/audio/speech — text-to-speech: JSON in, binary audio out.
/// Billed entirely on the request side (input chars × input_price), so
/// settlement happens before the audio streams — client aborts can't
/// under-bill (§9.3 input-side semantics).
async fn audio_speech(
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
    let (token_id, user_id) = auth::token_owner(&token);
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
    if !matches!(info.model_type, LlmModelType::Tts) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!(
                "model {} is not a tts model: {}",
                model,
                info.model_type.as_str()
            ),
        );
    }

    // Billable unit known upfront: input chars (token mode) / flat per call.
    let chars = billing::count_prompt_chars(&body).max(1) as i64;
    let usage = RelayUsage {
        prompt_tokens: chars,
        completion_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    let estimate = billing::settle_quota(&info.pricing, &usage, group_ratio);
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
    let mut attempts_log: Vec<String> = Vec::new();
    let mut last_error: Option<(u16, String)> = None;

    for attempt in 0..=DEFAULT_RETRY_TIMES {
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
                let code = StatusCode::from_u16(status).unwrap_or(StatusCode::TOO_MANY_REQUESTS);
                let typ = if status == 503 {
                    "server_error"
                } else {
                    "rate_limit_error"
                };
                let resp = openai_error(code, typ, message);
                return (code, hdrs, resp).into_response();
            }
        };

        let upstream_model = crate::llm::service::mapped_model(&channel, &model);
        // anthropic-native channels serve chat only (see relay_json guard).
        if channel.provider == "anthropic" {
            attempts_log.push(format!(
                "ch{} skip: anthropic serves chat only",
                channel.id.0
            ));
            last_error = Some((
                400,
                "provider 'anthropic' does not support this endpoint".to_owned(),
            ));
            continue;
        }
        let converted = match crate::llm::relay::adaptor::OpenaiAdaptor::convert_plain(
            body.clone(),
            &upstream_model,
            channel.param_override.as_ref(),
        ) {
            Ok(converted) => converted,
            Err(err) => {
                return openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    err.to_string(),
                );
            }
        };
        let url = crate::llm::relay::adaptor::OpenaiAdaptor::request_url(
            &channel.base_url,
            RelayEndpoint::AudioSpeech,
        );
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
                // Input-side billing: settle now (actual == hold), then
                // stream the audio through untouched.
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
                    completion_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    quota: actual,
                    cost_quota: cost,
                    detail: Some(serde_json::json!({
                        "pre_consumed": charge.pre_consumed,
                        "group_ratio": group_ratio,
                        "cost_mode": channel.cost_mode.as_str(),
                        "cost_discount": channel.cost_discount,
                        "tts_chars": usage.prompt_tokens,
                        "attempts": attempts_log.len() + 1,
                    })),
                    elapsed_ms: None,
                    status_code: Some(200),
                    error_message: None,
                    request_id: None,
                    day: None,
                };
                write_log(&state, log).await;
                let content_type = upstream
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("audio/mpeg")
                    .to_owned();
                let stream = upstream.bytes_stream();
                return Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, content_type)
                    .body(Body::from_stream(stream))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                let retry_after =
                    crate::llm::relay::adaptor::parse_reset_deadline(upstream.headers());
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
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

    /// openai canonical → anthropic native (anthropic channels, design §8.3).
    fn convert_anthropic(
        body: serde_json::Value,
        upstream_model: &str,
        param_override: Option<&serde_json::Value>,
        stream: bool,
    ) -> serde_json::Value {
        match anthropic::AnthropicAdaptor::convert_chat(
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
        let cost = billing::cost_quota(&self.pricing, &usage, self.cost_mode, self.cost_discount);
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
            day: None,
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
