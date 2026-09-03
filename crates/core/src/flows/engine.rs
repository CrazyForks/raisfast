//! Serial execution core (execution-engine.md, contracts C4-adjacent).
//!
//! P1.2 skeleton: ready-queue + single-point state writes + Join/skip + whole
//! snapshot. Internal semantics for start/end/branch; `script`/`egress` run via
//! an injected [`NodeExecutor`] (wired in P1.5/1.6); await/resume in P2.
//!
//! Execution is serial (one node at a time). Fan-out runs every target that has
//! data; join waits for ALL incoming edges to be decided with at least one taken.

use std::collections::{HashMap, VecDeque};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::app_error::{AppError, AppResult};

use super::graph::{Edge, Graph, GraphNode};
use super::nodes::{self, BranchConfig, EndConfig};

/// variable pool: ns -> name -> value (child fields live inside object values).
pub type Pool = HashMap<String, HashMap<String, Value>>;

pub const S_RUNNING: &str = "running";
pub const S_WAITING: &str = "waiting";
pub const S_SUCCESS: &str = "success";
pub const S_FAILED: &str = "failed";
pub const S_SKIPPED: &str = "skipped";

pub const N_SUCCESS: &str = "success";
pub const N_SKIPPED: &str = "skipped";
pub const N_IN_PROGRESS: &str = "in_progress";
pub const N_WAITING: &str = "waiting";
pub const N_FAILED: &str = "failed";

/// One node's per-attempt result (idempotency: success/skipped never re-run).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeState {
    pub status: String,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub attempt: i64,
}

/// `modifiers.retry` config.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RetryModifier {
    #[serde(default)]
    pub attempts: Option<i64>,
    #[allow(dead_code)]
    #[serde(default)]
    pub backoff: Option<String>,
}

/// Orthogonal node modifiers (contracts C1.4). Retry is attempted in-process;
/// `continue_on_error` fails the node but lets the run pass through.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NodeModifiers {
    #[serde(default)]
    pub retry: Option<RetryModifier>,
    #[serde(default)]
    pub continue_on_error: bool,
}

/// Edge verdict for readiness (join = all decided; skip = all skipped).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EdgeMark {
    Taken,
    Skipped,
}

/// Whole runnable snapshot (durable; one row per instance).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub pool: Pool,
    pub node_states: HashMap<String, NodeState>,
    /// keyed `source|handle->target`.
    pub edge_marks: HashMap<String, EdgeMark>,
    pub status: String,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub outputs: Option<Value>,
    /// Nodes parked on `await` (resume completes the head of this list).
    #[serde(default)]
    pub waiting_nodes: Vec<String>,
    /// Node ids in the order this run (re)executed them — observability feed
    /// for `flow_node_run`; not used for scheduling decisions.
    #[serde(default)]
    pub exec_order: Vec<String>,
}

fn edge_key(e: &Edge) -> String {
    format!("{}|{}->{}", e.source, e.source_handle, e.target)
}

impl Snapshot {
    pub fn new() -> Self {
        Self {
            pool: HashMap::new(),
            node_states: HashMap::new(),
            edge_marks: HashMap::new(),
            status: S_RUNNING.to_string(),
            error: None,
            outputs: None,
            waiting_nodes: Vec::new(),
            exec_order: Vec::new(),
        }
    }
}

/// Complete the head waiting node with `payload` and resume the run.
pub fn resume_snapshot(snap: &mut Snapshot, payload: Option<Value>) -> AppResult<()> {
    let Some(node) = snap.waiting_nodes.first().cloned() else {
        return Err(AppError::BadRequest("实例没有等待中的节点".into()));
    };
    snap.waiting_nodes.retain(|n| *n != node);
    let payload = payload.unwrap_or(Value::Null);
    let st = snap.node_states.entry(node.clone()).or_default();
    st.status = N_SUCCESS.to_string();
    st.output = Some(payload.clone());
    st.attempt += 1;
    let ns = snap.pool.entry(node.clone()).or_default();
    ns.insert("resume".to_string(), payload.clone());
    ns.insert("output".to_string(), payload);
    snap.status = S_RUNNING.to_string();
    Ok(())
}

/// What an executable node produced.
#[derive(Debug)]
pub struct ExecOutcome {
    pub output: Value,
}

/// Async executor for action nodes (`script`/`egress`). Wired in P1.5/1.6.
#[async_trait]
pub trait NodeExecutor: Send + Sync {
    async fn exec(&self, node: &GraphNode, input: Value) -> AppResult<ExecOutcome>;
}

/// Durability hook: persist the snapshot after each claim / node completion.
#[async_trait]
pub trait Persist: Send + Sync {
    async fn persist(&self, snap: &Snapshot) -> AppResult<()>;
}

/// No-op persistence (pure in-memory runs / tests).
pub struct NoopPersist;
#[async_trait]
impl Persist for NoopPersist {
    async fn persist(&self, _snap: &Snapshot) -> AppResult<()> {
        Ok(())
    }
}

/// Run one full pass of the graph over the snapshot (serial, in-memory).
pub async fn run(graph: &Graph, snap: &mut Snapshot, exec: &dyn NodeExecutor) -> AppResult<()> {
    run_persisted(graph, snap, exec, &NoopPersist).await
}

/// Serial pass with a durability hook: [`Persist::persist`] is invoked right
/// after a node is claimed (`in_progress`, before its side effect runs) and
/// after each node completes — so a crash mid-node leaves a claimed state and
/// completed nodes never re-run on resume (A.3 claim-then-act, at-least-once).
pub async fn run_persisted(
    graph: &Graph,
    snap: &mut Snapshot,
    exec: &dyn NodeExecutor,
    persist: &dyn Persist,
) -> AppResult<()> {
    if snap.status != S_RUNNING {
        return Ok(());
    }
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(graph.start.clone());

    while let Some(id) = queue.pop_front() {
        if snap.status != S_RUNNING {
            break;
        }
        let Some(node) = graph.nodes.get(&id) else {
            continue;
        };
        // Idempotent resume: already decided nodes never re-run, but their
        // downstream must still be (re)propagated from persisted state.
        if let Some(st) = snap.node_states.get(&id)
            && st.status == N_SUCCESS
        {
            resume_completed(graph, snap, &id, &mut queue)?;
            continue;
        }
        if let Some(st) = snap.node_states.get(&id)
            && st.status == N_SKIPPED
        {
            skip_node(graph, snap, &id, &mut queue)?;
            continue;
        }

        snap.exec_order.push(id.clone());

        let attempt = snap.node_states.get(&id).map_or(1, |s| s.attempt + 1);
        match node.data.kind.as_str() {
            nodes::T_START => {
                set_in_progress(snap, &id, attempt);
                persist.persist(snap).await?;
                mark_node_success(snap, &id, Value::Null);
                fan_out_after_run(graph, snap, &id, &mut queue)?;
            }
            nodes::T_END => {
                set_in_progress(snap, &id, attempt);
                persist.persist(snap).await?;
                let outputs = resolve_end_outputs(node, snap)?;
                finish_success(snap, &id, outputs);
            }
            nodes::T_BRANCH => {
                set_in_progress(snap, &id, attempt);
                persist.persist(snap).await?;
                let cfg: BranchConfig = serde_json::from_value(node.data.config.clone())
                    .map_err(|e| AppError::BadRequest(format!("branch config: {e}")))?;
                let handle = pick_branch(&cfg, &snap.pool)?;
                mark_node_success(snap, &id, json!({"handle": handle}));
                fan_out_after_branch(graph, snap, &id, Some(handle.as_str()), &mut queue)?;
            }
            nodes::T_SCRIPT | nodes::T_EGRESS => {
                let mods: NodeModifiers =
                    serde_json::from_value(node.data.modifiers.clone()).unwrap_or_default();
                let attempts = mods
                    .retry
                    .as_ref()
                    .and_then(|r| r.attempts)
                    .unwrap_or(1)
                    .max(1);
                // Directly after `start` with no explicit `input` mapping: pass
                // the caller's trigger inputs through by default, so external /
                // manual runs reach the first script without extra wiring.
                let has_explicit_input = node.data.config.get("input").is_some();
                let fed_by_start_only = graph
                    .in_edges
                    .get(&id)
                    .map(|idx| {
                        !idx.is_empty()
                            && idx.iter().all(|&ei| graph.edges[ei].source == graph.start)
                    })
                    .unwrap_or(false);
                let input = if !has_explicit_input && fed_by_start_only {
                    let mut m = serde_json::Map::new();
                    if let Some(ns) = snap.pool.get(&graph.start) {
                        for (k, v) in ns {
                            m.insert(k.clone(), v.clone());
                        }
                    }
                    Value::Object(m)
                } else {
                    match resolve_inputs(&node.data.config, &snap.pool) {
                        Ok(v) => v,
                        Err(e) => {
                            fail(snap, &id, e.to_string());
                            continue;
                        }
                    }
                };
                snap.node_states.entry(id.clone()).or_default().input = Some(input.clone());
                let mut last_error: Option<String> = None;
                let mut outcome: Option<ExecOutcome> = None;
                for i in 1..=attempts {
                    // Claim before each attempt: persisted → crash mid-node
                    // re-runs at-least-once from this claim (A.3).
                    set_in_progress(snap, &id, i);
                    persist.persist(snap).await?;
                    match exec.exec(node, input.clone()).await {
                        Ok(o) => {
                            outcome = Some(o);
                            break;
                        }
                        Err(e) => {
                            last_error = Some(e.to_string());
                        }
                    }
                }
                match outcome {
                    Some(o) => {
                        mark_node_success(snap, &id, o.output);
                        fan_out_after_run(graph, snap, &id, &mut queue)?;
                    }
                    None => {
                        let msg = last_error.unwrap_or_default();
                        if mods.continue_on_error {
                            // Fail the node but pass the run through (downstream
                            // continues; its inputs still resolve upstream).
                            let st = snap.node_states.entry(id.clone()).or_default();
                            st.status = N_FAILED.to_string();
                            st.error = Some(json!({"message": msg}));
                            fan_out_after_run(graph, snap, &id, &mut queue)?;
                        } else {
                            fail(snap, &id, msg);
                        }
                    }
                }
            }
            nodes::T_AWAIT => {
                // Park the run: node marked waiting + recorded, instance ->
                // waiting. Resume completes it (see `resume_snapshot`), then the
                // engine's success-resume fan-out continues downstream.
                let st = snap.node_states.entry(id.clone()).or_default();
                st.status = N_WAITING.to_string();
                if !snap.waiting_nodes.contains(&id) {
                    snap.waiting_nodes.push(id.clone());
                }
                snap.status = S_WAITING.to_string();
                persist.persist(snap).await?;
                break;
            }
            other => {
                fail(snap, &id, format!("unsupported node type '{other}'"));
            }
        }
        persist.persist(snap).await?;
    }
    if snap.status == S_RUNNING {
        // queue drained without an end → treat as success (no outputs).
        snap.status = S_SUCCESS.to_string();
        persist.persist(snap).await?;
    }
    Ok(())
}

fn set_in_progress(snap: &mut Snapshot, id: &str, attempt: i64) {
    let st = snap.node_states.entry(id.to_string()).or_default();
    st.status = N_IN_PROGRESS.to_string();
    st.attempt = attempt;
}

fn mark_node_success(snap: &mut Snapshot, id: &str, output: Value) {
    let st = snap.node_states.entry(id.to_string()).or_default();
    st.status = N_SUCCESS.to_string();
    st.output = Some(output.clone());
    if output.is_object() || output.is_array() {
        snap.pool
            .entry(id.to_string())
            .or_default()
            .insert("output".to_string(), output);
    }
}

fn finish_success(snap: &mut Snapshot, id: &str, outputs: Value) {
    let st = snap.node_states.entry(id.to_string()).or_default();
    st.status = N_SUCCESS.to_string();
    st.output = Some(outputs.clone());
    snap.outputs = Some(outputs);
    snap.status = S_SUCCESS.to_string();
}

fn fail(snap: &mut Snapshot, id: &str, msg: String) {
    let st = snap.node_states.entry(id.to_string()).or_default();
    st.status = N_FAILED.to_string();
    st.error = Some(json!({"message": msg}));
    snap.error = Some(json!({"node_id": id, "message": msg}));
    snap.status = S_FAILED.to_string();
}

/// Non-branch node: every outgoing edge is taken; then targets become ready.
fn fan_out_after_run(
    graph: &Graph,
    snap: &mut Snapshot,
    id: &str,
    queue: &mut VecDeque<String>,
) -> AppResult<()> {
    let Some(idx) = graph.out_edges.get(id) else {
        return Ok(());
    };
    for &ei in idx {
        mark(graph, snap, ei, EdgeMark::Taken);
    }
    for &ei in idx {
        consider_target(graph, snap, &graph.edges[ei].target, queue)?;
    }
    Ok(())
}

/// Branch node: only the chosen handle edges are taken; others skipped.
fn fan_out_after_branch(
    graph: &Graph,
    snap: &mut Snapshot,
    id: &str,
    handle: Option<&str>,
    queue: &mut VecDeque<String>,
) -> AppResult<()> {
    let Some(idx) = graph.out_edges.get(id) else {
        return Ok(());
    };
    for &ei in idx {
        let edge = &graph.edges[ei];
        let taken = Some(edge.source_handle.as_str()) == handle;
        mark(
            graph,
            snap,
            ei,
            if taken {
                EdgeMark::Taken
            } else {
                EdgeMark::Skipped
            },
        );
    }
    for &ei in idx {
        consider_target(graph, snap, &graph.edges[ei].target, queue)?;
    }
    Ok(())
}

fn mark(graph: &Graph, snap: &mut Snapshot, ei: usize, mark: EdgeMark) {
    snap.edge_marks.insert(edge_key(&graph.edges[ei]), mark);
}

/// After an incoming edge of `target` was decided: if all decided → ready
/// (any taken) or skipped (all skipped, propagated).
fn consider_target(
    graph: &Graph,
    snap: &mut Snapshot,
    target: &str,
    queue: &mut VecDeque<String>,
) -> AppResult<()> {
    let Some(in_edges) = graph.in_edges.get(target) else {
        return Ok(());
    };
    let decided = in_edges
        .iter()
        .filter(|ei| snap.edge_marks.contains_key(&edge_key(&graph.edges[**ei])))
        .count();
    if decided < in_edges.len() {
        return Ok(()); // still waiting on other branches (join)
    }
    let any_taken = in_edges.iter().any(|ei| {
        matches!(
            snap.edge_marks.get(&edge_key(&graph.edges[*ei])),
            Some(EdgeMark::Taken)
        )
    });
    if any_taken {
        if !queue.contains(&target.to_string()) {
            queue.push_back(target.to_string());
        }
    } else {
        skip_node(graph, snap, target, queue)?;
    }
    Ok(())
}

/// Mark a node skipped and propagate down (skip = its edges are Skipped).
fn skip_node(
    graph: &Graph,
    snap: &mut Snapshot,
    id: &str,
    queue: &mut VecDeque<String>,
) -> AppResult<()> {
    let st = snap.node_states.entry(id.to_string()).or_default();
    st.status = N_SKIPPED.to_string();
    let Some(idx) = graph.out_edges.get(id) else {
        return Ok(());
    };
    for &ei in idx {
        mark(graph, snap, ei, EdgeMark::Skipped);
    }
    for &ei in idx {
        consider_target(graph, snap, &graph.edges[ei].target, queue)?;
    }
    Ok(())
}

/// Replay fan-out for a node already completed in a persisted snapshot (resume).
fn resume_completed(
    graph: &Graph,
    snap: &mut Snapshot,
    id: &str,
    queue: &mut VecDeque<String>,
) -> AppResult<()> {
    let Some(node) = graph.nodes.get(id) else {
        return Ok(());
    };
    match node.data.kind.as_str() {
        nodes::T_BRANCH => {
            let handle = snap
                .node_states
                .get(id)
                .and_then(|s| s.output.as_ref())
                .and_then(|o| o.get("handle"))
                .and_then(Value::as_str)
                .map(str::to_string);
            fan_out_after_branch(graph, snap, id, handle.as_deref(), queue)?;
        }
        nodes::T_END => {}
        _ => {
            fan_out_after_run(graph, snap, id, queue)?;
        }
    }
    Ok(())
}

// ── value resolution (literal/ref; expr → P1.7) ──────────────────────────

fn resolve(raw: &Value, pool: &Pool) -> AppResult<Value> {
    if let Some(arr) = raw.get("ref").and_then(Value::as_array) {
        return resolve_ref(arr, pool);
    }
    if let Some(v) = raw.get("literal") {
        return Ok(v.clone());
    }
    if raw.get("expr").is_some() {
        return Err(AppError::BadRequest("expr 求值尚未接线（P1.7）".into()));
    }
    Ok(raw.clone())
}

fn resolve_ref(sel: &[Value], pool: &Pool) -> AppResult<Value> {
    if sel.len() < 2 {
        return Err(AppError::BadRequest("ref 需至少 [namespace, name]".into()));
    }
    let ns = sel[0].as_str().unwrap_or_default();
    let name = sel[1].as_str().unwrap_or_default();
    let mut v = pool
        .get(ns)
        .and_then(|m| m.get(name))
        .cloned()
        .ok_or_else(|| AppError::BadRequest(format!("ref 引用不存在: {ns}.{name}")))?;
    for part in sel.iter().skip(2) {
        let key = part
            .as_str()
            .ok_or_else(|| AppError::BadRequest("ref 子路径元素必须是字符串".into()))?;
        v = v
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::BadRequest(format!("ref 子路径不存在: {key}")))?;
    }
    Ok(v)
}

fn resolve_inputs(config: &Value, pool: &Pool) -> AppResult<Value> {
    let mut out = serde_json::Map::new();
    if let Some(input) = config.get("input").and_then(Value::as_object) {
        for (k, v) in input {
            out.insert(k.clone(), resolve(v, pool)?);
        }
    }
    Ok(Value::Object(out))
}

fn resolve_end_outputs(node: &GraphNode, snap: &Snapshot) -> AppResult<Value> {
    let cfg: EndConfig = serde_json::from_value(node.data.config.clone())
        .map_err(|e| AppError::BadRequest(format!("end config: {e}")))?;
    let mut out = serde_json::Map::new();
    for o in &cfg.outputs {
        out.insert(o.name.clone(), resolve(&o.value, &snap.pool)?);
    }
    Ok(Value::Object(out))
}

fn pick_branch(cfg: &BranchConfig, pool: &Pool) -> AppResult<String> {
    for rule in &cfg.branches {
        let matched = eval_condition(&rule.when, pool)?;
        if matched {
            return Ok(rule.handle.clone().unwrap_or_default());
        }
    }
    Ok(cfg.else_handle.clone().unwrap_or_default())
}

/// Structured condition `{op, var, value}` or a literal bool. Expression
/// strings (`{{#…#}} >= 3`) require the P1.7 evaluator.
fn eval_condition(when: &Value, pool: &Pool) -> AppResult<bool> {
    if let Some(op) = when.get("op").and_then(Value::as_str) {
        let left = resolve(when.get("var").unwrap_or(&Value::Null), pool)?;
        let right = when.get("value").cloned().unwrap_or(Value::Null);
        return eval_op(op, &left, &right);
    }
    if let Some(b) = when.as_bool() {
        return Ok(b);
    }
    if let Some(s) = when.as_str() {
        return super::expr::eval_bool(s, pool);
    }
    Ok(false)
}

fn eval_op(op: &str, left: &Value, right: &Value) -> AppResult<bool> {
    let num_cmp = |o: &str| -> Option<bool> {
        let l = left.as_f64()?;
        let r = right.as_f64()?;
        Some(match o {
            ">" => l > r,
            ">=" => l >= r,
            "<" => l < r,
            "<=" => l <= r,
            "==" => l == r,
            "!=" => l != r,
            _ => return None,
        })
    };
    let r = match op {
        "==" => equalish(left, right),
        "!=" => !equalish(left, right),
        "in" => right
            .as_array()
            .map(|a| a.iter().any(|x| equalish(x, left)))
            .unwrap_or(false),
        "contains" => left
            .as_str()
            .map(|s| s.contains(right.as_str().unwrap_or_default()))
            .unwrap_or(false),
        "starts_with" => left
            .as_str()
            .map(|s| s.starts_with(right.as_str().unwrap_or_default()))
            .unwrap_or(false),
        "ends_with" => left
            .as_str()
            .map(|s| s.ends_with(right.as_str().unwrap_or_default()))
            .unwrap_or(false),
        "and" | "or" | "not" => {
            return Err(AppError::BadRequest(format!(
                "组合条件 '{op}' 待 P1.7 递归支持"
            )));
        }
        _ => num_cmp(op).unwrap_or(false),
    };
    Ok(r)
}

fn equalish(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct StubExec;
    #[async_trait]
    impl NodeExecutor for StubExec {
        async fn exec(&self, _node: &GraphNode, _input: Value) -> AppResult<ExecOutcome> {
            Ok(ExecOutcome {
                output: json!({"stub": true}),
            })
        }
    }

    /// Executor that fails the first `fail_until` calls, then succeeds.
    struct FlakyExec {
        calls: std::sync::atomic::AtomicUsize,
        fail_until: usize,
    }
    #[async_trait]
    impl NodeExecutor for FlakyExec {
        async fn exec(&self, _node: &GraphNode, _input: Value) -> AppResult<ExecOutcome> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if n < self.fail_until {
                Err(AppError::BadRequest("flaky boom".into()))
            } else {
                Ok(ExecOutcome {
                    output: json!({"stub": true}),
                })
            }
        }
    }

    fn graph_of(def: Value) -> Graph {
        super::super::graph::load_definition(&def).unwrap()
    }
    fn def(nodes: Value, edges: Value) -> Value {
        json!({"name":"t","graph":{"nodes":nodes,"edges":edges}})
    }

    fn node(id: &str, kind: &str, config: Value) -> Value {
        json!({"id": id, "data": {"type": kind, "config": config}})
    }
    fn edge(s: &str, h: &str, t: &str) -> Value {
        json!({"source": s, "sourceHandle": h, "target": t})
    }

    #[tokio::test]
    async fn linear_end_outputs_resolved() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node(
                    "end",
                    "end",
                    json!({"outputs": [{"name": "answer", "value": {"ref": ["start", "msg"]}}]})
                )
            ]),
            json!([edge("start", "out", "end")]),
        ));
        let mut snap = Snapshot::new();
        let mut start_input = HashMap::new();
        start_input.insert("msg".into(), json!("hi"));
        snap.pool.insert("start".into(), start_input);
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.outputs.unwrap()["answer"], "hi");
        assert_eq!(snap.node_states["end"].status, N_SUCCESS);
    }

    #[tokio::test]
    async fn branch_false_skips_true_path() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node(
                    "br",
                    "branch",
                    json!({
                        "branches": [{"id": "b1", "when": {"op": "==", "var": ["start", "msg"], "value": "hi"}, "handle": "true"}],
                        "else_handle": "false"
                    })
                ),
                node("na", "end", json!({"outputs": []})),
                node("nb", "end", json!({"outputs": []}))
            ]),
            json!([
                edge("start", "out", "br"),
                edge("br", "true", "na"),
                edge("br", "false", "nb")
            ]),
        ));
        let mut snap = Snapshot::new();
        let mut start_input = HashMap::new();
        start_input.insert("msg".into(), json!("bye"));
        snap.pool.insert("start".into(), start_input);
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.node_states["na"].status, N_SKIPPED);
        assert_eq!(snap.node_states["nb"].status, N_SUCCESS);
    }

    #[tokio::test]
    async fn branch_expr_string_when() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node(
                    "br",
                    "branch",
                    json!({
                        "branches": [{"id": "b1", "when": "{{#start.msg#}} == \"hi\"", "handle": "true"}],
                        "else_handle": "false"
                    })
                ),
                node("na", "end", json!({"outputs": []})),
                node("nb", "end", json!({"outputs": []}))
            ]),
            json!([
                edge("start", "out", "br"),
                edge("br", "true", "na"),
                edge("br", "false", "nb")
            ]),
        ));
        let mut snap = Snapshot::new();
        let mut start_input = HashMap::new();
        start_input.insert("msg".into(), json!("hi"));
        snap.pool.insert("start".into(), start_input);
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.node_states["na"].status, N_SUCCESS);
        assert_eq!(snap.node_states["nb"].status, N_SKIPPED);
    }

    #[tokio::test]
    async fn join_waits_for_both_branches() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node("e1", "egress", json!({"client_key": "k", "op": "o"})),
                node("e2", "egress", json!({"client_key": "k", "op": "o"})),
                node(
                    "end",
                    "end",
                    json!({"outputs": [{"name": "v", "value": {"ref": ["e2", "output"]}}]})
                )
            ]),
            json!([
                edge("start", "out", "e1"),
                edge("start", "out", "e2"),
                edge("e1", "out", "end"),
                edge("e2", "out", "end")
            ]),
        ));
        let mut snap = Snapshot::new();
        snap.pool.insert("start".into(), HashMap::new());
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.node_states["e1"].status, N_SUCCESS);
        assert_eq!(snap.node_states["e2"].status, N_SUCCESS);
        assert_eq!(snap.outputs.unwrap()["v"]["stub"], true);
    }

    fn node_m(id: &str, kind: &str, config: Value, mods: Value) -> Value {
        json!({"id": id, "data": {"type": kind, "config": config, "modifiers": mods}})
    }

    #[tokio::test]
    async fn retry_recovers_after_failure() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node_m(
                    "e1",
                    "egress",
                    json!({"client_key": "k", "op": "o"}),
                    json!({"retry": {"attempts": 3}})
                ),
                node("end", "end", json!({}))
            ]),
            json!([edge("start", "out", "e1"), edge("e1", "out", "end")]),
        ));
        let mut snap = Snapshot::new();
        snap.pool.insert("start".into(), HashMap::new());
        let flaky = FlakyExec {
            calls: Default::default(),
            fail_until: 1,
        };
        run(&g, &mut snap, &flaky).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.node_states["e1"].status, N_SUCCESS);
        assert_eq!(snap.node_states["e1"].attempt, 2, "one fail + one success");
    }

    #[tokio::test]
    async fn continue_on_error_passes_through() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node_m(
                    "e1",
                    "egress",
                    json!({"client_key": "k", "op": "o"}),
                    json!({"continue_on_error": true})
                ),
                node("end", "end", json!({"outputs": []}))
            ]),
            json!([edge("start", "out", "e1"), edge("e1", "out", "end")]),
        ));
        let mut snap = Snapshot::new();
        snap.pool.insert("start".into(), HashMap::new());
        let always_fail = FlakyExec {
            calls: Default::default(),
            fail_until: usize::MAX,
        };
        run(&g, &mut snap, &always_fail).await.unwrap();
        assert_eq!(
            snap.status, S_SUCCESS,
            "run passes through on continue_on_error"
        );
        assert_eq!(snap.node_states["e1"].status, N_FAILED);
        assert_eq!(snap.node_states["end"].status, N_SUCCESS);
    }

    #[tokio::test]
    async fn await_parks_then_resume_continues() {
        let g = graph_of(def(
            json!([
                node("start", "start", json!({})),
                node("gate", "await", json!({"kind": "human"})),
                node(
                    "end",
                    "end",
                    json!({
                        "outputs": [{"name": "ok", "value": {"ref": ["gate", "resume", "approved"]}}]
                    })
                )
            ]),
            json!([edge("start", "out", "gate"), edge("gate", "out", "end")]),
        ));
        let mut snap = Snapshot::new();
        snap.pool.insert("start".into(), HashMap::new());
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_WAITING);
        assert_eq!(snap.node_states["gate"].status, N_WAITING);
        assert!(snap.waiting_nodes.contains(&"gate".to_string()));

        // Resume: complete the gate with an approval payload.
        resume_snapshot(&mut snap, Some(json!({"approved": true}))).unwrap();
        assert_eq!(snap.status, S_RUNNING);
        run(&g, &mut snap, &StubExec).await.unwrap();
        assert_eq!(snap.status, S_SUCCESS);
        assert_eq!(snap.node_states["gate"].status, N_SUCCESS);
        assert_eq!(snap.node_states["end"].status, N_SUCCESS);
        assert_eq!(snap.outputs.unwrap()["ok"], true);
    }
}
