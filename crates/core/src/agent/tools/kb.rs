//! Knowledge-base retrieval tool (`knowledge_search`) — the agent-side
//! consumption seam of the KB subsystem (kb-technical-design §10:
//! "Agent 工具化消费的入口").
//!
//! Behavior mirrors WeKnora's agent knowledge_search tool
//! [抄WK:internal/agent/tools/knowledge_search.go]:
//! - 1–5 short semantic queries per call (the agent formulates them);
//! - `kb_ids` can only **narrow** the pre-bound scope, never expand it
//!   (WeKnora `validateKnowledgeBaseIDsInSearchTargets` semantics);
//! - XML `<search_results>` output with per-unit ids for citations;
//! - empty results return anti-fabrication guidance instead of nothing;
//! - units already returned earlier in the same turn render compactly
//!   (`already_seen="true"`, content omitted) so repeat calls don't burn
//!   tokens (WeKnora seenChunks).
//!
//! Retrieval goes through `kb::pipeline::search_units` — S2–S7 without S1
//! (query rewriting is the agent's job) and without S9 generation (the
//! agent's own turn is the generator).

use async_trait::async_trait;
use raisfast_agent::tool::ToolExecution;
use raisfast_agent::{Tool, ToolRegistry};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use crate::AppState;
use crate::agent::models::ai_agent::AiAgent;
use crate::constants::DEFAULT_TENANT;
use crate::kb::pipeline::{self, ContextUnit};
use crate::kb::service::KbDeps;
use crate::middleware::auth::AuthUser;

/// Max queries per call [抄WK schema maxItems=5].
const MAX_QUERIES: usize = 5;
/// Default / max kept units per call (output-size guard: agent context
/// windowing exists, but a single tool result must stay bounded).
const DEFAULT_TOP_K: usize = 5;
const MAX_TOP_K: usize = 10;
/// Per-unit content cap in chars [自造+理由: WK emits full chunk content;
/// our parent-expanded units can exceed it — cap with an explicit marker].
const CONTENT_CAP: usize = 4000;

/// Register the `knowledge_search` tool when the KB subsystem is enabled
/// and the agent has a valid KB scope. Scope resolution is fail-closed:
/// any error (KB disabled, invalid `params.kb_ids`, no active KBs) skips
/// registration with a warning instead of failing the turn.
/// Register the `knowledge_search` tool when the KB subsystem is enabled
/// and the agent carries an **explicit** KB binding (`params.kb_ids`).
/// Binding is select-only: no binding (or an empty/invalid list) means no
/// tool — there is deliberately no "default all tenant KBs" fallback
/// (least privilege; mounting must be an explicit per-KB choice).
pub async fn register(
    registry: &mut ToolRegistry,
    state: &AppState,
    auth: &AuthUser,
    agent: &AiAgent,
) {
    if state.kb_runtime.is_none() {
        return; // KB subsystem disabled — tool simply absent.
    }
    let Ok(deps) = state.kb_deps() else {
        return;
    };
    let tenant = auth.tenant_id().unwrap_or(DEFAULT_TENANT).to_string();
    let requested = match bound_kb_ids(agent) {
        Ok(Some(ids)) if !ids.is_empty() => ids,
        Ok(_) => return, // unbound → tool not mounted (explicit-only)
        Err(e) => {
            tracing::warn!(
                agent = agent.id.0,
                error = %e,
                "knowledge_search: invalid params.kb_ids, tool skipped"
            );
            return;
        }
    };
    let scope = match pipeline::resolve_kbs(&deps, &requested, &tenant).await {
        Ok(scope) => scope,
        Err(e) => {
            tracing::warn!(
                agent = agent.id.0,
                error = %e,
                "knowledge_search: bound kb ids failed tenant/active validation, tool skipped"
            );
            return;
        }
    };
    registry.register(KnowledgeSearchTool {
        deps,
        tenant,
        scope,
        seen: Mutex::new(HashSet::new()),
    });
}

/// Catalog registration for the admin tool listing (`GET /admin/ai/tools`):
/// exposes the `knowledge_search` spec whenever the KB subsystem is enabled.
/// Mounting is a separate per-agent binding, so no scope is bound here and
/// this instance is never executed — only its name/description are read.
pub fn register_catalog(registry: &mut ToolRegistry, state: &AppState) {
    if let Ok(deps) = state.kb_deps() {
        registry.register(KnowledgeSearchTool {
            deps,
            tenant: String::new(),
            scope: Vec::new(),
            seen: Mutex::new(HashSet::new()),
        });
    }
}

/// Parse `ai_agents.params.kb_ids` — `Some(ids)` = explicit binding,
/// `None` = unbound (tool stays unmounted). Accepts JSON numbers or
/// numeric strings (admin forms may submit strings); any garbage entry
/// rejects the whole list (fail-closed).
fn bound_kb_ids(agent: &AiAgent) -> Result<Option<Vec<i64>>, String> {
    let Some(params) = agent.params.as_ref() else {
        return Ok(None);
    };
    let Some(raw) = params.get("kb_ids") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Value::Array(items) = raw else {
        return Err("params.kb_ids must be an array of kb ids".into());
    };
    let mut ids = Vec::with_capacity(items.len());
    for item in items {
        let id = match item {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse::<i64>().ok(),
            _ => None,
        };
        ids.push(id.ok_or("params.kb_ids contains a non-id entry")?);
    }
    Ok(Some(ids))
}

/// The `knowledge_search` tool instance. One per turn (built by
/// `build_domain_tools`), so the seen-set dedups repeat calls within a
/// single agent turn, matching WeKnora's per-session instance semantics
/// at our turn granularity.
struct KnowledgeSearchTool {
    deps: KbDeps,
    tenant: String,
    /// Tenant-validated KB ids bound at registration (search targets).
    scope: Vec<i64>,
    /// Unit ids already returned earlier this turn.
    seen: Mutex<HashSet<i64>>,
}

#[async_trait]
impl Tool for KnowledgeSearchTool {
    fn name(&self) -> &str {
        "knowledge_search"
    }

    fn description(&self) -> &str {
        "Semantic search over knowledge bases. Retrieves chunks by meaning, \
intent, and conceptual relevance.\n\
Use for: conceptual explanations, topic overviews, how/why questions, \
definitions, comparisons.\n\
Do NOT use for: exact keyword or entity lookup, error-code search.\n\
Input: 1-5 short, well-formed semantic questions (not keyword lists, not \
raw user text). Optionally narrow the KB scope with kb_ids, and set top_k \
(1-10, default 5).\n\
Output: XML <search_results> with ranked units (unit_id/title/score/content). \
Cite unit_id when using retrieved content. When nothing is retrieved, state \
that the knowledge base does not cover the question — never fabricate."
    }

    fn category(&self) -> &'static str {
        "kb"
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "queries": {
                    "type": "array",
                    "description": "REQUIRED: 1-5 semantic questions/topics (e.g. [\"What is RAG?\", \"RAG benefits\"])",
                    "items": { "type": "string" },
                    "minItems": 1,
                    "maxItems": 5
                },
                "kb_ids": {
                    "type": "array",
                    "description": "Optional: narrow the search to these knowledge-base ids (must be within the bound scope)",
                    "items": { "type": "integer" }
                },
                "top_k": {
                    "type": "integer",
                    "description": "Max units to return (1-10, default 5)",
                    "minimum": 1,
                    "maximum": 10
                }
            },
            "required": ["queries"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let queries = parse_queries(&args)?;
        if queries.is_empty() {
            return Err("queries must contain 1-5 non-empty strings".into());
        }

        // kb_ids may only narrow the bound scope [抄WK narrowing semantics].
        let effective_scope: Vec<i64> = match args.get("kb_ids") {
            None | Some(Value::Null) => self.scope.clone(),
            Some(Value::Array(items)) => {
                let mut narrowed = Vec::with_capacity(items.len());
                for item in items {
                    let Some(id) = item.as_i64() else {
                        return Err("kb_ids entries must be integers".into());
                    };
                    if !self.scope.contains(&id) {
                        return Err(format!(
                            "kb_id {id} is outside this agent's bound knowledge-base scope"
                        ));
                    }
                    narrowed.push(id);
                }
                if narrowed.is_empty() {
                    self.scope.clone()
                } else {
                    narrowed
                }
            }
            Some(_) => return Err("kb_ids must be an array of integers".into()),
        };

        let top_k = args
            .get("top_k")
            .and_then(Value::as_i64)
            .map_or(DEFAULT_TOP_K, |k| k.clamp(1, MAX_TOP_K as i64) as usize);

        // Per-query retrieval (S2–S7, no S1 rewrite, no generation), merged
        // across queries keeping each unit's best score and first source.
        let mut merged: HashMap<i64, (f32, String, ContextUnit)> = HashMap::new();
        for q in &queries {
            let (_top, units) =
                pipeline::search_units(&self.deps, &self.tenant, &effective_scope, q)
                    .await
                    .map_err(|e| format!("knowledge_search failed: {e}"))?;
            for u in units {
                match merged.get(&u.unit_id) {
                    Some((best, _, _)) if *best >= u.score => {}
                    _ => {
                        merged.insert(u.unit_id, (u.score, q.clone(), u));
                    }
                }
            }
        }
        let mut ranked: Vec<(f32, String, ContextUnit)> = merged.into_values().collect();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(top_k);

        if ranked.is_empty() {
            // [抄WK formatOutput empty-result guidance] — anti-fabrication.
            return Ok("No relevant content found in the knowledge base.\n\
                 - DO NOT answer from your own knowledge or invent facts.\n\
                 - Tell the user the knowledge base does not cover this question."
                .to_string());
        }

        let mut out = String::new();
        out.push_str(&format!("<search_results count=\"{}\">\n", ranked.len()));
        for q in &queries {
            out.push_str(&format!("<query>{}</query>\n", xml_escape(q)));
        }
        // Mark all outgoing units as seen (compact on repeat calls).
        let previously_seen: HashSet<i64> = {
            let mut seen = self.seen.lock().unwrap_or_else(|p| p.into_inner());
            let prev: HashSet<i64> = ranked
                .iter()
                .map(|(_, _, u)| u.unit_id)
                .filter(|id| seen.contains(id))
                .collect();
            for (_, _, u) in &ranked {
                seen.insert(u.unit_id);
            }
            prev
        };
        for (i, (score, source, u)) in ranked.iter().enumerate() {
            let seen_attr = if previously_seen.contains(&u.unit_id) {
                " already_seen=\"true\""
            } else {
                ""
            };
            out.push_str(&format!(
                "<unit rank=\"{}\" unit_id=\"{}\" kind=\"{}\" title=\"{}\" score=\"{:.3}\" source_query=\"{}\"{}>\n",
                i + 1,
                u.unit_id,
                xml_escape(&u.kind),
                xml_escape(&u.title),
                score,
                xml_escape(source),
                seen_attr
            ));
            if previously_seen.contains(&u.unit_id) {
                out.push_str(
                    "<note>(content omitted, already returned in an earlier knowledge_search call this turn)</note>\n",
                );
            } else {
                let content = truncate_chars(&u.content, CONTENT_CAP);
                out.push_str(&format!("<content>{}</content>\n", xml_escape(content)));
            }
            out.push_str("</unit>\n");
        }
        out.push_str("</search_results>");
        Ok(out)
    }
}

/// Parse the `queries` arg: non-empty strings, deduped, capped at MAX_QUERIES.
fn parse_queries(args: &Value) -> Result<Vec<String>, String> {
    let Some(Value::Array(items)) = args.get("queries") else {
        return Err("queries (array of 1-5 strings) is required".into());
    };
    let mut queries: Vec<String> = items
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
        .collect();
    queries.dedup();
    queries.truncate(MAX_QUERIES);
    Ok(queries)
}

/// XML-escape attribute values and text nodes [抄WK xmlEscape].
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Char-boundary-safe truncation with an explicit marker.
fn truncate_chars(s: &str, cap: usize) -> &str {
    if s.chars().count() <= cap {
        return s;
    }
    let end = s.char_indices().nth(cap).map_or(s.len(), |(i, _)| i);
    &s[..end]
}

// ─────────────────────────── tests ────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::app_error::AppResult;
    use std::sync::Arc;

    struct MockEmbedder;

    #[async_trait::async_trait]
    impl crate::kb::service::KbEmbedder for MockEmbedder {
        async fn embed(&self, texts: &[&str]) -> AppResult<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0_f32; 4];
                    let seed = t.bytes().map(|b| b as usize).sum::<usize>();
                    v[seed % 4] = 1.0;
                    v
                })
                .collect())
        }
    }

    async fn deps() -> KbDeps {
        let pool = crate::test_pool!();
        let mut config = crate::config::app::AppConfig::test_defaults();
        config.kb.enabled = true;
        let bus = crate::eventbus::EventBus::new(16);
        KbDeps {
            pool,
            config: Arc::new(config),
            storage: Arc::new(
                crate::storage::local::LocalStorage::new("/tmp/kb-tool-test", "/uploads").unwrap(),
            ),
            vector: Arc::new(crate::kb::vectors::BruteForceIndex::new()),
            kbsearch: Arc::new(crate::kb::kbsearch::KbSearchEngine::open_in_memory().unwrap()),
            embedder: Arc::new(MockEmbedder),
            provider: None, // search_units must never need a chat provider
            emitter: crate::event::EventEmitter::eventbus_only(bus),
        }
    }

    async fn seeded_kb(deps: &KbDeps, tenant: &str, slug: &str, body: &str) -> i64 {
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: slug.into(),
                description: None,
                slug: slug.into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            tenant,
        )
        .await
        .unwrap();
        let mut markdown = "# 文档\n\n".to_string();
        markdown.push_str(&body.repeat(60));
        let doc =
            crate::kb::service::create_online_document(deps, kb.id, slug, &markdown, None, tenant)
                .await
                .unwrap();
        crate::kb::service::process_document(deps, doc.id, tenant)
            .await
            .unwrap();
        i64::from(kb.id)
    }

    fn tool(deps: KbDeps, tenant: &str, scope: Vec<i64>) -> KnowledgeSearchTool {
        KnowledgeSearchTool {
            deps,
            tenant: tenant.to_string(),
            scope,
            seen: Mutex::new(HashSet::new()),
        }
    }

    #[tokio::test]
    async fn returns_xml_results() {
        let deps = deps().await;
        let kb_id = seeded_kb(
            &deps,
            "default",
            "db-doc",
            "raisfast 支持 SQLite PostgreSQL MySQL 数据库后端。",
        )
        .await;
        let t = tool(deps, "default", vec![kb_id]);
        let out = t
            .execute(serde_json::json!({ "queries": ["支持哪些数据库？"] }))
            .await
            .unwrap();
        assert!(out.contains("<search_results"), "XML envelope: {out}");
        assert!(out.contains("unit_id="), "units present: {out}");
        assert!(out.contains("PostgreSQL"), "content present: {out}");
    }

    #[tokio::test]
    async fn empty_result_returns_antifabrication_guidance() {
        let deps = deps().await;
        // KB with no documents: recall is empty (with mock one-hot
        // embeddings any seeded chunk would match — same premise as the
        // pipeline `uncovered` test).
        let kb = crate::kb::models::knowledge_base::create_kb(
            &deps.pool,
            &crate::kb::models::knowledge_base::CreateKbCmd {
                name: "empty".into(),
                description: None,
                slug: "empty".into(),
                kind: "document".into(),
                indexing_strategy: None,
                embedding_model: Some("m".into()),
                embedding_dim: Some(4),
            },
            "default",
        )
        .await
        .unwrap();
        let t = tool(deps, "default", vec![i64::from(kb.id)]);
        let out = t
            .execute(serde_json::json!({ "queries": ["量子力学诠释"] }))
            .await
            .unwrap();
        assert!(out.to_lowercase().contains("do not"), "guidance: {out}");
        assert!(!out.contains("<search_results"));
    }

    #[tokio::test]
    async fn kb_ids_can_only_narrow_bound_scope() {
        let deps = deps().await;
        let kb_id = seeded_kb(&deps, "default", "scope", "内容内容内容。").await;
        let t = tool(deps, "default", vec![kb_id]);
        let err = t
            .execute(serde_json::json!({
                "queries": ["任何问题"],
                "kb_ids": [987654321i64]
            }))
            .await
            .unwrap_err();
        assert!(err.contains("outside"), "narrow-only: {err}");
    }

    #[tokio::test]
    async fn cross_tenant_scope_rejected_on_execute_path() {
        let deps = deps().await;
        let kb_id = seeded_kb(&deps, "tenant-a", "island", "租户 A 的私有内容。").await;
        // Even if a stale/misbound scope leaked a foreign kb id in, the
        // execute path re-validates against the tenant.
        let t = tool(deps, "tenant-b", vec![kb_id]);
        let err = t
            .execute(serde_json::json!({ "queries": ["私有内容"] }))
            .await
            .unwrap_err();
        assert!(err.contains("failed"), "tenant validation: {err}");
    }

    #[tokio::test]
    async fn repeat_call_renders_seen_units_compactly() {
        let deps = deps().await;
        let kb_id = seeded_kb(
            &deps,
            "default",
            "seen",
            "raisfast 知识库检索支持混合召回。",
        )
        .await;
        let t = tool(deps, "default", vec![kb_id]);
        let q = serde_json::json!({ "queries": ["混合召回是什么"] });
        let first = t.execute(q.clone()).await.unwrap();
        assert!(!first.contains("already_seen"), "first call full: {first}");
        let second = t.execute(q).await.unwrap();
        assert!(
            second.contains("already_seen=\"true\""),
            "second call compact: {second}"
        );
        assert!(
            !second.contains("<content>"),
            "seen unit omits content: {second}"
        );
    }

    #[test]
    fn bound_kb_ids_parses_params() {
        let agent = AiAgent {
            id: crate::types::snowflake_id::SnowflakeId(1),
            tenant_id: Some("default".into()),
            user_id: None,
            name: "a".into(),
            system_prompt: String::new(),
            provider: "openai".into(),
            model: "m".into(),
            temperature: None,
            max_iterations: 10,
            tools: serde_json::json!(["*"]),
            memory_enabled: false,
            params: Some(serde_json::json!({ "kb_ids": ["123", 456] })),
            created_at: crate::utils::tz::now_utc(),
            updated_at: crate::utils::tz::now_utc(),
        };
        assert_eq!(bound_kb_ids(&agent).unwrap(), Some(vec![123, 456]));
        let unbound = AiAgent {
            params: None,
            ..agent
        };
        assert_eq!(bound_kb_ids(&unbound).unwrap(), None);
    }
}
