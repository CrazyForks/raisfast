//! Internal flow triggers (kind=event): an eventbus subscriber matches events
//! to enabled `flow_trigger` rows and runs their referenced flows.
//!
//! Decoupled: a trigger points at a flow; flows never know their triggers.
//! Public API exposure stays a separate concern (flow_api_key).

use std::sync::Arc;

use crate::db::Pool;
use crate::plugins::PluginManager;

use super::model;
use super::run;

/// Resolve a trigger `inputs_map` value against the event payload.
/// Supported forms per key:
/// - `{"literal": v}` → v
/// - `{"ref": ["data","a","b"]}` or `{"ref": ["a","b"]}` → walk the payload
/// - plain scalar/object without those markers → used as-is
pub fn resolve_input(source: &serde_json::Value, payload: &serde_json::Value) -> serde_json::Value {
    if let Some(obj) = source.as_object() {
        if let Some(lit) = obj.get("literal") {
            return lit.clone();
        }
        if let Some(r) = obj.get("ref").and_then(|v| v.as_array()) {
            let mut cur = payload;
            for part in r {
                let Some(key) = part.as_str() else {
                    return serde_json::Value::Null;
                };
                if key == "data" && cur.is_object() && cur.get("data").is_some() {
                    // keep walking inside the real event data envelope
                    if let Some(d) = cur.get("data") {
                        cur = d;
                    }
                    continue;
                }
                let Some(next) = cur.get(key) else {
                    return serde_json::Value::Null;
                };
                cur = next;
            }
            return cur.clone();
        }
    }
    source.clone()
}

/// Build flow start inputs from a trigger's `inputs_map` (default: whole data).
pub fn build_inputs(
    inputs_map: Option<&serde_json::Value>,
    event_data: &serde_json::Value,
) -> serde_json::Value {
    match inputs_map {
        None => event_data.clone(),
        Some(serde_json::Value::Object(map)) if !map.is_empty() => {
            let mut out = serde_json::Map::new();
            for (k, expr) in map {
                out.insert(k.clone(), resolve_input(expr, event_data));
            }
            serde_json::Value::Object(out)
        }
        Some(other) => other.clone(),
    }
}

/// Whether the trigger's optional `filter` passes. Only an explicit `true`
/// literal (or absent) enables the trigger; anything else is logged + skipped.
fn filter_passes(filter: Option<&serde_json::Value>) -> bool {
    match filter {
        None => true,
        Some(v) => matches!(v, serde_json::Value::Bool(true)),
    }
}

async fn run_trigger(
    pool: Pool,
    plugins: Arc<PluginManager>,
    trigger: model::FlowTrigger,
    event_data: serde_json::Value,
) {
    let inputs = build_inputs(trigger.inputs_map.as_ref(), &event_data);
    if let Err(e) = run::run_flow_latest(
        &pool,
        crate::integration::shared(),
        Some(plugins),
        trigger.flow_id,
        Some(inputs),
        "event",
    )
    .await
    {
        tracing::warn!(
            "flow event trigger {} ({}) failed: {e}",
            trigger.id,
            trigger.event_type.as_deref().unwrap_or("")
        );
    }
}

/// Spawn the eventbus subscriber that forwards matching events to flow triggers.
pub fn spawn_flow_event_subscriber(
    eventbus: crate::eventbus::EventBus,
    pool: Pool,
    plugins: Arc<PluginManager>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut rx = eventbus.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(_) if *shutdown_rx.borrow() => break,
                        Err(_) => break,
                        _ => {}
                    }
                }
                result = rx.recv() => {
                    let event = match result {
                        Ok(e) => e,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("flow trigger subscriber lagged, skipped {n} events");
                            continue;
                        }
                    };
                    let Some(event_type) = event.event_name().map(|s| s.into_owned()) else {
                        continue;
                    };
                    let payload_value =
                        serde_json::to_value(event.as_ref()).unwrap_or_default();
                    let event_data = payload_value
                        .get("data")
                        .cloned()
                        .unwrap_or(payload_value);
                    let triggers = match model::list_flow_triggers_by_event(
                        &pool,
                        crate::constants::DEFAULT_TENANT,
                        &event_type,
                    )
                    .await
                    {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("flow trigger lookup error: {e}");
                            continue;
                        }
                    };
                    for trigger in triggers {
                        if !filter_passes(trigger.filter.as_ref()) {
                            tracing::debug!(
                                "flow trigger {} filter not satisfied, skip",
                                trigger.id
                            );
                            continue;
                        }
                        run_trigger(
                            pool.clone(),
                            plugins.clone(),
                            trigger,
                            event_data.clone(),
                        )
                        .await;
                    }
                }
            }
        }
        tracing::info!("flow event trigger subscriber shutting down");
    });
}

/// Spawn the cron ticker that runs enabled `kind=cron` flow triggers.
/// Uses the worker cron `next_run` semantics; fires at most once per second per
/// trigger (guarded by `last_triggered_at`).
pub fn spawn_flow_cron_subscriber(
    pool: Pool,
    plugins: Arc<PluginManager>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
        ticker.tick().await; // first tick fires immediately otherwise
        loop {
            tokio::select! {
                biased;
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(_) if *shutdown_rx.borrow() => break,
                        Err(_) => break,
                        _ => {}
                    }
                }
                _ = ticker.tick() => {
                    let triggers = match model::list_flow_triggers_cron(&pool, crate::constants::DEFAULT_TENANT).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!("flow cron trigger list error: {e}");
                            continue;
                        }
                    };
                    let now = crate::utils::tz::now_utc();
                    for trigger in triggers {
                        // Never run before: anchor in the past so the next
                        // occurrence lands at-or-before "now" and fires once.
                        let after = trigger
                            .last_triggered_at
                            .unwrap_or(now - chrono::Duration::hours(1));
                        let due = crate::worker::next_run(
                            trigger.cron_expr.as_deref().unwrap_or(""),
                            after,
                        )
                        .ok();
                        let Some(next) = due else {
                            tracing::warn!(
                                "flow cron trigger {} invalid expr, skip",
                                trigger.id
                            );
                            continue;
                        };
                        if next > now {
                            continue; // not due yet
                        }
                        let inputs = build_inputs(trigger.inputs_map.as_ref(), &serde_json::json!({}));
                        if let Err(e) = run::run_flow_latest(
                            &pool,
                            crate::integration::shared(),
                            Some(plugins.clone()),
                            trigger.flow_id,
                            Some(inputs),
                            "cron",
                        )
                        .await
                        {
                            tracing::warn!("flow cron trigger {} run failed: {e}", trigger.id);
                            continue;
                        }
                        let _ = model::set_flow_trigger_last_triggered(&pool, trigger.id, now).await;
                    }
                }
            }
        }
        tracing::info!("flow cron trigger subscriber shutting down");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_inputs_ref_and_literal() {
        let map = json!({
            "user_id": {"ref": ["user_id"]},
            "tag": {"literal": "welcome"},
            "raw": "scalar"
        });
        let data = json!({"user_id": 88, "extra": true});
        let out = build_inputs(Some(&map), &data);
        assert_eq!(out["user_id"], 88);
        assert_eq!(out["tag"], "welcome");
        assert_eq!(out["raw"], "scalar");
    }

    #[test]
    fn build_inputs_default_whole_payload() {
        let data = json!({"a": 1});
        assert_eq!(build_inputs(None, &data), data);
    }
}
