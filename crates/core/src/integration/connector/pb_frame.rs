//! `pb-frame` — protobuf-framed WebSocket streams (integration.md §2 L2).
//!
//! A generic framing for gateways that wrap payloads in a protobuf "envelope"
//! frame (headers + payload + service/method discriminator). The wire shape
//! follows the widely-deployed pbbp2 Frame (Feishu long-connection family);
//! every semantic — which header discriminates events, heartbeat values,
//! fragment reassembly, ack body — is configuration, not code.
//!
//! Envelope (wire tags fixed, semantics config-driven):
//! `Header{key=1, value=2}`, `Frame{seq_id=1, log_id=2, service=3,
//! method=4, headers=5, payload_encoding=6, payload_type=7, payload=8}`.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

/// Envelope header entry (`key`/`value` pairs on the wire).
#[derive(Clone, PartialEq, Eq, prost::Message)]
pub struct PbHeader {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// The envelope frame.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PbFrame {
    #[prost(uint64, tag = "1")]
    pub seq_id: u64,
    #[prost(uint64, tag = "2")]
    pub log_id: u64,
    #[prost(int32, tag = "3")]
    pub service: i32,
    /// 0 = CONTROL (heartbeat), 1 = DATA (payload) — config decides usage.
    #[prost(int32, tag = "4")]
    pub method: i32,
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<PbHeader>,
    #[prost(string, optional, tag = "6")]
    pub payload_encoding: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub payload_type: Option<String>,
    #[prost(bytes = "vec", optional, tag = "8")]
    pub payload: Option<Vec<u8>>,
    #[prost(string, optional, tag = "9")]
    pub log_id_new: Option<String>,
}

impl PbFrame {
    /// First header value for `key`, if present.
    #[must_use]
    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|h| h.key == key)
            .map(|h| h.value.as_str())
    }

    /// Build a CONTROL heartbeat frame (`type` header + service id).
    #[must_use]
    pub fn ping(service: i32, ping_type: &str) -> Self {
        Self {
            service,
            method: 0,
            headers: vec![PbHeader {
                key: "type".into(),
                value: ping_type.to_string(),
            }],
            ..Self::default()
        }
    }
}

/// Fragment reassembly for chunked payloads (`sum`/`seq` headers, §L2).
///
/// Gateways split large events across frames sharing a message id; the
/// complete payload only routes once every seq arrived. Incomplete buffers
/// are bounded (`MAX_TRACKED`) and evicted oldest-first so a dropped tail
/// cannot grow memory forever.
#[derive(Default)]
pub struct FragmentBuffer {
    parts: HashMap<String, (BTreeMap<u64, Vec<u8>>, u64)>,
    order: Vec<String>,
}

const MAX_TRACKED: usize = 256;

impl FragmentBuffer {
    /// Feed one chunk; `Some(bytes)` when the message is complete.
    pub fn push(&mut self, id: &str, sum: u64, seq: u64, chunk: Vec<u8>) -> Option<Vec<u8>> {
        if sum <= 1 {
            return Some(chunk);
        }
        let entry = self.parts.entry(id.to_string()).or_insert_with(|| {
            self.order.push(id.to_string());
            (BTreeMap::new(), sum)
        });
        if entry.0.insert(seq, chunk).is_none() && entry.0.len() >= entry.1 as usize {
            let (parts, _) = self.parts.remove(id).expect("just inserted");
            self.order.retain(|k| k != id);
            return Some(parts.into_values().flatten().collect());
        }
        while self.order.len() > MAX_TRACKED {
            let oldest = self.order.remove(0);
            self.parts.remove(&oldest);
            tracing::warn!(message_id = %oldest, "pb-frame: dropped incomplete fragment buffer");
        }
        None
    }
}

/// Configured semantics for the `pb-frame` framing (parsed from
/// `stream_config.pb_frame`).
pub struct PbFrameProfile {
    /// Header key that discriminates frames (default `type`).
    pub type_header: String,
    /// Heartbeat: client sends CONTROL frames with this type value.
    pub ping_type: String,
    /// Server PONG type (logged, no action).
    pub pong_type: String,
    pub ping_interval_secs: u64,
    /// DATA frames whose type header equals this are events.
    pub event_type: String,
    /// Fragment headers (message id / total / index).
    pub frag_id: String,
    pub frag_sum: String,
    pub frag_seq: String,
    /// Reply an ack DATA frame after a routed event (HTTP-ish code body).
    pub ack: bool,
    pub ack_code: u16,
}

impl PbFrameProfile {
    /// Parse from `stream_config.pb_frame` (all fields defaulted).
    #[must_use]
    pub fn parse(cfg: &Value) -> Self {
        let str_of = |path: &str, default: &str| {
            cfg.pointer(path)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string()
        };
        Self {
            type_header: str_of("/type_header", "type"),
            ping_type: str_of("/ping_type", "ping"),
            pong_type: str_of("/pong_type", "pong"),
            ping_interval_secs: cfg
                .get("ping_interval_secs")
                .and_then(Value::as_u64)
                .unwrap_or(25),
            event_type: str_of("/events/equals", "event"),
            frag_id: str_of("/fragment/id_header", "message_id"),
            frag_sum: str_of("/fragment/sum_header", "sum"),
            frag_seq: str_of("/fragment/seq_header", "seq"),
            ack: cfg.get("ack").and_then(Value::as_bool).unwrap_or(true),
            ack_code: cfg
                .pointer("/ack_code")
                .and_then(Value::as_u64)
                .unwrap_or(200) as u16,
        }
    }
}

/// Build the ack reply for a completed event: the received frame's headers
/// with payload swapped for `{"code": N}` (gateway convention).
#[must_use]
pub fn ack_frame(original: &PbFrame, code: u16) -> PbFrame {
    let mut frame = original.clone();
    frame.method = 1;
    frame.payload = Some(serde_json::json!({"code": code}).to_string().into_bytes());
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message as _;

    #[test]
    fn frame_roundtrip() {
        let frame = PbFrame {
            seq_id: 1,
            log_id: 2,
            service: 42,
            method: 1,
            headers: vec![
                PbHeader {
                    key: "type".into(),
                    value: "event".into(),
                },
                PbHeader {
                    key: "message_id".into(),
                    value: "m-1".into(),
                },
            ],
            payload: Some(br#"{"text":"hi"}"#.to_vec()),
            payload_encoding: Some("json".into()),
            payload_type: None,
            log_id_new: None,
        };
        let bytes = frame.encode_to_vec();
        let back = PbFrame::decode(bytes.as_slice()).expect("decode");
        assert_eq!(back, frame);
        assert_eq!(back.header("type"), Some("event"));
        assert_eq!(back.header("message_id"), Some("m-1"));
    }

    #[test]
    fn fragment_reassembly_in_order_and_out_of_order() {
        let mut buf = FragmentBuffer::default();
        assert!(buf.push("m", 2, 1, b"ab".to_vec()).is_none());
        let joined = buf.push("m", 2, 2, b"cd".to_vec()).expect("complete");
        assert_eq!(joined, b"abcd".to_vec());
        // Out-of-order delivery also joins by seq.
        let mut buf = FragmentBuffer::default();
        assert!(buf.push("n", 2, 2, b"yz".to_vec()).is_none());
        assert_eq!(
            buf.push("n", 2, 1, b"x".to_vec()).expect("complete"),
            b"xyz".to_vec()
        );
    }

    #[test]
    fn single_chunk_passthrough() {
        let mut buf = FragmentBuffer::default();
        assert_eq!(
            buf.push("s", 1, 1, b"one".to_vec()).expect("complete"),
            b"one".to_vec()
        );
    }
}
