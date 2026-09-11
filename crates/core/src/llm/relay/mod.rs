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

pub use auth::generate_sk;
pub use billing::QUOTA_PER_USD;

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
    let estimate = billing::estimate_precharge(&info.pricing, info.model_type, &body);
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
    let cache = router.cache_snapshot();
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
                return openai_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request_error",
                    err.to_string(),
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
                if stream {
                    let inner = crate::llm::relay::adaptor::OpenaiAdaptor::handle_stream(
                        upstream,
                        usage_cell.clone(),
                    );
                    let settle_stream = SettleStream {
                        inner,
                        state: SettleCtx {
                            pool: state.pool.clone(),
                            charge: charge.clone(),
                            pricing: info.pricing.clone(),
                            usage: usage_cell,
                            router: router.clone(),
                            tenant: tenant.clone(),
                            token_id: Some(token_id),
                            user_id: Some(user_id),
                            channel_id: channel.id,
                            key_index,
                            model: model.clone(),
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
                        let actual = billing::settle_quota(&info.pricing, &usage);
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
                            detail: Some(serde_json::json!({
                                "pre_consumed": charge.pre_consumed,
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
                    }
                }
            }
            Ok(upstream) => {
                let status = upstream.status().as_u16();
                let text = upstream.text().await.unwrap_or_default();
                attempts_log.push(format!("ch{} key{key_index}: {status}", channel.id.0));
                router.report_failure(
                    channel.id,
                    key_index,
                    &crate::llm::service::UpstreamFailure {
                        status: Some(status),
                        message: text.clone(),
                        retry_after: None,
                    },
                );
                last_error = Some((status, text));
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

/// Settlement context carried by the streaming response.
struct SettleCtx {
    pool: crate::db::Pool,
    charge: billing::PreCharge,
    pricing: crate::llm::cache::Pricing,
    usage: Arc<std::sync::Mutex<RelayUsage>>,
    router: Arc<LlmRouter>,
    tenant: String,
    token_id: Option<SnowflakeId>,
    user_id: Option<SnowflakeId>,
    channel_id: SnowflakeId,
    key_index: usize,
    model: String,
}

impl SettleCtx {
    fn settle_now(&mut self) {
        let usage = self.usage.lock().map(|u| u.clone()).unwrap_or_default();
        let actual = billing::settle_quota(&self.pricing, &usage);
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
            detail: Some(
                serde_json::json!({ "pre_consumed": self.charge.pre_consumed, "stream": true }),
            ),
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
