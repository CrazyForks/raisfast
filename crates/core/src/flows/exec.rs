//! Production node executors wired to the integration plane + plugin engines.
//!
//! [`FlowsExec`] implements [`super::engine::NodeExecutor`]:
//! - `egress` → `IntegrationPlane.call_api` (OAuth2/signing/rate-limit/log/trace)
//! - `script` → `PluginManager::run_inline_script_value` — runs the node code as
//!   a one-shot inline **script** (not an installed plugin) on the JS engine,
//!   reusing the same sandboxed host (`host.callApi` gated by an egress
//!   allowlist derived from `host_permissions`). Zero state leakage: load →
//!   call → unload per invocation.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::app_error::{AppError, AppResult};
use crate::integration::IntegrationPlane;
use crate::plugins::{Permissions, PluginManager};

use super::engine::{ExecOutcome, NodeExecutor};
use super::graph::GraphNode;
use super::nodes::{self, EgressConfig, ScriptConfig};

pub struct FlowsExec {
    pub plane: Option<Arc<IntegrationPlane>>,
    pub plugins: Option<Arc<PluginManager>>,
}

impl FlowsExec {
    async fn run_script(&self, node: &GraphNode, input: Value) -> AppResult<ExecOutcome> {
        let cfg: ScriptConfig = serde_json::from_value(node.data.config.clone())
            .map_err(|e| AppError::BadRequest(format!("script config: {e}")))?;
        if cfg.language != "js" {
            return Err(AppError::BadRequest(format!(
                "script language '{}' 暂未接入（v1 支持 js）",
                cfg.language
            )));
        }
        let Some(plugins) = &self.plugins else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "plugin manager disabled"
            )));
        };

        // Flows are authored by admins only — the script runs with the full
        // platform host surface by default (egress clients, ct.*, raw db/sql,
        // raw http (SSRF filter still applies), session, presence, receipts).
        // `host_permissions` may narrow any allowlist further if desired.
        let star = vec!["*".to_string()];
        let mut perms = Permissions {
            egress: star.clone(),
            content_types: star.clone(),
            database: star.clone(),
            http: star.clone(),
            session: star.clone(),
            presence: star.clone(),
            integration: star.clone(),
            ..Permissions::default()
        };
        if let Some(hp) = &cfg.host_permissions {
            if let Some(list) = &hp.call_api {
                perms.egress = list.clone();
            }
            if let Some(list) = &hp.content_types {
                perms.content_types = list.clone();
            }
            if let Some(list) = &hp.database {
                perms.database = list.clone();
            }
            if let Some(list) = &hp.http {
                perms.http = list.clone();
            }
            if let Some(list) = &hp.session {
                perms.session = list.clone();
            }
            if let Some(list) = &hp.presence {
                perms.presence = list.clone();
            }
        }
        if let Some(sb) = &cfg.sandbox {
            perms.max_memory_mb = sb.memory_mb.filter(|&m| m > 0).map(|m| m as u32);
            perms.timeout_ms = sb.timeout_ms.filter(|&t| t > 0).map(|t| t as u64);
        }

        // Authoring contract (same as cron inline scripts): the code IS a full
        // exported function. It receives the input as a JSON string and may
        // return any value (engine stringifies it back).
        if cfg.plugin_id.is_some() {
            return Err(AppError::BadRequest(
                "script: plugin_id 引用未接入（v1 用 code）".into(),
            ));
        }
        if cfg.code.trim().is_empty() {
            return Err(AppError::BadRequest("script: code 为空".into()));
        }
        // Unique id per invocation: no KV bleed across concurrent instances.
        let id = format!("__wf__{}", crate::utils::id::new_snowflake_id());

        let out = plugins
            .run_inline_script_value("js", &id, &cfg.code, "main", &input, perms)
            .await?;
        Ok(ExecOutcome { output: out })
    }
}

#[async_trait]
impl NodeExecutor for FlowsExec {
    async fn exec(&self, node: &GraphNode, input: Value) -> AppResult<ExecOutcome> {
        match node.data.kind.as_str() {
            nodes::T_SCRIPT => self.run_script(node, input).await,
            nodes::T_EGRESS => {
                let cfg: EgressConfig = serde_json::from_value(node.data.config.clone())
                    .map_err(|e| AppError::BadRequest(format!("egress config: {e}")))?;
                let Some(plane) = &self.plane else {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "integration plane disabled"
                    )));
                };
                let receipt = plane
                    .call_api(&cfg.client_key, &cfg.op, input)
                    .await
                    .map_err(|e| {
                        AppError::Internal(anyhow::anyhow!(
                            "egress '{}'.{} failed: {e}",
                            cfg.client_key,
                            cfg.op
                        ))
                    })?;
                Ok(ExecOutcome {
                    output: receipt.output,
                })
            }
            other => Err(AppError::BadRequest(format!(
                "FlowsExec 不支持节点 '{other}'"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flows::graph::NodeData;
    use serde_json::json;

    fn node(kind: &str, config: Value) -> GraphNode {
        GraphNode {
            id: "n1".into(),
            data: NodeData {
                kind: kind.into(),
                version: 1,
                title: "".into(),
                desc: None,
                config,
                modifiers: Value::Null,
            },
        }
    }

    #[tokio::test]
    async fn egress_without_plane_errors_clearly() {
        let exec = FlowsExec {
            plane: None,
            plugins: None,
        };
        let err = exec
            .exec(
                &node(nodes::T_EGRESS, json!({"client_key": "k", "op": "o"})),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("integration plane disabled"));
    }

    #[tokio::test]
    async fn script_without_plugins_errors_clearly() {
        let exec = FlowsExec {
            plane: None,
            plugins: None,
        };
        let err = exec
            .exec(
                &node(
                    nodes::T_SCRIPT,
                    json!({"language": "js", "code": "return 1"}),
                ),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("plugin manager disabled"));
    }

    #[tokio::test]
    async fn script_non_js_rejected() {
        let exec = FlowsExec {
            plane: None,
            plugins: None,
        };
        let err = exec
            .exec(
                &node(nodes::T_SCRIPT, json!({"language": "rhai", "code": "1"})),
                json!({}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("暂未接入"));
    }
}
