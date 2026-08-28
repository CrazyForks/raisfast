//! InboundEnvelope — the single normalized shape of every inbound message,
//! regardless of mode (push/pull/stream/listen) or protocol.
//!
//! Business code and plugins only ever see this type, never the wire protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::snowflake_id::SnowflakeId;
use crate::utils::tz::Timestamp;

/// What kind of inbound item this is. Drives routing behavior and telemetry
/// sampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundKind {
    /// Human/bot conversation message (IM text, email body, chat widget).
    Message,
    /// State notification from the provider (subscription changes, group events).
    Event,
    /// Request→callback style (payment result, OAuth code exchange).
    Callback,
    /// High-frequency machine data (sampled; batch pipeline in P2).
    Telemetry,
    /// The connection lifecycle itself (connected/disconnected/degraded).
    ConnectionState,
}

impl InboundKind {
    /// Stable wire name (also used in `itg_receipts.kind`).
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Message => "message",
            Self::Event => "event",
            Self::Callback => "callback",
            Self::Telemetry => "telemetry",
            Self::ConnectionState => "connection_state",
        }
    }

    /// Parse from the stored wire name. Unknown values map to `None`.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "message" => Some(Self::Message),
            "event" => Some(Self::Event),
            "callback" => Some(Self::Callback),
            "telemetry" => Some(Self::Telemetry),
            "connection_state" => Some(Self::ConnectionState),
            _ => None,
        }
    }
}

/// Connection context, filled for stream/listen modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionCtx {
    /// Supervisor-assigned session id (locates the connection for egress replies).
    pub session_id: String,
    /// Remote-side metadata (node version, gateway id, …).
    pub remote_meta: Value,
}

/// The normalized envelope. See integration.md §4.
///
/// `payload` is always a JSON tree — protobuf/XML sources are converted at L2,
/// so neither business code nor the plugin ecosystem needs a second codec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundEnvelope {
    /// Receipt id — doubles as the whole-chain trace id (integration.md §10.7).
    pub receipt_id: SnowflakeId,
    pub channel_id: SnowflakeId,
    pub provider: String,
    /// Provider-side unique id — the idempotency key.
    pub external_id: String,

    /// Normalized sender identity (open id / address / device id).
    pub sender: Option<String>,
    pub recipient: Option<String>,

    pub kind: InboundKind,
    /// Normalized structured data (mapping / plugin output).
    pub payload: Value,
    /// VFS archive path of the raw message (replay & audit source).
    pub raw_ref: Option<String>,

    /// Connection context for stream/listen modes.
    pub connection: Option<ConnectionCtx>,

    /// Connector-side arrival moment (queue wait = received_at - ingested_at).
    pub ingested_at: Timestamp,
    /// Pipeline start moment.
    pub received_at: Timestamp,
}
