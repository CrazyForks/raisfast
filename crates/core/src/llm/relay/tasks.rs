//! `/v1/videos` — async video generation relay (OpenAI Videos API shape)
//! on top of the generic `llm_tasks` table. Submit is synchronous
//! (pre-charged, failover-able); completion is poll-on-read: `GET
//! /v1/videos/{id}` lazily polls the upstream, settles on completion and
//! refunds on failure/expiry. A cron sweep (`llm_task_sweep`) expires
//! abandoned tasks so unattended holds never leak.

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::errors::app_error::{AppError, AppResult};
use crate::llm::models::log::{LogSource, NewLog};
use crate::llm::models::task::{self, LlmTask, LlmTaskKind, LlmTaskStatus};
use crate::llm::relay::billing::{self, PreCharge};
use crate::llm::relay::{RelayUsage, bearer_of, client_ip_of, openai_error, shared_client};
use crate::types::quota::Quota;
use crate::types::snowflake_id::{SnowflakeId, parse_id};
use crate::utils::tz::now_utc;

use std::time::{Duration, Instant};

use crate::llm::models::model::LlmModelType;
use crate::llm::service::{DEFAULT_RETRY_TIMES, RELAY_TIER, ResolveCtx, RetryState, SlotError};

/// Task TTL: tasks past this are expired + refunded (abandoned clients
/// can't hold quota forever).
const TASK_TTL_SECS: i64 = 30 * 60;

/// POST /v1/videos — submit: pre-charge → channel select (failover on
/// submit errors) → upstream submit → task row → queued response.
#[allow(clippy::too_many_lines)]
pub(crate) async fn submit_video(
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
    let identity =
        match super::auth::authenticate(&state.pool, &bearer, &client_ip_of(&headers)).await {
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
    let (token_id, user_id) = super::auth::token_owner(&token);
    let token_group = token
        .token_group
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let group_ratio = super::group_ratio_of(&state.pool, &token_group).await;

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
    if !super::auth::model_allowed(&token, &model) {
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
    if !matches!(info.model_type, LlmModelType::Video) {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!(
                "model {} is not a video model: {}",
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
        // anthropic-native channels serve chat only (see relay/mod.rs
        // relay_json guard) — config mismatch, not an upstream failure.
        if channel.provider == "anthropic" {
            last_error = Some((
                400,
                "provider 'anthropic' does not support this endpoint".to_owned(),
            ));
            continue;
        }
        let converted = crate::llm::relay::adaptor::OpenaiAdaptor::convert_plain(
            body.clone(),
            &upstream_model,
            channel.param_override.as_ref(),
        )
        .unwrap_or_else(|_| body.clone());
        let url = crate::llm::relay::adaptor::OpenaiAdaptor::request_url(
            &channel.base_url,
            crate::llm::relay::adaptor::RelayEndpoint::Videos,
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
                    Ok((resp_body, _)) => {
                        let Some(upstream_id) = resp_body
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                        else {
                            last_error =
                                Some((502, "upstream submit response missing id".to_owned()));
                            break;
                        };
                        // Task snapshot: everything settle needs later
                        // (group ratio at submit, seconds for billing).
                        let payload = serde_json::json!({
                            "request": body,
                            "group_ratio": group_ratio,
                            "seconds": body
                                .get("seconds")
                                .cloned()
                                .unwrap_or_else(|| serde_json::Value::String("12".to_owned())),
                        });
                        let expires_at = now_utc() + chrono::Duration::seconds(TASK_TTL_SECS);
                        let created = match task::create_task(
                            &state.pool,
                            token.tenant_id.as_deref(),
                            LlmTaskKind::Video,
                            user_id,
                            token_id,
                            channel.id,
                            key_index,
                            &upstream_id,
                            &model,
                            charge.pre_consumed,
                            charge.unlimited,
                            &payload,
                            expires_at,
                        )
                        .await
                        {
                            Ok(t) => t,
                            Err(err) => {
                                billing::refund_all(&state.pool, &charge).await;
                                return openai_error(
                                    StatusCode::INTERNAL_SERVER_ERROR,
                                    "api_error",
                                    format!("persist task: {err}"),
                                );
                            }
                        };
                        router.report_success(channel.id, key_index);
                        return (StatusCode::OK, Json(task_video_json(&created))).into_response();
                    }
                    Err(err) => {
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

/// Load a task scoped to the calling token (404-shaped mismatches).
async fn own_task(
    state: &AppState,
    token: &crate::llm::models::token::LlmToken,
    id_str: &str,
) -> Result<LlmTask, Response> {
    let task_id = parse_id(id_str).map_err(|_| {
        openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "invalid task id".into(),
        )
    })?;
    let row = task::find_by_id(&state.pool, task_id, token.tenant_id.as_deref())
        .await
        .unwrap_or(None)
        .ok_or_else(|| {
            openai_error(
                StatusCode::NOT_FOUND,
                "invalid_request_error",
                format!("no such video task: {id_str}"),
            )
        })?;
    if row.token_id != Some(token.id) {
        return Err(openai_error(
            StatusCode::NOT_FOUND,
            "invalid_request_error",
            format!("no such video task: {id_str}"),
        ));
    }
    Ok(row)
}

/// GET /v1/videos/{id} — poll-on-read: lazily polls the upstream, settles
/// on completion, refunds on failure, expires overdue tasks.
pub(crate) async fn get_video(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let Some(bearer) = bearer_of(&headers) else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "missing bearer token".into(),
        );
    };
    let Ok(identity) =
        super::auth::authenticate(&state.pool, &bearer, &client_ip_of(&headers)).await
    else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "invalid token".into(),
        );
    };
    let token = identity.token;
    let mut row = match own_task(&state, &token, &id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    if !row.status.is_terminal() {
        row = match poll_task(&state, &row).await {
            Ok(r) => r,
            Err(err) => {
                // Transient upstream error — report the last known state.
                tracing::warn!(%err, "video task poll failed");
                row
            }
        };
    }

    (StatusCode::OK, Json(task_video_json(&row))).into_response()
}

/// GET /v1/videos/{id}/content — ensure completed, then proxy the MP4
/// bytes from upstream.
pub(crate) async fn get_video_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let Some(bearer) = bearer_of(&headers) else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "missing bearer token".into(),
        );
    };
    let Ok(identity) =
        super::auth::authenticate(&state.pool, &bearer, &client_ip_of(&headers)).await
    else {
        return openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "invalid token".into(),
        );
    };
    let token = identity.token;
    let mut row = match own_task(&state, &token, &id).await {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    if !row.status.is_terminal() {
        row = match poll_task(&state, &row).await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "video task poll failed");
                row
            }
        };
    }
    if row.status != LlmTaskStatus::Completed {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            format!("video not ready: {}", row.status.as_str()),
        );
    }
    let Some(upstream_id) = row.upstream_task_id.clone() else {
        return openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_error",
            "task missing upstream id".into(),
        );
    };
    let Some(channel) = channel_of(&state, row.channel_id) else {
        return openai_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_error",
            "task channel vanished".into(),
        );
    };
    let key = key_of(&channel, row.key_index);
    let url = format!(
        "{}/videos/{upstream_id}/content",
        channel.base_url.trim_end_matches('/')
    );
    let client = shared_client();
    match client
        .get(&url)
        .headers(crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
            &key,
            channel.header_override.as_ref(),
        ))
        .send()
        .await
    {
        Ok(upstream) if upstream.status().is_success() => {
            let content_type = upstream
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("video/mp4")
                .to_owned();
            let stream = upstream.bytes_stream();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, content_type)
                .body(axum::body::Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Ok(upstream) => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            format!(
                "upstream {}: {}",
                upstream.status().as_u16(),
                upstream.text().await.unwrap_or_default()
            ),
        ),
        Err(err) => openai_error(
            StatusCode::BAD_GATEWAY,
            "api_error",
            format!("transport: {err}"),
        ),
    }
}

/// Cached channel view for a bound task (poll/content paths).
struct ServedChannel {
    base_url: String,
    header_override: Option<serde_json::Value>,
    keys: Vec<crate::llm::cache::CachedKey>,
    cost_mode: crate::llm::models::channel::LlmCostMode,
    cost_discount: f64,
}

fn channel_of(state: &AppState, channel_id: Option<SnowflakeId>) -> Option<ServedChannel> {
    let id = channel_id?;
    let cache = state.llm_router.cache_snapshot();
    let ch = cache.channels.get(&id)?;
    Some(ServedChannel {
        base_url: ch.base_url.clone(),
        header_override: ch.header_override.clone(),
        keys: ch.keys.clone(),
        cost_mode: ch.cost_mode,
        cost_discount: ch.cost_discount,
    })
}

fn key_of(channel: &ServedChannel, key_index: Option<i32>) -> String {
    channel
        .keys
        .get(
            key_index
                .map(|i| i as usize)
                .unwrap_or(0)
                .min(channel.keys.len().saturating_sub(1)),
        )
        .and_then(|k| k.plain.clone())
        .unwrap_or_default()
}

/// Poll one non-terminal task against its bound channel and apply the
/// lifecycle transition (settle / refund / progress). Returns the fresh
/// row (or the unchanged one on non-terminal outcomes).
async fn poll_task(state: &AppState, row: &LlmTask) -> AppResult<LlmTask> {
    // Expiry first: abandoned tasks refund and terminate.
    if row.expires_at.is_some_and(|e| e <= now_utc()) || row.upstream_task_id.is_none() {
        return expire_task(&state.pool, row).await;
    }
    let Some(channel) = channel_of(state, row.channel_id) else {
        return expire_task(&state.pool, row).await;
    };
    let upstream_id = row.upstream_task_id.clone().unwrap_or_default();
    let key = key_of(&channel, row.key_index);
    let url = format!(
        "{}/videos/{upstream_id}",
        channel.base_url.trim_end_matches('/')
    );
    let client = shared_client();
    let resp = client
        .get(&url)
        .headers(crate::llm::relay::adaptor::OpenaiAdaptor::setup_headers(
            &key,
            channel.header_override.as_ref(),
        ))
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    if !resp.status().is_success() {
        // Transient upstream error — keep the task pending (next poll or
        // the expiry sweep decides).
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "poll upstream {status}: {text}"
        )));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))?;
    let upstream_status = body.get("status").and_then(|s| s.as_str()).unwrap_or("");
    let tenant_id = row.tenant_id.as_deref();

    match upstream_status {
        "completed" => {
            let (quota, cost_quota) = settle_amounts(state, row, &channel);
            // Move the pre-charge hold to used on the token ledger (§9.3).
            if let Some(tid) = row.token_id {
                let charge = PreCharge {
                    token_id: tid,
                    pre_consumed: row.pre_consumed,
                    unlimited: row.unlimited_quota,
                };
                billing::settle(&state.pool, &charge, quota).await;
            }
            task::finish_task(
                &state.pool,
                tenant_id,
                row.id,
                task::TaskOutcome {
                    status: LlmTaskStatus::Completed,
                    quota,
                    cost_quota,
                    result: Some(body),
                    error: None,
                },
            )
            .await?;
            write_task_log(&state.pool, row, quota, cost_quota, Some(200), None).await;
            if let (Some(ch), Some(ki)) = (row.channel_id, row.key_index) {
                state.llm_router.report_success(ch, ki as usize);
            }
        }
        "failed" => {
            let err_msg = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("upstream video generation failed")
                .to_owned();
            task::finish_task(
                &state.pool,
                tenant_id,
                row.id,
                task::TaskOutcome {
                    status: LlmTaskStatus::Failed,
                    quota: Quota(0),
                    cost_quota: Quota(0),
                    result: Some(body),
                    error: Some(err_msg.to_owned()),
                },
            )
            .await?;
            refund_task(&state.pool, row).await;
            write_task_log(&state.pool, row, Quota(0), Quota(0), None, Some(&err_msg)).await;
        }
        _ => {
            // queued / in_progress — persist progress for observability.
            let progress = body
                .get("progress")
                .and_then(|p| p.as_i64())
                .unwrap_or(i64::from(row.progress)) as i32;
            let status = if upstream_status == "in_progress" {
                LlmTaskStatus::InProgress
            } else {
                row.status
            };
            let _ = task::update_progress(&state.pool, tenant_id, row.id, status, progress).await;
        }
    }
    Ok(task::find_by_id(&state.pool, row.id, tenant_id)
        .await?
        .unwrap_or_else(|| row.clone()))
}

/// Settle amounts for a completed video task: usage = (1, seconds) —
/// mirrors the images per-image convention (pricing §3.1).
fn settle_amounts(state: &AppState, row: &LlmTask, channel: &ServedChannel) -> (Quota, Quota) {
    let tenant = row
        .tenant_id
        .clone()
        .unwrap_or_else(|| "default".to_owned());
    let Some(info) = state.llm_router.model_info(&tenant, &row.model_name) else {
        return (Quota(0), Quota(0));
    };
    let usage = RelayUsage {
        prompt_tokens: 1,
        completion_tokens: payload_seconds(row),
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    };
    let group_ratio = row
        .payload
        .as_ref()
        .and_then(|p| p.get("group_ratio"))
        .and_then(|r| r.as_f64())
        .unwrap_or(1.0);
    let quota = billing::settle_quota(&info.pricing, &usage, group_ratio);
    let cost = billing::cost_quota(
        &info.pricing,
        &usage,
        channel.cost_mode,
        channel.cost_discount,
    );
    (quota, cost)
}

fn payload_seconds(row: &LlmTask) -> i64 {
    row.payload
        .as_ref()
        .and_then(|p| p.get("seconds"))
        .and_then(|s| {
            s.as_i64()
                .or_else(|| s.as_str().and_then(|v| v.parse().ok()))
        })
        .filter(|v| *v > 0)
        .unwrap_or(12)
}

/// Refund a task's pre-charge (failed/expired paths).
async fn refund_task(pool: &crate::db::Pool, row: &LlmTask) {
    let Some(token_id) = row.token_id else {
        return;
    };
    let charge = PreCharge {
        token_id,
        pre_consumed: row.pre_consumed,
        unlimited: row.unlimited_quota,
    };
    billing::refund_all(pool, &charge).await;
}

/// Expire an overdue task: refund + terminal state + error log.
async fn expire_task(pool: &crate::db::Pool, row: &LlmTask) -> AppResult<LlmTask> {
    let tenant_id = row.tenant_id.as_deref();
    task::finish_task(
        pool,
        tenant_id,
        row.id,
        task::TaskOutcome {
            status: LlmTaskStatus::Expired,
            quota: Quota(0),
            cost_quota: Quota(0),
            result: None,
            error: Some("task expired".to_owned()),
        },
    )
    .await?;
    refund_task(pool, row).await;
    write_task_log(
        pool,
        row,
        Quota(0),
        Quota(0),
        None,
        Some("video task expired"),
    )
    .await;
    Ok(task::find_by_id(pool, row.id, tenant_id)
        .await?
        .unwrap_or_else(|| row.clone()))
}

/// One relay log row per terminal transition (feeds the existing stats).
async fn write_task_log(
    pool: &crate::db::Pool,
    row: &LlmTask,
    quota: Quota,
    cost_quota: Quota,
    status_code: Option<i32>,
    error: Option<&str>,
) {
    let log = NewLog {
        tenant_id: row.tenant_id.clone(),
        user_id: row.user_id,
        token_id: row.token_id,
        source: LogSource::Relay,
        channel_id: row.channel_id,
        key_index: row.key_index,
        model_name: row.model_name.clone(),
        is_stream: false,
        prompt_tokens: 1,
        completion_tokens: payload_seconds(row) as i32,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        quota,
        cost_quota,
        detail: Some(serde_json::json!({
            "kind": row.kind.as_str(),
            "task_id": row.id.0,
            "pre_consumed": row.pre_consumed,
            "async": true,
        })),
        elapsed_ms: None,
        status_code,
        error_message: error.map(str::to_owned),
        request_id: None,
        day: None,
    };
    if let Err(err) = crate::llm::models::log::insert_log(pool, log).await {
        tracing::warn!(%err, "video task log insert failed");
    }
}

/// OpenAI video-object JSON for a task row.
fn task_video_json(row: &LlmTask) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "object": "video",
        "status": row.status.as_str(),
        "progress": row.progress,
        "model": row.model_name,
        "seconds": row.payload.as_ref()
            .and_then(|p| p.get("seconds")).cloned()
            .unwrap_or_else(|| serde_json::Value::String("12".to_owned())),
        "error": row.error_message.as_ref().map(|m| serde_json::json!({
            "message": m, "type": "api_error", "code": null
        })),
        "created_at": row.created_at.to_rfc3339(),
        "result": row.result,
    })
}

/// Cron entrypoint: expire overdue non-terminal tasks (refund + log).
/// Called by the `llm_task_sweep` cron handler; safe to run anytime.
pub async fn sweep_expired_tasks(pool: &crate::db::Pool) -> usize {
    let expired = match task::find_expired(pool, now_utc()).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(%err, "llm task sweep query failed");
            return 0;
        }
    };
    let mut count = 0usize;
    for row in expired {
        if let Err(err) = expire_task(pool, &row).await {
            tracing::warn!(%err, task = %row.id, "llm task expire failed");
        } else {
            count += 1;
        }
    }
    count
}
