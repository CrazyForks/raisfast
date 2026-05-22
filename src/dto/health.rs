use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub components: HashMap<String, ComponentHealth>,
    pub uptime_seconds: u64,
    pub version: String,
}

#[derive(Serialize)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HealthStatus {
    Up,
    Down,
    Degraded,
}
