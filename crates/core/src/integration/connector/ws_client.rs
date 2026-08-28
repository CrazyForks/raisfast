//! `ws-client` connector — outbound WebSocket for stream channels
//! (integration.md §5.2/§5.3). Supports `raw` framing (each text frame is a
//! JSON body) and `json-rpc` framing (subscribe on connect, route
//! notifications, ack-by-reply on the same connection — Slack Socket Mode).
//!
//! `stream_config` shape:
//! ```json
//! {
//!   "heartbeat_secs": 30,
//!   "subscribe": [ {"method": "connections.open", "params": {}} ],
//!   "notification_method": "events",
//!   "payload_path": "$.payload",
//!   "reply_id_path": "$.envelope_id"
//! }
//! ```
//! Credentials `{"token": "..."}` → `Authorization: Bearer <token>`.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::supervisor::StreamConnector;
use crate::integration::framing::jsonrpc;
use crate::integration::supervisor::ConnectionSink;

pub struct WsClientConnector;

#[async_trait::async_trait]
impl StreamConnector for WsClientConnector {
    async fn run(&self, ch: Arc<ItgChannel>, sink: ConnectionSink) -> anyhow::Result<()> {
        run(ch, sink).await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn walk_path<'a>(root: &'a Value, path: &str) -> &'a Value {
    let mut cur = root;
    for key in path.strip_prefix("$.").unwrap_or(path).split('.') {
        if key.is_empty() {
            continue;
        }
        cur = cur.get(key).unwrap_or(&Value::Null);
    }
    cur
}

async fn run(ch: Arc<ItgChannel>, sink: ConnectionSink) -> AppResult<()> {
    let endpoint = ch
        .endpoint
        .as_deref()
        .filter(|s| s.starts_with("ws://") || s.starts_with("wss://"))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "ws stream requires ws:// or wss:// endpoint — got {:?}",
                ch.endpoint
            ))
        })?;

    let cfg = ch.stream_config.clone().unwrap_or(Value::Null);
    let heartbeat = cfg
        .get("heartbeat_secs")
        .and_then(Value::as_u64)
        .unwrap_or(30);
    let notif_filter = cfg
        .get("notification_method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let payload_path = cfg
        .get("payload_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let reply_id_path = cfg
        .get("reply_id_path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let subscribes: Vec<(String, Value)> = cfg
        .get("subscribe")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some((
                        s.get("method")?.as_str()?.to_string(),
                        s.get("params").cloned().unwrap_or(json_null()),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = endpoint
        .into_client_request()
        .map_err(|e| AppError::BadRequest(format!("invalid ws endpoint: {e}")))?;
    if let Some(token) = auth_token(&ch)? {
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| AppError::BadRequest("invalid bearer header".into()))?,
        );
    }

    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("ws connect: {e}")))?;
    tracing::info!(channel = %ch.channel_key, endpoint, "ws connected");

    let (mut write, mut read) = ws.split();

    // Subscribe handshake (§5.3): responses are logged, fire-and-forget.
    for (i, (method, params)) in subscribes.iter().enumerate() {
        let frame = jsonrpc::request(i as i64 + 1, method, params.clone());
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("ws subscribe send: {e}")))?;
    }

    let mut ping_tick = tokio::time::interval(Duration::from_secs(heartbeat.max(1)));
    ping_tick.tick().await; // first tick fires immediately — skip it

    loop {
        tokio::select! {
            _ = ping_tick.tick() => {
                write
                    .send(tokio_tungstenite::tungstenite::Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("ws ping: {e}")))?;
            }
            frame = read.next() => {
                let Some(msg) = frame else {
                    return Err(AppError::Internal(anyhow::anyhow!("ws closed by peer")));
                };
                let msg = msg.map_err(|e| AppError::Internal(anyhow::anyhow!("ws read: {e}")))?;
                match msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        handle_text(&ch, &sink, &text, notif_filter, payload_path, reply_id_path, &mut write).await?;
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(p) => {
                        write.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await
                            .map_err(|e| AppError::Internal(anyhow::anyhow!("ws pong: {e}")))?;
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                        return Err(AppError::Internal(anyhow::anyhow!("ws close frame")));
                    }
                    _ => {}
                }
            }
        }
    }
}

fn json_null() -> Value {
    Value::Null
}

fn auth_token(ch: &ItgChannel) -> AppResult<Option<String>> {
    let Some(sealed) = ch.credentials.as_deref() else {
        return Ok(None);
    };
    let Some(vault) = crate::integration::shared_plane().and_then(|p| p.vault().cloned()) else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "ws credentials present but vault sealed"
        )));
    };
    let json = vault.unseal(sealed)?;
    Ok(json
        .parse::<Value>()
        .ok()
        .and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_string)))
}

#[allow(clippy::too_many_arguments)]
async fn handle_text(
    ch: &ItgChannel,
    sink: &ConnectionSink,
    text: &str,
    notif_filter: &str,
    payload_path: &str,
    reply_id_path: &str,
    write: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
) -> AppResult<()> {
    match ch.framing.as_str() {
        "raw" => {
            let outcome = sink.submit(ch, text.as_bytes().to_vec()).await;
            let _ = outcome;
            Ok(())
        }
        "json-rpc" => {
            let Some(frame) = jsonrpc::parse(text) else {
                tracing::debug!(channel = %ch.channel_key, "non-jsonrpc frame ignored");
                return Ok(());
            };
            if frame.is_response() {
                tracing::debug!(
                    channel = %ch.channel_key,
                    id = ?frame.id,
                    "subscribe response received"
                );
                return Ok(());
            }
            if !frame.is_notification() {
                return Ok(());
            }
            if !notif_filter.is_empty()
                && frame.method.as_deref() != Some(notif_filter)
            {
                tracing::debug!(
                    channel = %ch.channel_key,
                    method = ?frame.method,
                    "notification filtered out"
                );
                return Ok(());
            }
            let payload = if payload_path.is_empty() {
                frame.payload.clone()
            } else {
                walk_path(&frame.payload, payload_path).clone()
            };
            let body = serde_json::to_vec(&payload).unwrap_or_default();
            let outcome = sink.submit(ch, body).await;
            // Ack-by-reply on the same connection (§5.3): the wire id comes
            // from the frame (e.g. Slack's envelope_id), not the receipt.
            if ch.ack_kind == "rpc-reply" && !reply_id_path.is_empty() {
                let reply_id = walk_path(&frame.payload, reply_id_path).clone();
                if !reply_id.is_null() {
                    let reply = jsonrpc::response(reply_id, serde_json::json!({}));
                    write
                        .send(tokio_tungstenite::tungstenite::Message::Text(reply.into()))
                        .await
                        .map_err(|e| {
                            AppError::Internal(anyhow::anyhow!("ws ack send: {e}"))
                        })?;
                }
            }
            let _ = outcome;
            Ok(())
        }
        other => Err(AppError::BadRequest(format!(
            "ws connector supports framing raw|json-rpc — got '{other}'"
        ))),
    }
}
