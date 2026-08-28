//! `ws-client` connector — outbound WebSocket for stream channels
//! (integration.md §5.2/§5.3). Three framings:
//!
//! - `raw` — each text frame is a JSON body.
//! - `json-rpc` — subscribe on connect, route notifications, ack-by-reply on
//!   the same connection (Slack Socket Mode).
//! - `dispatch` — **declarative protocol profile**: any JSON-over-WS protocol
//!   whose frames carry a discriminator field (`command`/`type`/`action`…)
//!   is expressed as config, no Rust changes. DingTalk push, WeCom and
//!   friends are all instances of this shape.
//! - `pb-frame` — protobuf-enveloped binary frames (pbbp2 wire shape) with
//!   config-driven semantics: client heartbeat, fragment reassembly,
//!   event dispatch and ack replies. Feishu long-connection is the
//!   reference deployment (`dev-docs/integration/feishu-ws.md`).
//!
//! Generic pre-connect: `stream_config.pre_connect` exchanges the static
//! endpoint for a dynamic one over HTTP (gateways that hand out
//! per-connection URLs with tokens in the query).
//!
//! `stream_config` for `dispatch`:
//! ```json
//! {
//!   "handshake": {
//!     "frames": [ {"command":"connect","headers":{"Authorization":"Bearer {{token}}",
//!                   "app_id":"{{app_id}}"}, "service_id": 1} ],
//!     "ack": {"path": "$.command", "equals": "conn_ack", "code_path": "$.code"}
//!   },
//!   "reply_heartbeat": {"match": {"path": "$.command", "equals": "ping"},
//!                       "reply": {"command": "pong"}},
//!   "events": {"match": {"path": "$.command", "equals": "event"},
//!              "payload_path": "$"},
//!   "ack_reply": {"command": "ok", "id_path": "$.headers.event_id"}
//! }
//! ```
//! Template vars come from the credentials: `{{token}}` (static, or a
//! cached OAuth client-credentials token when `kind == "oauth-cc"`) plus
//! every `grant.*` field. Credentials `{"token": "..."}` on other framings
//! still map to the HTTP `Authorization: Bearer` header.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use prost::Message as _;
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::connector::pb_frame::{self, PbFrame, PbFrameProfile};
use crate::integration::framing::jsonrpc;
use crate::integration::supervisor::ConnectionSink;
use crate::integration::supervisor::StreamConnector;

pub struct WsClientConnector;

/// `{path, equals}` frame matcher — the dispatch framing's discriminator.
#[derive(Debug, Clone)]
struct Matcher {
    path: String,
    equals: String,
}

impl Matcher {
    fn parse(v: &Value) -> Option<Self> {
        Some(Self {
            path: v.get("path")?.as_str()?.to_string(),
            equals: v.get("equals")?.as_str()?.to_string(),
        })
    }

    fn matches(&self, frame: &Value) -> bool {
        walk_path(frame, &self.path).as_str() == Some(self.equals.as_str())
    }
}

/// Declarative protocol profile for the `dispatch` framing.
struct DispatchProfile {
    /// Templates rendered with credential vars and sent right after connect.
    handshake_frames: Vec<String>,
    /// Optional handshake ack: match + success-code check.
    ack: Option<(Matcher, Option<String>)>,
    /// Server-initiated heartbeat: match frame, reply with template.
    reply_heartbeat: Option<(Matcher, String)>,
    /// Event frames: matcher + payload path submitted to the pipeline.
    events: Option<(Matcher, String)>,
    /// Optional per-event in-connection ack (rendered with `{{id}}`).
    ack_reply: Option<String>,
    ack_reply_id_path: String,
}

impl DispatchProfile {
    fn parse(cfg: &Value) -> Option<Self> {
        let events = cfg.get("events").and_then(|e| {
            Some((
                Matcher::parse(e.get("match")?)?,
                e.get("payload_path")
                    .and_then(Value::as_str)
                    .unwrap_or("$")
                    .to_string(),
            ))
        });
        // A profile without event dispatch is a misconfiguration.
        events.as_ref()?;
        let ack = cfg.pointer("/handshake/ack").and_then(|a| {
            Some((
                Matcher::parse(a.get("match")?)?,
                a.get("code_path")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ))
        });
        Some(Self {
            handshake_frames: cfg
                .pointer("/handshake/frames")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|f| f.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            ack,
            reply_heartbeat: cfg.get("reply_heartbeat").and_then(|h| {
                Some((
                    Matcher::parse(h.get("match")?)?,
                    serde_json::to_string(h.get("reply")?).ok()?,
                ))
            }),
            events,
            ack_reply: cfg
                .get("ack_reply")
                .and_then(|a| serde_json::to_string(a).ok()),
            // Accept the id path at the top level (alongside ack_reply) or
            // nested inside the ack_reply object; default the frame's `id`.
            ack_reply_id_path: cfg
                .get("ack_reply_id_path")
                .and_then(Value::as_str)
                .or_else(|| cfg.pointer("/ack_reply/id_path").and_then(Value::as_str))
                .unwrap_or("$.id")
                .to_string(),
        })
    }
}

#[async_trait::async_trait]
impl StreamConnector for WsClientConnector {
    async fn run(&self, ch: Arc<ItgChannel>, sink: ConnectionSink) -> anyhow::Result<()> {
        run(ch, sink).await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn walk_path<'a>(root: &'a Value, path: &str) -> &'a Value {
    // "$" (or "") → the whole frame; "$.a.b" → nested walk.
    if path == "$" || path.is_empty() {
        return root;
    }
    let mut cur = root;
    for key in path.strip_prefix("$.").unwrap_or(path).split('.') {
        if key.is_empty() {
            continue;
        }
        cur = cur.get(key).unwrap_or(&Value::Null);
    }
    cur
}

/// `pre_connect` exchange: POST a template body, gate on the returned code,
/// extract the dynamic WS URL. Generic for any gateway that hands out
/// per-connection endpoints (Feishu `/callback/ws/endpoint` family).
async fn pre_connect_url(pc: &Value, vars: &serde_json::Map<String, Value>) -> AppResult<String> {
    let var_keys: Vec<&str> = vars.keys().map(String::as_str).collect();
    let url = pc
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::BadRequest("pre_connect requires url".into()))?;
    let body = pc
        .get("body")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let body_str = crate::integration::token::render_template(&body.to_string(), vars);
    if body_str.contains("{{") {
        return Err(AppError::BadRequest(format!(
            "pre_connect body has unresolved template vars: {body_str} \
             (available vars: {var_keys:?})"
        )));
    }
    let body: Value = serde_json::from_str(&body_str)
        .map_err(|e| AppError::BadRequest(format!("pre_connect body render: {e}")))?;
    let fp = body
        .get("AppID")
        .and_then(|v| v.as_str())
        .map(|v| format!("{}(len {})", &v[..v.len().min(6)], v.len()))
        .unwrap_or_else(|| "<none>".into());
    tracing::debug!(app_id_fingerprint = %fp, "pre_connect body rendered");

    // Native TLS stack: gateways fingerprint ClientHellos (rustls' shape
    // gets rejected with 1000040346 while system-OpenSSL stacks pass).
    let mut req = reqwest::Client::builder()
        .use_native_tls()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pre_connect client: {e}")))?
        .post(url)
        .json(&body);
    // Gateways may gate on headers (User-Agent allow-lists, locale, …).
    if let Some(headers) = pc.get("headers").and_then(Value::as_object) {
        for (k, v) in headers {
            let value =
                crate::integration::token::render_template(v.as_str().unwrap_or_default(), vars);
            req = req.header(k, value);
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pre_connect request: {e}")))?;
    let status = resp.status();
    let payload: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("pre_connect body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "pre_connect endpoint returned {status}: {payload}"
        )));
    }
    if let (Some(code_path), Some(ok_code)) = (
        pc.get("code_path").and_then(Value::as_str),
        pc.get("ok_code").and_then(Value::as_i64),
    ) {
        let code = walk_path(&payload, code_path).as_i64().unwrap_or(-1);
        if code != ok_code {
            return Err(AppError::Internal(anyhow::anyhow!(
                "pre_connect rejected (code {code}, sent AppID {:?}): {payload}",
                body.get("AppID")
                    .and_then(Value::as_str)
                    .map(|v| &v[..v.len().min(8)])
            )));
        }
    }
    // Compose the WS url: either a single extracted path (url_path) or a
    // template over multiple response fields (url_template), e.g.
    // `"{endpoint}?ticket={ticket}"` — gateways that hand out the endpoint
    // and an admission ticket separately.
    let dynamic = match pc.get("url_template").and_then(Value::as_str) {
        Some(template) => {
            let mut vars = serde_json::Map::new();
            // Response fields first (endpoint/ticket/…), then credential vars
            // so a static fallback can be referenced too.
            if let Some(obj) = payload.as_object() {
                for (k, v) in obj {
                    if let Value::String(sv) = v {
                        vars.insert(k.clone(), Value::String(sv.clone()));
                    }
                }
            }
            let rendered = crate::integration::token::render_template(template, &vars);
            if rendered.contains('{') && rendered.contains('}') {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "pre_connect url_template has unresolved vars: {rendered} (response: {payload})"
                )));
            }
            rendered
        }
        None => {
            let url_path = pc
                .get("url_path")
                .and_then(Value::as_str)
                .unwrap_or("$.data.URL");
            walk_path(&payload, url_path)
                .as_str()
                .ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!(
                        "pre_connect: no WS url at {url_path}: {payload}"
                    ))
                })?
                .to_string()
        }
    };
    if !(dynamic.starts_with("ws://") || dynamic.starts_with("wss://")) {
        return Err(AppError::Internal(anyhow::anyhow!(
            "pre_connect returned a non-ws url: {dynamic}"
        )));
    }
    tracing::info!(url = %dynamic, "pre_connect exchanged ws url");
    Ok(dynamic)
}

async fn run(ch: Arc<ItgChannel>, sink: ConnectionSink) -> AppResult<()> {
    let static_endpoint = ch
        .endpoint
        .as_deref()
        .filter(|s| s.starts_with("ws://") || s.starts_with("wss://"))
        .ok_or_else(|| {
            AppError::BadRequest(format!(
                "ws stream requires ws:// or wss:// endpoint — got {:?}",
                ch.endpoint
            ))
        })?
        .to_string();

    let cfg = ch.stream_config.clone().unwrap_or(Value::Null);
    let creds = unseal_credentials(&sink, &ch)?;
    let vars = {
        let token = if let Some(c) = &creds {
            if crate::integration::token::is_oauth_cc(c) {
                Some(
                    crate::integration::token::resolve_token(
                        &format!("channel:{}", ch.channel_key),
                        c,
                    )
                    .await?,
                )
            } else {
                auth_token_static(c)?
            }
        } else {
            None
        };
        crate::integration::token::template_vars(token, &creds.clone().unwrap_or(Value::Null))
    };

    // Generic pre-connect: exchange the static endpoint for a dynamic
    // per-connection URL over HTTP (template body, code gate, URL extract).
    let endpoint = match cfg.get("pre_connect") {
        Some(pc) => pre_connect_url(pc, &vars).await?,
        None => static_endpoint,
    };
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

    // Dispatch profile (declarative framing) — generic for any
    // discriminator-field JSON protocol.
    let profile = if ch.framing == "dispatch" {
        Some(DispatchProfile::parse(&cfg).ok_or_else(|| {
            AppError::BadRequest(
                "dispatch framing requires stream_config.events {match:{path,equals}, payload_path}"
                    .into(),
            )
        })?)
    } else {
        None
    };
    // pb-frame profile (protobuf envelope, config-driven semantics).
    let pb_profile = (ch.framing == "pb-frame")
        .then(|| PbFrameProfile::parse(cfg.get("pb_frame").unwrap_or(&Value::Null)));
    // The envelope heartbeat needs the service id from the (possibly
    // exchanged) endpoint query.
    let pb_service_id = pb_profile
        .as_ref()
        .and_then(|_| {
            endpoint
                .split('?')
                .nth(1)
                .and_then(|q| {
                    q.split('&').find_map(|kv| {
                        let (k, v) = kv.split_once('=')?;
                        (k == "service_id").then(|| v.to_string())
                    })
                })
                .and_then(|v| v.parse::<i32>().ok())
        })
        .unwrap_or(0);

    // Connection credential: static token or OAuth client-credentials.
    let creds = unseal_credentials(&sink, &ch)?;
    let token = if let Some(c) = &creds {
        if crate::integration::token::is_oauth_cc(c) {
            Some(
                crate::integration::token::resolve_token(&format!("channel:{}", ch.channel_key), c)
                    .await?,
            )
        } else {
            auth_token_static(c)?
        }
    } else {
        None
    };

    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = endpoint
        .as_str()
        .into_client_request()
        .map_err(|e| AppError::BadRequest(format!("invalid ws endpoint: {e}")))?;
    if profile.is_none()
        && let Some(creds) = &creds
        && let Some(static_token) = auth_token_static(creds)?
    {
        request.headers_mut().insert(
            "authorization",
            format!("Bearer {static_token}")
                .parse()
                .map_err(|_| AppError::BadRequest("invalid bearer header".into()))?,
        );
    }

    // Bound the whole establishment phase (DNS + TCP + TLS + upgrade):
    // gateways may silently park unauthorized upgrades — without a timeout
    // the supervisor waits forever in `connecting`.
    let (ws, _resp) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .map_err(|_| AppError::Internal(anyhow::anyhow!("ws connect timeout (15s)")))?
    .map_err(|e| AppError::Internal(anyhow::anyhow!("ws connect: {e}")))?;
    tracing::info!(channel = %ch.channel_key, endpoint, "ws connected");

    let (mut write, mut read) = ws.split();

    if pb_profile.is_some() {
        // pb-frame has no app-level handshake to await — the established
        // WS link plus the heartbeat loop is the stable state.
        sink.mark_connected(&ch).await;
    }

    if let Some(profile) = &profile {
        // Handshake templates: rendered with token + grant vars.
        let vars = crate::integration::token::template_vars(
            token.clone(),
            &creds.clone().unwrap_or(Value::Null),
        );
        for tpl in &profile.handshake_frames {
            let frame = crate::integration::token::render_template(tpl, &vars);
            write
                .send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("ws handshake send: {e}")))?;
        }
        // Await the handshake ack when declared (fail → supervisor backoff).
        if let Some((matcher, code_path)) = &profile.ack {
            let deadline = Duration::from_secs(10);
            let ack = tokio::time::timeout(deadline, async {
                while let Some(Ok(msg)) = read.next().await {
                    if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                        let Ok(v) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        if matcher.matches(&v) {
                            return Some(v);
                        }
                        // Non-ack frames during handshake: park for the loop?
                        // Gateways do not send events before ack — drop them.
                    }
                }
                None
            })
            .await
            .map_err(|_| AppError::Internal(anyhow::anyhow!("ws handshake ack timeout")))?
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("ws closed before handshake ack")))?;
            if let Some(code_path) = code_path {
                let code = walk_path(&ack, code_path);
                let ok = code.as_i64().unwrap_or(-1) == 0 || code.as_bool() == Some(true);
                if !ok {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "ws handshake rejected: {ack}"
                    )));
                }
            }
            tracing::info!(channel = %ch.channel_key, "ws handshake acked");
        }
        sink.mark_connected(&ch).await;
    } else {
        // Subscribe handshake (§5.3): responses are logged, fire-and-forget.
        for (i, (method, params)) in subscribes.iter().enumerate() {
            let frame = jsonrpc::request(i as i64 + 1, method, params.clone());
            write
                .send(tokio_tungstenite::tungstenite::Message::Text(frame.into()))
                .await
                .map_err(|e| AppError::Internal(anyhow::anyhow!("ws subscribe send: {e}")))?;
        }
        sink.mark_connected(&ch).await;
    }

    // Keepalive: pb-frame sends application-level CONTROL pings; raw and
    // json-rpc use the WS protocol layer. dispatch protocols declare their
    // own heartbeat — EXCEPT those without one (opt in via ws_keepalive).
    let ws_keepalive = profile.is_none()
        || (profile.is_some()
            && cfg
                .get("ws_keepalive")
                .and_then(Value::as_bool)
                .unwrap_or(false));
    let pb_keepalive = pb_profile.is_some();
    let pb_ping_secs = pb_profile
        .as_ref()
        .map_or(heartbeat, |p| p.ping_interval_secs.max(1));
    let mut ping_tick = tokio::time::interval(Duration::from_secs(if pb_keepalive {
        pb_ping_secs
    } else {
        heartbeat.max(1)
    }));
    ping_tick.tick().await; // first tick fires immediately — skip it
    let mut fragments = pb_frame::FragmentBuffer::default();

    loop {
        tokio::select! {
            _ = ping_tick.tick(), if ws_keepalive || pb_keepalive => {
                let msg = if let Some(p) = &pb_profile {
                    tokio_tungstenite::tungstenite::Message::Binary(
                        PbFrame::ping(pb_service_id, &p.ping_type).encode_to_vec().into(),
                    )
                } else {
                    tokio_tungstenite::tungstenite::Message::Ping(Vec::new().into())
                };
                write
                    .send(msg)
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
                        if let Some(profile) = &profile {
                            handle_dispatch(&ch, &sink, &text, profile, &mut write).await?;
                        } else {
                            handle_text(&ch, &sink, &text, notif_filter, payload_path, reply_id_path, &mut write).await?;
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bin) => {
                        if let Some(p) = &pb_profile {
                            handle_pb_frame(&ch, &sink, &bin, p, &mut fragments, &mut write)
                                .await?;
                        }
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

/// Static token from credentials (no oauth-cc involvement).
fn auth_token_static(creds: &Value) -> AppResult<Option<String>> {
    if crate::integration::token::is_oauth_cc(creds) {
        return Ok(None);
    }
    Ok(creds
        .get("token")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// pb-frame handler: CONTROL (heartbeat) vs DATA (events), fragment
/// reassembly, ack-by-reply on the same connection.
async fn handle_pb_frame(
    ch: &Arc<ItgChannel>,
    sink: &ConnectionSink,
    bytes: &[u8],
    profile: &PbFrameProfile,
    fragments: &mut pb_frame::FragmentBuffer,
    write: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
) -> AppResult<()> {
    let Ok(frame) = PbFrame::decode(bytes) else {
        tracing::debug!(channel = %ch.channel_key, "pb-frame: undecodable binary ignored");
        return Ok(());
    };
    let Some(frame_type) = frame.header(&profile.type_header) else {
        return Ok(());
    };
    if frame.method == 0 {
        // CONTROL: pong (possibly carrying refreshed ClientConfig) — log only.
        if frame_type == profile.pong_type {
            tracing::debug!(channel = %ch.channel_key, "pb-frame: pong");
        }
        return Ok(());
    }
    if frame_type != profile.event_type {
        tracing::debug!(
            channel = %ch.channel_key,
            r#type = frame_type,
            "pb-frame: non-event DATA ignored"
        );
        return Ok(());
    }
    // Fragmented delivery: route only when every seq arrived.
    let msg_id = frame
        .header(&profile.frag_id)
        .unwrap_or_default()
        .to_string();
    let sum = frame
        .header(&profile.frag_sum)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);
    let seq = frame
        .header(&profile.frag_seq)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(1);
    let chunk = frame.payload.clone().unwrap_or_default();
    let Some(complete) = fragments.push(&msg_id, sum, seq, chunk) else {
        return Ok(());
    };
    let outcome = sink.submit(ch, complete).await;
    if profile.ack {
        let reply = pb_frame::ack_frame(&frame, profile.ack_code);
        write
            .send(tokio_tungstenite::tungstenite::Message::Binary(
                reply.encode_to_vec().into(),
            ))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("pb-frame ack send: {e}")))?;
    }
    let _ = outcome;
    Ok(())
}

/// Dispatch-framing text handler: reverse heartbeat → ack reply → events.
async fn handle_dispatch(
    ch: &Arc<ItgChannel>,
    sink: &ConnectionSink,
    text: &str,
    profile: &DispatchProfile,
    write: &mut futures::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::Message,
    >,
) -> AppResult<()> {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        tracing::debug!(channel = %ch.channel_key, "dispatch: non-JSON frame ignored");
        return Ok(());
    };
    // Reverse heartbeat: the server pings, we answer on the application layer.
    if let Some((matcher, reply_tpl)) = &profile.reply_heartbeat
        && matcher.matches(&frame)
    {
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                reply_tpl.clone().into(),
            ))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("ws heartbeat reply: {e}")))?;
        return Ok(());
    }
    let Some((matcher, payload_path)) = &profile.events else {
        return Ok(());
    };
    if !matcher.matches(&frame) {
        tracing::debug!(channel = %ch.channel_key, "dispatch: unmatched frame ignored");
        return Ok(());
    }
    let payload = walk_path(&frame, payload_path).clone();
    let body = serde_json::to_vec(&payload).unwrap_or_default();
    let outcome = sink.submit(ch, body).await;
    // Optional in-connection event ack (rendered with the frame's id).
    if let Some(tpl) = &profile.ack_reply {
        let mut vars = serde_json::Map::new();
        if let Some(id) = walk_path(&frame, &profile.ack_reply_id_path).as_str() {
            vars.insert("id".into(), Value::String(id.to_string()));
        }
        let reply = crate::integration::token::render_template(tpl, &vars);
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(reply.into()))
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("ws event ack send: {e}")))?;
    }
    let _ = outcome;
    Ok(())
}

fn json_null() -> Value {
    Value::Null
}

fn unseal_credentials(sink: &ConnectionSink, ch: &ItgChannel) -> AppResult<Option<Value>> {
    let Some(sealed) = ch.credentials.as_deref() else {
        return Ok(None);
    };
    let Some(vault) = sink.vault() else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "ws credentials present but vault sealed"
        )));
    };
    let json = vault.unseal(sealed)?;
    json.parse::<Value>()
        .map(Some)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("credentials json: {e}")))
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
            if !notif_filter.is_empty() && frame.method.as_deref() != Some(notif_filter) {
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
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("ws ack send: {e}")))?;
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
