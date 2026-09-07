//! C4 SSE 事件（workflow.* 命名空间，contracts.md §C4）。
//!
//! 发射点把 C4 事件表打到全局 EventBus：`GET /api/v1/events?filter=workflow.*`
//! （SSE）、插件与 webhook 订阅（JobEnqueuer → WebhookNotify）三条消费面同时
//! 打通（C4.4）；专用实例端点 `GET /admin/flows/instances/{id}/events` 在
//! handler 侧按 instance_id 过滤同一总线。
//!
//! 参考矩阵：事件表与终态保证 [照抄 contracts.md C4（P0.1 冻结契约）]；
//! 送达模型（在带 SSE + 快照回放）[照抄 dify workflow events]
//! （`services/workflow_event_snapshot_service.py` 的 replay 形态，v1 弱化为
//! 无回放——C4.3 明示"重连即查实例 status 继续"）。

use crate::event::Event;
use crate::eventbus::EventBus;
use crate::types::snowflake_id::SnowflakeId;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};

/// Set once at server startup (same pattern as `panic_hook::set_event_bus`).
static BUS: OnceLock<EventBus> = OnceLock::new();
/// Monotonic per-process sequence (per-instance monotonic follows — strictly
/// increasing for every emission on any instance).
static SEQ: AtomicI64 = AtomicI64::new(1);

#[must_use]
pub fn next_seq() -> i64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

pub fn set_bus(bus: EventBus) {
    let _ = BUS.set(bus);
}

#[must_use]
pub fn bus() -> Option<EventBus> {
    BUS.get().cloned()
}

pub const EV_RUN_STARTED: &str = "workflow.graph_run_started";
pub const EV_NODE_STARTED: &str = "workflow.node_run_started";
pub const EV_NODE_FINISHED: &str = "workflow.node_run_finished";
pub const EV_NODE_RETRY: &str = "workflow.node_run_retry";
pub const EV_AWAIT_PAUSED: &str = "workflow.node_await_paused";
pub const EV_AWAIT_RESUMED: &str = "workflow.node_await_resumed";
pub const EV_RUN_FINISHED: &str = "workflow.graph_run_finished";

/// Emit a C4-shaped event `{type, ts, seq, instance_id, data}` onto the
/// global bus. No bus configured (unit tests / CLI) → no-op.
pub fn emit(instance_id: SnowflakeId, event_type: &str, data: serde_json::Value) {
    let Some(bus) = bus() else { return };
    let envelope = serde_json::json!({
        "type": event_type,
        "ts": crate::utils::tz::now_utc().to_rfc3339(),
        "seq": next_seq(),
        "instance_id": instance_id.to_string(),
        "data": data,
    });
    bus.emit(Event::Custom {
        source: "flows".into(),
        event_type: event_type.into(),
        data: envelope,
    });
}

/// `outputs_summary` 脱敏（C4.2）：JSON 序列化后截断。
#[must_use]
pub fn summarize_output(v: &serde_json::Value) -> String {
    let s = serde_json::to_string(v).unwrap_or_default();
    if s.chars().count() > 200 {
        let cut: String = s.chars().take(200).collect();
        format!("{cut}…")
    } else {
        s
    }
}
