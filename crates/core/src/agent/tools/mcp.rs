//! MCP external-tool adapter: discover configured MCP servers (stdio or
//! streamable-HTTP) and expose each tool as a `raisfast Tool` (composed name
//! `mcp__{server}__{tool}`). One shared session per server per turn; failed
//! calls reconnect once. Registered per turn from admin config
//! (`RAISFAST_AI_MCP_SERVERS`) and gated by the `ai_agents.tools` allowlist.

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::mcp_client::{McpHttpSession, McpServerConfig, McpSession, McpTransport};

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(60)
        .collect()
}

fn description_with_server(tool_description: Option<String>, server: &str) -> String {
    let hint = format!(" (via MCP server '{server}')");
    let d = tool_description.unwrap_or_default().trim().to_string();
    if d.is_empty() {
        format!("External MCP tool from server '{server}'{hint}")
    } else {
        format!("{d}{hint}")
    }
}

const MAX_CACHED_SERVERS: usize = 16;

static STDIO_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<Mutex<McpSession>>>>,
> = std::sync::OnceLock::new();
static HTTP_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, Arc<Mutex<McpHttpSession>>>>,
> = std::sync::OnceLock::new();

fn stdio_key(cfg: &McpServerConfig) -> String {
    format!("{}:{}/{}", cfg.name, cfg.command, cfg.args.join(" "))
}

async fn stdio_session_cached(cfg: &McpServerConfig) -> Result<Arc<Mutex<McpSession>>, String> {
    let map = STDIO_CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let key = stdio_key(cfg);
    if let Some(existing) = map.lock().unwrap().get(&key).cloned() {
        return Ok(existing);
    }
    let session = McpSession::connect(cfg)
        .await
        .map_err(|e| format!("connect {}: {e}", cfg.name))?;
    let arc = Arc::new(Mutex::new(session));
    let mut guard = map.lock().unwrap();
    guard.insert(key, Arc::clone(&arc));
    if guard.len() > MAX_CACHED_SERVERS
        && let Some(k) = guard.keys().next().cloned()
    {
        guard.remove(&k);
    }
    Ok(arc)
}

async fn http_session_cached(cfg: &McpServerConfig) -> Result<Arc<Mutex<McpHttpSession>>, String> {
    let url = cfg
        .url
        .as_deref()
        .ok_or_else(|| "http mcp config missing url".to_string())?;
    let map = HTTP_CACHE.get_or_init(|| std::sync::Mutex::new(Default::default()));
    let key = format!("{}:{}", cfg.name, url);
    if let Some(existing) = map.lock().unwrap().get(&key).cloned() {
        return Ok(existing);
    }
    let mut session = McpHttpSession::new(cfg);
    session
        .initialize()
        .await
        .map_err(|e| format!("initialize {}: {e}", cfg.name))?;
    let arc = Arc::new(Mutex::new(session));
    let mut guard = map.lock().unwrap();
    guard.insert(key, Arc::clone(&arc));
    if guard.len() > MAX_CACHED_SERVERS
        && let Some(k) = guard.keys().next().cloned()
    {
        guard.remove(&k);
    }
    Ok(arc)
}

enum Session {
    Stdio(Arc<Mutex<McpSession>>),
    Http(Arc<Mutex<McpHttpSession>>),
}

pub struct McpTool {
    name: String,
    description: String,
    schema: Value,
    cfg: McpServerConfig,
    tool_name: String,
    session: Session,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn category(&self) -> &'static str {
        "mcp"
    }
    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let call = match &self.session {
            Session::Stdio(s) => {
                s.lock()
                    .await
                    .call_tool(&self.tool_name, args.clone())
                    .await
            }
            Session::Http(s) => {
                s.lock()
                    .await
                    .call_tool(&self.tool_name, args.clone())
                    .await
            }
        };
        match call {
            Ok(out) => Ok(out),
            Err(e) => {
                tracing::warn!(server = %self.cfg.name, error = %e, "mcp call failed; reconnecting once");
                match &self.session {
                    Session::Stdio(s) => {
                        let fresh = McpSession::connect(&self.cfg)
                            .await
                            .map_err(|ce| format!("mcp reconnect {}: {ce}", self.cfg.name))?;
                        *s.lock().await = fresh;
                        s.lock()
                            .await
                            .call_tool(&self.tool_name, args)
                            .await
                            .map_err(|ce| format!("mcp call {}: {ce}", self.cfg.name))
                    }
                    Session::Http(s) => {
                        let mut fresh = McpHttpSession::new(&self.cfg);
                        fresh
                            .initialize()
                            .await
                            .map_err(|ie| format!("mcp http reconnect {}: {ie}", self.cfg.name))?;
                        *s.lock().await = fresh;
                        s.lock()
                            .await
                            .call_tool(&self.tool_name, args)
                            .await
                            .map_err(|ce| format!("mcp call {}: {ce}", self.cfg.name))
                    }
                }
            }
        }
    }
}

/// Catalog metadata cache for `GET /admin/ai/tools`: MCP tool specs are
/// the only live-derived part of the catalog, so they are cached with a
/// TTL instead of re-handshaking on every request. On refresh failure the
/// last-known list keeps being served (stale beats blocking); the refresh
/// attempt itself is capped at [`CATALOG_ATTEMPT`] so a hanging server
/// can only ever delay one request per TTL by that much.
const CATALOG_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const CATALOG_ATTEMPT: std::time::Duration = std::time::Duration::from_secs(3);

/// `(fetched_at, specs)` pairs keyed fresh for one TTL.
type CatalogCache = Option<(std::time::Instant, Arc<Vec<(String, String)>>)>;

static CATALOG_SPECS: std::sync::OnceLock<std::sync::Mutex<CatalogCache>> =
    std::sync::OnceLock::new();

/// Cached `(name, description)` list of all MCP tools across servers.
/// Server-side tool changes surface within one TTL; server config changes
/// require a restart (env-derived, same as the turn path).
pub async fn cached_catalog_specs(servers: &[Value]) -> Arc<Vec<(String, String)>> {
    let cell = CATALOG_SPECS.get_or_init(|| std::sync::Mutex::new(None));
    if let Some((at, specs)) = cell.lock().unwrap_or_else(|p| p.into_inner()).clone()
        && at.elapsed() < CATALOG_TTL
    {
        return specs;
    }
    // Stale or absent: try one bounded refresh; on failure keep serving
    // whatever we had (possibly none).
    let refreshed = tokio::time::timeout(CATALOG_ATTEMPT, async {
        let mut registry = raisfast_agent::ToolRegistry::new();
        register_mcp_tools(&mut registry, servers).await;
        registry
            .specs()
            .into_iter()
            .map(|s| (s.name, s.description))
            .collect::<Vec<_>>()
    })
    .await;
    let mut guard = cell.lock().unwrap_or_else(|p| p.into_inner());
    match refreshed {
        Ok(list) => {
            let specs = Arc::new(list);
            *guard = Some((std::time::Instant::now(), Arc::clone(&specs)));
            specs
        }
        Err(_) => {
            // Attempt timed out (dead/hanging server): keep the previous
            // entry but bump its timestamp so we do not retry on every
            // request (negative cache until the next TTL).
            if let Some((_at, specs)) = guard.as_ref() {
                let cloned = Arc::clone(specs);
                *guard = Some((std::time::Instant::now(), Arc::clone(&cloned)));
                cloned
            } else {
                Arc::new(Vec::new())
            }
        }
    }
}

/// Discover and register all tools of every configured server.
pub async fn register_mcp_tools(
    registry: &mut raisfast_agent::ToolRegistry,
    servers: &[serde_json::Value],
) {
    for raw in servers {
        let cfg: McpServerConfig = match serde_json::from_value(raw.clone()) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "invalid MCP server config ignored");
                continue;
            }
        };
        match cfg.transport {
            McpTransport::Stdio => register_stdio(registry, cfg).await,
            McpTransport::Http => register_http(registry, cfg).await,
            McpTransport::Sse => {
                tracing::warn!(server = %cfg.name, "MCP SSE transport not implemented; skipped");
            }
        }
    }
}

async fn register_stdio(registry: &mut raisfast_agent::ToolRegistry, cfg: McpServerConfig) {
    let shared = match stdio_session_cached(&cfg).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(server = %cfg.name, error = %e, "stdio MCP server unavailable, skipped");
            return;
        }
    };
    let tools = match shared.lock().await.list_tools().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(server = %cfg.name, error = %e, "stdio tools/list failed, skipped");
            return;
        }
    };
    tracing::info!(server = %cfg.name, count = tools.len(), "registered stdio MCP tools");
    for def in tools {
        registry.register(McpTool {
            name: format!("mcp__{}__{}", sanitize(&cfg.name), sanitize(&def.name)),
            description: description_with_server(def.description, &cfg.name),
            schema: def.input_schema,
            cfg: cfg.clone(),
            tool_name: def.name,
            session: Session::Stdio(Arc::clone(&shared)),
        });
    }
}

async fn register_http(registry: &mut raisfast_agent::ToolRegistry, cfg: McpServerConfig) {
    if cfg.url.is_none() {
        tracing::warn!(server = %cfg.name, "http MCP server missing url; skipped");
        return;
    }
    let shared = match http_session_cached(&cfg).await {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(server = %cfg.name, error = %e, "http MCP server unavailable, skipped");
            return;
        }
    };
    let tools = match shared.lock().await.list_tools().await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(server = %cfg.name, error = %e, "http tools/list failed, skipped");
            return;
        }
    };
    tracing::info!(server = %cfg.name, count = tools.len(), "registered http MCP tools");
    for def in tools {
        registry.register(McpTool {
            name: format!("mcp__{}__{}", sanitize(&cfg.name), sanitize(&def.name)),
            description: description_with_server(def.description, &cfg.name),
            schema: def.input_schema,
            cfg: cfg.clone(),
            tool_name: def.name,
            session: Session::Http(Arc::clone(&shared)),
        });
    }
}
