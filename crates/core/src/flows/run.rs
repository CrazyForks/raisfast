//! Instance coordinator: DB-backed execution with per-node durable snapshots
//! (dev-docs/workflow db-schema.md §5, reliability A.3).
//!
//! Pipeline: load instance+version → build/seed snapshot → engine `run_persisted`
//! with a DB [`Persist`] (claim + every node completion → upsert snapshot) →
//! finalize instance status/outputs. Completed nodes never re-run on resume
//! (snapshot carries their `success` state).

use async_trait::async_trait;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::IntegrationPlane;
use crate::plugins::PluginManager;
use crate::types::snowflake_id::SnowflakeId;
use std::sync::Arc;

use super::exec::FlowsExec;

use super::engine::{self, NodeExecutor, Persist, S_FAILED, S_SUCCESS, S_WAITING, Snapshot};
use super::graph::{self, Graph};
use super::model;

/// Persist the snapshot to `flow_instance_snapshot` (1:1 upsert).
struct DbPersist {
    pool: crate::db::Pool,
    instance_id: SnowflakeId,
}

#[async_trait]
impl Persist for DbPersist {
    async fn persist(&self, snap: &Snapshot) -> AppResult<()> {
        let value = serde_json::to_value(snap)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot serialize: {e}")))?;
        model::upsert_snapshot(&self.pool, self.instance_id, &value).await
    }
}

/// C4 node-event decorator over a DB persist: diffs consecutive snapshots and
/// emits `workflow.node_*` events (contracts.md C4.2). The await resume path
/// (`resume_snapshot` outside the run loop) emits `await_resumed` separately,
/// so `waiting → success` transitions are NOT reported here.
struct EventingPersist<P: Persist> {
    inner: P,
    instance_id: SnowflakeId,
    node_types: std::collections::HashMap<String, String>,
    /// node_id → (last emitted status, attempt) — interior mutability over
    /// `&self` persists.
    seen: std::sync::Mutex<std::collections::HashMap<String, (String, i64)>>,
    bus: Option<crate::eventbus::EventBus>,
}

impl<P: Persist> EventingPersist<P> {
    fn emit(&self, event_type: &str, _node_id: &str, data: serde_json::Value) {
        let Some(bus) = &self.bus else { return };
        let envelope = serde_json::json!({
            "type": event_type,
            "ts": crate::utils::tz::now_utc().to_rfc3339(),
            "seq": super::events::next_seq(),
            "instance_id": self.instance_id.to_string(),
            "data": data,
        });
        bus.emit(crate::event::Event::Custom {
            source: "flows".into(),
            event_type: event_type.into(),
            data: envelope,
        });
    }

    fn diff_and_emit(&self, snap: &Snapshot) {
        let mut seen = match self.seen.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        for (node_id, st) in &snap.node_states {
            let cur = (st.status.clone(), st.attempt);
            let prev = seen.get(node_id).cloned();
            if prev
                .as_ref()
                .is_some_and(|(s, a)| s == &cur.0 && *a == cur.1)
            {
                continue; // unchanged since last emission
            }
            let node_type = self
                .node_types
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            match cur.0.as_str() {
                "in_progress" => {
                    // failed→in_progress with attempt bump = retry (C4.2).
                    if prev.as_ref().is_some_and(|(s, _)| s == "failed") {
                        self.emit(
                            super::events::EV_NODE_RETRY,
                            node_id,
                            serde_json::json!({"node_id": node_id, "attempt_next": cur.1}),
                        );
                    }
                    self.emit(
                        super::events::EV_NODE_STARTED,
                        node_id,
                        serde_json::json!({
                            "node_id": node_id, "node_type": node_type, "attempt": cur.1,
                        }),
                    );
                }
                "waiting" => {
                    self.emit(
                        super::events::EV_AWAIT_PAUSED,
                        node_id,
                        serde_json::json!({"node_id": node_id, "node_type": node_type}),
                    );
                }
                // Terminal states only reported when WE saw them start
                // (fresh pass); replayed success from resume passes is
                // not a re-run.
                "success" | "failed" | "skipped" | "error_output" if prev.is_some() => {
                    self.emit(
                        super::events::EV_NODE_FINISHED,
                        node_id,
                        serde_json::json!({
                            "node_id": node_id,
                            "status": cur.0,
                            "attempt": cur.1,
                            "latency_ms": st.latency_ms,
                            "outputs_summary": st.output.as_ref()
                                .map(super::events::summarize_output),
                            "error": st.error,
                        }),
                    );
                }
                _ => {}
            }
            seen.insert(node_id.clone(), cur);
        }
    }
}

#[async_trait]
impl<P: Persist> Persist for EventingPersist<P> {
    async fn persist(&self, snap: &Snapshot) -> AppResult<()> {
        self.inner.persist(snap).await?;
        self.diff_and_emit(snap);
        Ok(())
    }
}

/// Load a graph from the instance's locked flow version.
async fn load_graph_for_instance(
    pool: &crate::db::Pool,
    inst: &model::FlowInstance,
) -> AppResult<Graph> {
    let version = model::find_version_by_id(pool, inst.flow_version_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    graph::load_definition(&version.definition)
}

/// Seed the start-namespace pool from the instance trigger inputs.
fn seed_pool(
    inputs: Option<&serde_json::Value>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let mut ns = std::collections::HashMap::new();
    if let Some(obj) = inputs.and_then(|v| v.as_object()) {
        for (k, v) in obj {
            ns.insert(k.clone(), v.clone());
        }
    }
    ns
}

pub async fn run_flow_latest(
    pool: &crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
    flow_id: SnowflakeId,
    inputs: Option<serde_json::Value>,
    trigger: &str,
) -> AppResult<model::FlowInstance> {
    let flow = model::find_flow_by_id(pool, flow_id).await?;
    let version = model::latest_version(pool, flow_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    graph::load_definition(&version.definition)?;

    let instance_id = crate::utils::id::new_snowflake_id();
    let now = crate::utils::tz::now_utc();
    let instance = model::FlowInstance {
        id: instance_id,
        tenant_id: flow.tenant_id.clone(),
        flow_id,
        flow_version_id: version.id,
        status: "running".into(),
        has_exceptions: false,
        trigger_kind: trigger.to_string(),
        trigger_payload: inputs,
        inputs_summary: None,
        outputs: None,
        error: None,
        started_by: None,
        started_at: Some(now),
        finished_at: None,
        waiting_kind: None,
        waiting_needed: None,
        waiting_received: 0,
        resume_until: None,
        created_at: now,
    };
    model::insert_flow_instance(pool, &instance).await?;

    let exec = FlowsExec {
        plane,
        plugins,
        llm: None,
        tenant_id: Some(flow.tenant_id.clone()),
    };
    execute_instance(pool, instance_id, &exec).await?;
    model::find_instance_by_id(pool, instance_id).await
}

/// Run an ad-hoc definition (current canvas / draft) against a flow without
/// publishing a version. The instance references the latest published version
/// id; execution + node-runs reflect the provided definition. trigger='test'.
pub async fn run_definition_latest(
    pool: &crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
    flow_id: SnowflakeId,
    definition: serde_json::Value,
    inputs: Option<serde_json::Value>,
) -> AppResult<model::FlowInstance> {
    graph::load_definition(&definition)?;
    let flow = model::find_flow_by_id(pool, flow_id).await?;
    let version = model::latest_version(pool, flow_id)
        .await?
        .ok_or_else(|| AppError::not_found("flow_version"))?;
    let instance_id = crate::utils::id::new_snowflake_id();
    let now = crate::utils::tz::now_utc();
    let instance = model::FlowInstance {
        id: instance_id,
        tenant_id: flow.tenant_id.clone(),
        flow_id,
        flow_version_id: version.id,
        status: "running".into(),
        has_exceptions: false,
        trigger_kind: "test".into(),
        trigger_payload: inputs,
        inputs_summary: None,
        outputs: None,
        error: None,
        started_by: None,
        started_at: Some(now),
        finished_at: None,
        waiting_kind: None,
        waiting_needed: None,
        waiting_received: 0,
        resume_until: None,
        created_at: now,
    };
    model::insert_flow_instance(pool, &instance).await?;

    // Seed a fresh snapshot from the trigger payload + start params, then run
    // the engine against the provided (unpublished) definition.
    let graph = graph::load_definition(&definition)?;
    let mut ns = seed_pool(instance.trigger_payload.as_ref());
    if let Some(start_node) = graph.nodes.get(&graph.start) {
        let cfg: super::nodes::StartConfig = serde_json::from_value(start_node.data.config.clone())
            .map_err(|e| AppError::BadRequest(format!("start config: {e}")))?;
        super::params::apply(&cfg.params, &mut ns)?;
    }
    let mut snap = Snapshot::new();
    snap.pool.insert(graph.start.clone(), ns);
    let persist = DbPersist {
        pool: pool.clone(),
        instance_id,
    };
    let exec = FlowsExec {
        plane,
        plugins,
        llm: None,
        tenant_id: Some(flow.tenant_id.clone()),
    };
    engine::run_persisted(&graph, &mut snap, &exec, &persist).await?;
    record_node_runs(pool, instance_id, &graph, &snap).await?;

    if snap.status == S_WAITING {
        park_instance(pool, instance_id, &graph, &snap).await?;
    } else if snap.status == S_SUCCESS {
        model::finalize_instance(
            pool,
            instance_id,
            S_SUCCESS,
            false,
            snap.outputs.as_ref(),
            None,
        )
        .await?;
    } else if snap.status == S_FAILED {
        model::finalize_instance(
            pool,
            instance_id,
            S_FAILED,
            false,
            None,
            snap.error.as_ref(),
        )
        .await?;
    }
    model::find_instance_by_id(pool, instance_id).await
}

/// Run an instance to completion (idempotent: terminals return early; completed
/// nodes in a persisted snapshot are skipped).
///
/// # Errors
///
/// `AppError` on load/persist failures; node failures are reflected in the
/// instance status (`failed`), not bubbled up.
pub async fn execute_instance(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    exec: &dyn NodeExecutor,
) -> AppResult<()> {
    let inst = model::find_instance_by_id(pool, instance_id).await?;
    if matches!(inst.status.as_str(), S_SUCCESS | S_FAILED | "canceled") {
        return Ok(()); // terminal
    }
    let graph = load_graph_for_instance(pool, &inst).await?;

    // Load persisted snapshot, or seed a fresh one.
    let mut snap: Snapshot = match model::find_snapshot(pool, instance_id).await? {
        Some(v) => serde_json::from_value(v)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?,
        None => {
            // Fresh run → C4 `graph_run_started` (resume passes stay silent).
            super::events::emit(
                instance_id,
                super::events::EV_RUN_STARTED,
                serde_json::json!({
                    "flow_id": inst.flow_id.to_string(),
                    "flow_version": inst.flow_version_id.to_string(),
                    "trigger_kind": inst.trigger_kind,
                }),
            );
            let mut ns = seed_pool(inst.trigger_payload.as_ref());
            if let Some(start_node) = graph.nodes.get(&graph.start) {
                let cfg: super::nodes::StartConfig =
                    serde_json::from_value(start_node.data.config.clone())
                        .map_err(|e| AppError::BadRequest(format!("start config: {e}")))?;
                super::params::apply(&cfg.params, &mut ns)?;
            }
            let mut s = Snapshot::new();
            s.pool.insert(graph.start.clone(), ns);
            s
        }
    };

    // EventingPersist decorates the DB persist: every persisted snapshot is
    // diffed and turned into C4 node events (started/finished/retry/paused).
    let node_types: std::collections::HashMap<String, String> = graph
        .nodes
        .iter()
        .map(|(id, n)| (id.clone(), n.data.kind.clone()))
        .collect();
    let persist = EventingPersist {
        inner: DbPersist {
            pool: pool.clone(),
            instance_id,
        },
        instance_id,
        node_types,
        seen: std::sync::Mutex::new(std::collections::HashMap::new()),
        bus: super::events::bus(),
    };
    engine::run_persisted(&graph, &mut snap, exec, &persist).await?;

    record_node_runs(pool, instance_id, &graph, &snap).await?;

    if snap.status == S_WAITING {
        // Parked on an await node: keep the snapshot; a resume call continues.
        park_instance(pool, instance_id, &graph, &snap).await?;
        return Ok(());
    }
    if snap.status == S_SUCCESS {
        model::finalize_instance(
            pool,
            instance_id,
            S_SUCCESS,
            false,
            snap.outputs.as_ref(),
            None,
        )
        .await?;
        super::events::emit(
            instance_id,
            super::events::EV_RUN_FINISHED,
            serde_json::json!({
                "status": "success",
                "outputs": snap.outputs,
            }),
        );
        // Keep the snapshot? Terminal success can drop it; keep for replay (P2).
    } else if snap.status == S_FAILED {
        model::finalize_instance(
            pool,
            instance_id,
            S_FAILED,
            false,
            None,
            snap.error.as_ref(),
        )
        .await?;
        super::events::emit(
            instance_id,
            super::events::EV_RUN_FINISHED,
            serde_json::json!({
                "status": "failed",
                "error": snap.error,
            }),
        );
    }
    Ok(())
}

/// Mirror terminal node states onto `flow_node_run` (upsert per node). Ran for
/// every pass (initial + after resume), so a node parked as `waiting` gets its
/// row flipped to `success` once resumed.
async fn record_node_runs(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    graph: &Graph,
    snap: &Snapshot,
) -> AppResult<()> {
    let mut ordered: Vec<&String> = snap.exec_order.iter().collect();
    for id in snap.node_states.keys() {
        if !snap.exec_order.contains(id) {
            ordered.push(id);
        }
    }
    for node_id in ordered {
        let Some(st) = snap.node_states.get(node_id) else {
            continue;
        };
        let status = st.status.as_str();
        if !matches!(
            status,
            "success" | "failed" | "skipped" | "waiting" | "error_output"
        ) {
            continue;
        }
        let Some(node) = graph.nodes.get(node_id) else {
            continue;
        };
        let error = st.error.as_ref().map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                v.to_string()
            }
        });
        let input = st.input.as_ref().map(|v| v.to_string());
        let output = st.output.as_ref().map(|v| v.to_string());
        let usage = st.usage.as_ref().map(|v| v.to_string());
        model::record_node_run(
            pool,
            instance_id,
            node_id,
            node.data.kind.as_str(),
            status,
            st.attempt,
            input.as_deref(),
            output.as_deref(),
            error.as_deref(),
            usage.as_deref(),
            st.latency_ms,
        )
        .await?;
    }
    Ok(())
}

/// Fallback wait cap for await nodes without `timeout_secs`
/// (await-node.md §5 layer 3 — zombie reaping). `[自造]` const for v1;
/// config-ization is a follow-up if a real need appears.
pub const DEFAULT_AWAIT_TIMEOUT_SECS: i64 = 7 * 24 * 3600;

/// What the resume UI needs to know about a parked instance (await-node.md
/// §12: kind-aware controls). Resolved from the instance's LOCKED version —
/// draft edits never shift the goalposts of an in-flight approval.
///
/// # Errors
///
/// `BadRequest` when the instance is not waiting on any node.
pub async fn waiting_info(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
) -> AppResult<serde_json::Value> {
    let inst = model::find_instance_by_id(pool, instance_id).await?;
    if inst.status != "waiting" {
        return Err(AppError::BadRequest(format!(
            "实例状态不是 waiting: {}",
            inst.status
        )));
    }
    let graph = load_graph_for_instance(pool, &inst).await?;
    let Some(value) = model::find_snapshot(pool, instance_id).await? else {
        return Err(AppError::BadRequest("实例无快照".into()));
    };
    let snap: Snapshot = serde_json::from_value(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;
    let Some(node_id) = snap.waiting_nodes.first().cloned() else {
        return Err(AppError::BadRequest("实例没有等待中的节点".into()));
    };
    let cfg = await_node_config(&graph, &node_id)?;
    // Render form_content against the run pool (Dify
    // render_form_content_before_submission): the actor sees interpolated
    // context ("订单 {{#start.order_no#}} 金额 …"), not raw templates.
    let form_content = match super::expr::resolve_text(&cfg.form_content, &snap.pool) {
        Ok(v) => v.as_str().map(str::to_string).unwrap_or_default(),
        Err(_) => cfg.form_content.clone(),
    };
    let actions: Vec<serde_json::Value> = cfg
        .effective_actions()
        .iter()
        .map(|a| {
            serde_json::json!({
                "id": a.id,
                "title": if a.title.is_empty() { a.id.clone() } else { a.title.clone() },
            })
        })
        .collect();
    let inputs: Vec<serde_json::Value> = cfg
        .inputs
        .iter()
        .map(|f| {
            let mut o = serde_json::Map::new();
            o.insert("name".into(), serde_json::json!(f.name));
            o.insert(
                "label".into(),
                serde_json::json!(if f.label.is_empty() {
                    f.name.clone()
                } else {
                    f.label.clone()
                }),
            );
            o.insert("type".into(), serde_json::json!(f.r#type));
            o.insert("required".into(), serde_json::json!(f.required));
            if !f.options.is_empty() {
                o.insert("options".into(), serde_json::json!(f.options));
            }
            serde_json::Value::Object(o)
        })
        .collect();
    let events: Vec<serde_json::Value> = Vec::new();
    Ok(serde_json::json!({
        "node_id": node_id,
        "kind": "human",
        "form_content": form_content,
        "inputs": inputs,
        "actions": actions,
        "approvers": cfg.approvers,
        "timeout_secs": cfg.timeout_secs,
        "events": events,
        "resume_until": inst.resume_until,
    }))
}

/// Await config of a (parked) node, parsed.
fn await_node_config(graph: &Graph, node_id: &str) -> AppResult<super::nodes::AwaitConfig> {
    let node = graph
        .nodes
        .get(node_id)
        .ok_or_else(|| AppError::BadRequest(format!("图里没有节点 {node_id}")))?;
    if node.data.kind != super::nodes::T_AWAIT {
        return Err(AppError::BadRequest(format!(
            "节点 {node_id} 不是 await（{}）",
            node.data.kind
        )));
    }
    serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("await config: {e}")))
}

/// Park: denormalize waiting metadata onto the instance and open the
/// `flow_resume` claim row (await-node.md §3/§5).
async fn park_instance(
    pool: &crate::db::Pool,
    instance_id: SnowflakeId,
    graph: &Graph,
    snap: &Snapshot,
) -> AppResult<()> {
    let Some(node_id) = snap.waiting_nodes.first() else {
        return Ok(());
    };
    let cfg = await_node_config(graph, node_id)?;
    let timeout = cfg.timeout_secs.unwrap_or(DEFAULT_AWAIT_TIMEOUT_SECS);
    let until = crate::utils::tz::now_utc() + chrono::Duration::seconds(timeout);
    model::set_instance_waiting(pool, instance_id, "human", Some(until)).await?;
    model::ensure_flow_resume_open(pool, instance_id, node_id, "human", Some(until)).await?;
    Ok(())
}

/// Complete a parked `await` node with a typed resume envelope and continue
/// the instance (await-node.md §2.2/§3).
///
/// # Errors
///
/// - `BadRequest` when the instance is not waiting / envelope type mismatches
///   the parked node's kind.
/// - `Conflict` (409) when the open claim was already closed by a racing
///   resume or timeout sweep.
pub async fn resume_instance(
    pool: &crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
    instance_id: SnowflakeId,
    envelope: &super::nodes::ResumeEnvelope,
    resumed_by: Option<SnowflakeId>,
) -> AppResult<()> {
    envelope.validate()?;
    let inst = model::find_instance_by_id(pool, instance_id).await?;
    if inst.status != "waiting" {
        return Err(AppError::BadRequest(format!(
            "实例状态不是 waiting: {}",
            inst.status
        )));
    }
    let graph = load_graph_for_instance(pool, &inst).await?;
    let Some(value) = model::find_snapshot(pool, instance_id).await? else {
        return Err(AppError::BadRequest("实例无快照".into()));
    };
    let mut snap: Snapshot = serde_json::from_value(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;
    let Some(node_id) = snap.waiting_nodes.first().cloned() else {
        return Err(AppError::BadRequest("实例没有等待中的节点".into()));
    };
    let cfg = await_node_config(&graph, &node_id)?;
    // The fired action must be a declared action (or the default submit /
    // the built-in timeout sweep).
    {
        let allowed: Vec<&str> = if cfg.actions.is_empty() {
            vec![super::nodes::AWAIT_DEFAULT_ACTION, super::nodes::H_TIMEOUT]
        } else {
            cfg.actions
                .iter()
                .map(|a| a.id.as_str())
                .chain(std::iter::once(super::nodes::H_TIMEOUT))
                .collect()
        };
        if !allowed.contains(&envelope.action.as_str()) {
            return Err(AppError::BadRequest(format!(
                "resume: action '{}' 不是该节点的操作（{}）",
                envelope.action,
                allowed.join(" | ")
            )));
        }
    }

    // Claim first — the single serializer against concurrent resumes (§3).
    let envelope_json = serde_json::to_value(envelope)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("envelope serialize: {e}")))?;
    let claimed =
        model::claim_flow_resume(pool, instance_id, &node_id, &envelope_json, resumed_by).await?;
    if !claimed {
        return Err(AppError::Conflict(
            "该等待节点已被 resume（或已超时）".into(),
        ));
    }

    let (payload, handle) = {
        let (payload, h) = envelope.normalize();
        // No declared actions → the default submit keeps single-port semantics
        // (all outgoing edges taken); declared actions route by handle.
        (payload, h.filter(|_| !cfg.actions.is_empty()))
    };
    engine::resume_snapshot(&mut snap, payload, handle.as_deref())?;
    // C4: the resume decision itself (who/what fired it).
    super::events::emit(
        instance_id,
        super::events::EV_AWAIT_RESUMED,
        serde_json::json!({
            "node_id": node_id,
            "resume_kind": if handle.is_some() { "action" } else { "submit" },
            "action": envelope.action,
            "approver": resumed_by.map(|u| u.to_string()),
        }),
    );
    let persist = DbPersist {
        pool: pool.clone(),
        instance_id,
    };
    persist.persist(&snap).await?;
    let exec = FlowsExec {
        plane,
        plugins,
        llm: None,
        tenant_id: Some(inst.tenant_id.clone()),
    };
    execute_instance(pool, instance_id, &exec).await?;
    Ok(())
}

/// Sweep expired await claims (await-node.md §5): timeout port wired → resume
/// along it; unwired → instance failed with a structured reason. Returns the
/// number of claims acted upon.
///
/// # Errors
///
/// Propagates DB/load errors; per-claim failures are logged and skipped.
pub async fn sweep_expired_awaits(
    pool: &crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
) -> AppResult<u64> {
    let rows = model::find_expired_flow_resumes(pool, crate::utils::tz::now_utc()).await?;
    let mut acted = 0_u64;
    for row in rows {
        if let Err(e) = sweep_one(pool, plane.clone(), plugins.clone(), row).await {
            tracing::warn!("await timeout sweep failed: {e}");
            continue;
        }
        acted += 1;
    }
    Ok(acted)
}

async fn sweep_one(
    pool: &crate::db::Pool,
    plane: Option<Arc<IntegrationPlane>>,
    plugins: Option<Arc<PluginManager>>,
    row: model::FlowResume,
) -> AppResult<()> {
    // Claim as timeout so a racing human resume loses (or we lose, cleanly).
    let timeout_envelope = serde_json::json!({"type": "timeout", "resume_until": row.resume_until});
    let claimed =
        model::claim_flow_resume(pool, row.instance_id, &row.node_id, &timeout_envelope, None)
            .await?;
    if !claimed {
        return Ok(());
    }
    let inst = model::find_instance_by_id(pool, row.instance_id).await?;
    if inst.status != "waiting" {
        return Ok(()); // already terminal / resumed elsewhere
    }
    let graph = load_graph_for_instance(pool, &inst).await?;
    let Some(value) = model::find_snapshot(pool, row.instance_id).await? else {
        return Ok(());
    };
    let mut snap: Snapshot = serde_json::from_value(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("snapshot parse: {e}")))?;
    if !snap.waiting_nodes.contains(&row.node_id) {
        return Ok(()); // snapshot moved on; nothing to do
    }
    let timeout_wired = graph.out_edges.get(&row.node_id).is_some_and(|idx| {
        idx.iter()
            .any(|&ei| graph.edges[ei].source_handle == super::nodes::H_TIMEOUT)
    });
    if timeout_wired {
        engine::resume_snapshot(
            &mut snap,
            serde_json::json!({"timeout": true}),
            Some(super::nodes::H_TIMEOUT),
        )?;
        super::events::emit(
            row.instance_id,
            super::events::EV_AWAIT_RESUMED,
            serde_json::json!({
                "node_id": row.node_id,
                "resume_kind": "timeout",
                "action": super::nodes::H_TIMEOUT,
            }),
        );
        let persist = DbPersist {
            pool: pool.clone(),
            instance_id: row.instance_id,
        };
        persist.persist(&snap).await?;
        let exec = FlowsExec {
            plane,
            plugins,
            llm: None,
            tenant_id: Some(inst.tenant_id.clone()),
        };
        execute_instance(pool, row.instance_id, &exec).await?;
    } else {
        // Unwired timeout → instance failed via the error layer (§5.2).
        let st = snap.node_states.entry(row.node_id.clone()).or_default();
        st.status = engine::N_FAILED.to_string();
        st.error = Some(serde_json::json!({"message": "await timeout", "timeout": true}));
        snap.status = S_FAILED.to_string();
        snap.error = Some(serde_json::json!({
            "node_id": row.node_id,
            "error": {"message": "await timeout", "timeout": true}
        }));
        let persist = DbPersist {
            pool: pool.clone(),
            instance_id: row.instance_id,
        };
        persist.persist(&snap).await?;
        record_node_runs(pool, row.instance_id, &graph, &snap).await?;
        model::finalize_instance(
            pool,
            row.instance_id,
            S_FAILED,
            false,
            None,
            snap.error.as_ref(),
        )
        .await?;
        super::events::emit(
            row.instance_id,
            super::events::EV_RUN_FINISHED,
            serde_json::json!({
                "status": "failed",
                "error": snap.error,
            }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    struct StubExec;
    #[async_trait]
    impl NodeExecutor for StubExec {
        async fn exec(
            &self,
            _node: &crate::flows::graph::GraphNode,
            _input: Value,
            _pool: &engine::Pool,
        ) -> AppResult<engine::ExecOutcome> {
            Ok(engine::ExecOutcome {
                output: json!({"stub": true}),
                usage: None,
                latency_ms: None,
            })
        }
    }

    fn now() -> crate::utils::tz::Timestamp {
        crate::utils::tz::now_utc()
    }

    async fn seed_linear_flow(pool: &crate::db::Pool) -> SnowflakeId {
        let flow_id = crate::utils::id::new_snowflake_id();
        let flow = model::Flow {
            id: flow_id,
            tenant_id: "default".into(),
            name: "durable".into(),
            description: None,
            enabled: true,
            current_version: None,
            extra: None,
            created_at: now(),
            updated_at: now(),
        };
        model::insert_flow(pool, &flow).await.unwrap();

        let def = json!({
            "name": "durable",
            "graph": {
                "nodes": [
                    {"id": "start", "data": {"type": "start", "config": {}}},
                    {"id": "e1", "data": {"type": "egress", "config": {"client_key": "k", "op": "o"}}},
                    {"id": "end", "data": {"type": "end", "config": {"outputs": [{"key": "v", "value": {"ref": ["e1"]}}]}}}
                ],
                "edges": [
                    {"source": "start", "target": "e1"},
                    {"source": "e1", "target": "end"}
                ]
            }
        });
        let version = model::FlowVersion {
            id: crate::utils::id::new_snowflake_id(),
            flow_id,
            version_number: 1,
            definition: def,
            created_by: None,
            created_at: now(),
        };
        model::insert_flow_version(pool, &version).await.unwrap();
        model::set_flow_current_version(pool, flow_id, version.id)
            .await
            .unwrap();

        let instance_id = crate::utils::id::new_snowflake_id();
        let inst = model::FlowInstance {
            id: instance_id,
            tenant_id: "default".into(),
            flow_id,
            flow_version_id: version.id,
            status: "running".into(),
            has_exceptions: false,
            trigger_kind: "api".into(),
            trigger_payload: Some(json!({"msg": "hi"})),
            inputs_summary: None,
            outputs: None,
            error: None,
            started_by: None,
            started_at: Some(now()),
            finished_at: None,
            waiting_kind: None,
            waiting_needed: None,
            waiting_received: 0,
            resume_until: None,
            created_at: now(),
        };
        model::insert_flow_instance(pool, &inst).await.unwrap();
        instance_id
    }

    #[tokio::test]
    async fn durable_run_finishes_and_resume_skips_completed() {
        let pool = crate::test_pool!();
        let instance_id = seed_linear_flow(&pool).await;

        // First pass to completion.
        execute_instance(&pool, instance_id, &StubExec)
            .await
            .unwrap();
        let inst = model::find_instance_by_id(&pool, instance_id)
            .await
            .unwrap();
        assert_eq!(inst.status, "success");
        assert_eq!(inst.outputs.unwrap()["v"]["stub"], true);
        let snap_val = model::find_snapshot(&pool, instance_id)
            .await
            .unwrap()
            .unwrap();
        let snap: Snapshot = serde_json::from_value(snap_val).unwrap();
        assert_eq!(snap.node_states["start"].status, engine::N_SUCCESS);
        assert_eq!(snap.node_states["e1"].status, engine::N_SUCCESS);

        // Terminal → no-op on second call.
        execute_instance(&pool, instance_id, &StubExec)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resume_does_not_rerun_completed_start() {
        let pool = crate::test_pool!();
        let instance_id = seed_linear_flow(&pool).await;

        // Simulate crash after start completed: snapshot with start success,
        // e1 undecided/untouched.
        let mut snap = Snapshot::new();
        snap.node_states.insert(
            "start".into(),
            engine::NodeState {
                input: None,
                status: engine::N_SUCCESS.into(),
                output: Some(Value::Null),
                error: None,
                attempt: 1,
                usage: None,
                latency_ms: None,
                progress: None,
            },
        );
        let snap_json = serde_json::to_value(&snap).unwrap();
        model::upsert_snapshot(&pool, instance_id, &snap_json)
            .await
            .unwrap();

        execute_instance(&pool, instance_id, &StubExec)
            .await
            .unwrap();
        let inst = model::find_instance_by_id(&pool, instance_id)
            .await
            .unwrap();
        assert_eq!(inst.status, "success");
        let snap_val = model::find_snapshot(&pool, instance_id)
            .await
            .unwrap()
            .unwrap();
        let snap2: Snapshot = serde_json::from_value(snap_val).unwrap();
        // start not re-run (attempt stays 1), e1 ran once.
        assert_eq!(snap2.node_states["start"].attempt, 1);
        assert_eq!(snap2.node_states["e1"].status, engine::N_SUCCESS);
    }
}
