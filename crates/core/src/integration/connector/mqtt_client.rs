//! `mqtt-client` connector — MQTT QoS1 stream channel (integration.md §5.2).
//!
//! Sequential poll→pipeline→manual-ack loop = natural backpressure: while a
//! frame is in the pipeline the eventloop is not polled, and unacked QoS1
//! publishes are retained by the broker for redelivery (receipts idempotency
//! absorbs any duplicates).
//!
//! `stream_config` shape:
//! ```json
//! {
//!   "topics": ["sensors/#"],
//!   "client_id": "raisfast-1",
//!   "heartbeat_secs": 30,
//!   "username": "optional-plain",
//!   "payload_as_json": true
//! }
//! ```
//! Credentials `{"token"}` (vault) map to password (username from config).

use std::sync::Arc;
use std::time::Duration;

use rumqttc::{AsyncClient, Event, Incoming, MqttOptions, QoS};
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::channel::ItgChannel;
use crate::integration::supervisor::{ConnectionSink, StreamConnector};

pub struct MqttClientConnector;

#[async_trait::async_trait]
impl StreamConnector for MqttClientConnector {
    async fn run(&self, ch: Arc<ItgChannel>, sink: ConnectionSink) -> anyhow::Result<()> {
        run(ch, sink).await.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

struct Endpoint {
    host: String,
    port: u16,
}

fn parse_endpoint(ch: &ItgChannel) -> AppResult<Endpoint> {
    let raw = ch
        .endpoint
        .as_deref()
        .ok_or_else(|| AppError::BadRequest("mqtt stream requires endpoint".into()))?;
    let stripped = raw
        .strip_prefix("mqtts://")
        .map(|s| (s, true))
        .or_else(|| raw.strip_prefix("mqtt://").map(|s| (s, false)))
        .map(|(s, tls)| (s.to_string(), tls));
    let (hostport, _tls) =
        stripped.ok_or_else(|| AppError::BadRequest("endpoint must be mqtt:// or mqtts://".into()))?;
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h.to_string(),
            p.parse::<u16>().map_err(|_| {
                AppError::BadRequest(format!("invalid mqtt port in '{raw}'"))
            })?,
        ),
        None => (hostport, 1883),
    };
    Ok(Endpoint { host, port })
}

async fn run(ch: Arc<ItgChannel>, sink: ConnectionSink) -> AppResult<()> {
    let endpoint = parse_endpoint(&ch)?;
    let cfg = ch.stream_config.clone().unwrap_or(Value::Null);
    let client_id = cfg
        .get("client_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("raisfast-{}", ch.channel_key));
    let keepalive = cfg
        .get("heartbeat_secs")
        .and_then(Value::as_u64)
        .unwrap_or(30);
    let topics: Vec<String> = cfg
        .get("topics")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if topics.is_empty() {
        return Err(AppError::BadRequest(
            "mqtt stream_config requires at least one topic".into(),
        ));
    }
    let payload_as_json = cfg
        .get("payload_as_json")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let mut opts = MqttOptions::new(client_id, endpoint.host, endpoint.port);
    opts.set_keep_alive(Duration::from_secs(keepalive.max(5)));
    let password = vault_password(&ch)?;
    match (cfg.get("username").and_then(Value::as_str), password) {
        (Some(user), Some(pass)) => {
            opts.set_credentials(user, pass);
        }
        (Some(user), None) => {
            opts.set_credentials(user, "");
        }
        (None, Some(token)) => {
            // Token-only credentials: common broker convention uses token as user.
            let t = token.clone();
            opts.set_credentials(t.clone(), t);
        }
        (None, None) => {}
    }

    // Manual acks: PUBACK only after the pipeline pass completes (§5.3).
    opts.set_manual_acks(true);
    let (client, mut eventloop) = AsyncClient::new(opts, 64);
    for topic in &topics {
        client
            .subscribe(topic, QoS::AtLeastOnce)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("mqtt subscribe {topic}: {e}")))?;
    }
    tracing::info!(channel = %ch.channel_key, topics = ?topics, "mqtt subscribed");

    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Incoming::Publish(publish))) => {
                let payload = if payload_as_json {
                    publish.payload.to_vec()
                } else {
                    // Non-JSON payloads become a JSON wrapper the mapping can read.
                    let text = String::from_utf8_lossy(&publish.payload);
                    serde_json::to_vec(&Value::String(text.into_owned()))
                        .unwrap_or_default()
                };
                let outcome = sink.submit(&ch, payload).await;
                let _ = outcome;
                if let Err(e) = client.ack(&publish).await {
                    tracing::warn!(channel = %ch.channel_key, error = %e, "mqtt manual ack failed");
                }
            }
            Ok(_) => {}
            Err(e) => {
                return Err(AppError::Internal(anyhow::anyhow!("mqtt poll: {e}")));
            }
        }
    }
}

fn vault_password(ch: &ItgChannel) -> AppResult<Option<String>> {
    let Some(sealed) = ch.credentials.as_deref() else {
        return Ok(None);
    };
    let Some(vault) = crate::integration::shared_plane().and_then(|p| p.vault().cloned()) else {
        return Err(AppError::Internal(anyhow::anyhow!(
            "mqtt credentials present but vault sealed"
        )));
    };
    let json = vault.unseal(sealed)?;
    Ok(json
        .parse::<Value>()
        .ok()
        .and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_string)))
}
