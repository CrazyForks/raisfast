//! `http-pull` connector — generic REST cursor pagination (integration.md §5.3).
//!
//! Template (`itg_channels.pull_config`):
//! ```json
//! {
//!   "list_path": "$.data.items",     // path to the items array (simple $.a.b)
//!   "id_field":  "id",               // item field: external_id AND cursor value
//!   "param":     "since_id",         // query param carrying the cursor
//!   "page_size_param": "limit",
//!   "page_size": 50,
//!   "max_pages": 20                  // safety bound per run
//! }
//! ```
//! Auth: credentials `{"token": "..."}` → `Authorization: Bearer <token>`.
//!
//! Each fetched item is synthesized as a `verify_kind=none` POST into the
//! standard pipeline — receipts idempotency, envelope snapshot, retry
//! machinery all apply unchanged. Cursor advances to the last **delivered or
//! duplicate** id; failed items keep their own `ingress.retry` recovery.

use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::connector::PullSummary;
use crate::integration::cursor;
use crate::integration::pipeline::Pipeline;
use crate::integration::verify::InboundHttpRequest;

struct PullTemplate {
    endpoint: String,
    list_path: Vec<String>,
    id_field: String,
    param: String,
    page_size_param: String,
    page_size: u64,
    max_pages: u64,
    token: Option<String>,
}

fn parse_template(channel: &ItgChannel, token: Option<String>) -> AppResult<PullTemplate> {
    let cfg = channel.pull_config.clone().unwrap_or(Value::Null);
    let endpoint = channel
        .endpoint
        .as_deref()
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "pull channel '{}' missing http(s) endpoint",
                channel.channel_key
            ))
        })?
        .to_string();

    let list_path_src = cfg
        .get("list_path")
        .and_then(Value::as_str)
        .unwrap_or("$.data");
    let list_path: Vec<String> = list_path_src
        .strip_prefix("$.")
        .unwrap_or(list_path_src)
        .split('.')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();

    Ok(PullTemplate {
        endpoint,
        list_path,
        id_field: cfg
            .get("id_field")
            .and_then(Value::as_str)
            .unwrap_or("id")
            .to_string(),
        param: cfg
            .get("param")
            .and_then(Value::as_str)
            .unwrap_or("since_id")
            .to_string(),
        page_size_param: cfg
            .get("page_size_param")
            .and_then(Value::as_str)
            .unwrap_or("limit")
            .to_string(),
        page_size: cfg.get("page_size").and_then(Value::as_u64).unwrap_or(50),
        max_pages: cfg.get("max_pages").and_then(Value::as_u64).unwrap_or(20),
        token,
    })
}

fn walk<'a>(root: &'a Value, path: &[String]) -> &'a Value {
    let mut cur = root;
    for key in path {
        cur = cur.get(key).unwrap_or(&Value::Null);
    }
    cur
}

fn item_id(item: &Value, field: &str) -> Option<String> {
    let v = item.get(field)?;
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn id_from_cursor(cursor: &Value) -> Option<&str> {
    cursor.get("since_id").and_then(Value::as_str)
}

/// Execute one pull run for the channel.
///
/// # Errors
///
/// Returns `AppError` on template errors; HTTP/transport failures abort the
/// run early (cursor untouched → full retry next tick).
pub async fn run(
    pool: &crate::db::Pool,
    pipeline: &Pipeline,
    channel: &ItgChannel,
    token: Option<String>,
) -> AppResult<PullSummary> {
    let template = parse_template(channel, token)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pull http client: {e}")))?;

    let mut summary = PullSummary::default();
    let expected_cursor = cursor::read(pool, channel.id).await?;
    let mut current_id = expected_cursor
        .as_ref()
        .and_then(id_from_cursor)
        .map(str::to_string);

    for page in 0..template.max_pages {
        let mut url = reqwest::Url::parse(&template.endpoint)
            .map_err(|e| AppError::BadRequest(format!("invalid endpoint: {e}")))?;
        if let Some(id) = &current_id {
            url.query_pairs_mut().append_pair(&template.param, id);
        }
        url.query_pairs_mut()
            .append_pair(&template.page_size_param, &template.page_size.to_string());

        let mut req = client.get(url.clone());
        if let Some(token) = &template.token {
            req = req.bearer_auth(token);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("pull fetch: {e}")))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("pull body: {e}")))?;
        if !status.is_success() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "pull endpoint returned {status}"
            )));
        }
        let doc: Value = serde_json::from_str(&body)
            .map_err(|e| AppError::BadRequest(format!("pull response not JSON: {e}")))?;

        let items = walk(&doc, &template.list_path).as_array().cloned();
        let items = items.unwrap_or_default();
        summary.pages += 1;
        summary.fetched += items.len() as u64;
        if items.is_empty() {
            break;
        }

        for item in &items {
            let Some(id) = item_id(item, &template.id_field) else {
                tracing::warn!(
                    channel = %channel.channel_key,
                    "pull item without id_field '{}' — skipped",
                    template.id_field
                );
                continue;
            };
            // Synthesize a pipeline request: verify_kind=none (pull trusts the
            // credentials it fetched with), body = the raw item JSON.
            let req = InboundHttpRequest {
                method: "POST".into(),
                query: String::new(),
                headers: Vec::new(),
                body: serde_json::to_vec(item).unwrap_or_default(),
            };
            let outcome = pipeline
                .run_push(&std::sync::Arc::new(clone_channel_shallow(channel)), &req)
                .await;
            if outcome.duplicate {
                summary.duplicates += 1;
            } else if outcome.delivered {
                summary.delivered += 1;
            } else {
                summary.failed += 1;
            }
            current_id = Some(id);
        }

        // Short page = exhausted.
        if (items.len() as u64) < template.page_size {
            break;
        }
        let _ = page;
    }

    // Conditional cursor advance (loser abandons, §3.1.1).
    if let Some(new_id) = current_id {
        let new_cursor = serde_json::json!({"since_id": new_id});
        let won = cursor::advance(
            pool,
            channel.id,
            expected_cursor.as_ref(),
            &new_cursor,
            crate::utils::tz::now_utc(),
        )
        .await?;
        if !won {
            tracing::info!(
                channel = %channel.channel_key,
                "pull cursor advance lost the race — abandoned (next tick re-reads)"
            );
        }
    }

    Ok(summary)
}

/// Shallow clone without Arc wrapper for pipeline calls.
fn clone_channel_shallow(channel: &ItgChannel) -> ItgChannel {
    channel.clone()
}

/// Unseal the pull bearer token from channel credentials (None when absent).
///
/// # Errors
///
/// Returns `AppError` when credentials exist but the vault is sealed.
pub fn pull_token(
    channel: &ItgChannel,
    vault: Option<&crate::integration::vault::Vault>,
) -> AppResult<Option<String>> {
    let Some(sealed) = channel.credentials.as_deref() else {
        return Ok(None);
    };
    let Some(vault) = vault else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "pull channel has credentials but vault is sealed"
        )));
    };
    let json = vault.unseal(sealed)?;
    Ok(json
        .parse::<Value>()
        .ok()
        .and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_string)))
}
