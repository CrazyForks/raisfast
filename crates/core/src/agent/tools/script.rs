//! Sandboxed code-execution tools: `run_js` / `run_lua` / `run_rhai`.
//!
//! Reuses the platform plugin sandboxes (`PluginManager::run_inline_script_value`):
//! zero state between invocations, engine timeouts/instruction budgets enforced,
//! host/VFS/network capabilities denied by default (`Permissions::default()`).
//! Results return to the model as a normal tool row (audit + truncation cap).

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;
use std::sync::Arc;

use crate::plugins::{Permissions, PluginManager};

pub struct RunCodeTool {
    name: String,
    description: String,
    runtime: &'static str,
    plugins: Arc<PluginManager>,
}

impl RunCodeTool {
    pub fn new(runtime: &'static str, plugins: Arc<PluginManager>) -> Self {
        let name = format!("run_{runtime}");
        let guidance = match runtime {
            "lua" => {
                "export a `main` on the `Plugin` table: `Plugin = { main = function(input) return { ... } end }`. \
                 `input` is a Lua table of the args object; return a table/string/number."
            }
            "rhai" => {
                "define a top-level function `fn main(input) { ... }`. `input` is a map of the args object; \
                 return a value (number/string/map/array)."
            }
            _ => {
                "write an ESM script that exports `export function main(__in) { ... }`. `__in` is the args object \
                 as a JSON **string** (`JSON.parse(__in)`); return a JSON-serializable value."
            }
        };
        Self {
            name,
            description: format!(
                "Runs user-provided code inside a sandboxed {} interpreter and returns its JSON value. \
                 Use for computation, string/JSON transformation or generation. {guidance}",
                runtime
            ),
            runtime,
            plugins,
        }
    }
}

#[async_trait]
impl Tool for RunCodeTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": { "type": "string", "description": "source code to run in the sandbox" },
                "args": {
                    "type": "object",
                    "description": "optional input passed to the script's main function",
                    "additionalProperties": true
                },
                "max_output_chars": {
                    "type": "integer",
                    "default": 12000,
                    "description": "hard guard on how many chars are returned; raise only when you truly need the whole output"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let Some(code) = args.get("code").and_then(Value::as_str) else {
            return Err("code (string) is required".to_string());
        };
        let input = args
            .get("args")
            .cloned()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| Value::Object(Default::default()));
        let max_chars =
            args.get("max_output_chars")
                .and_then(Value::as_u64)
                .unwrap_or(DEFAULT_MAX_OUTPUT_CHARS as u64)
                .clamp(MIN_OUTPUT_CHARS as u64, MAX_OUTPUT_CHARS_LIMIT as u64) as usize;

        // Unique engine id per invocation: load/call/unload is per call and the
        // engines are sharded by id, so no state leaks across tool calls.
        let id = format!("__ai_code_{}", crate::utils::id::new_id());
        let out = self
            .plugins
            .run_inline_script_value(
                self.runtime,
                &id,
                code,
                "main",
                &input,
                Permissions::default(),
            )
            .await
            .map_err(|e| format!("{} sandbox error: {e}", self.runtime))?;

        if out.is_null() {
            return Ok(format!(
                "(script returned no value — export `main` and return a value; see the tool description for the {} contract)",
                self.runtime
            ));
        }

        let text = if let Value::String(s) = &out {
            s.clone()
        } else {
            serde_json::to_string_pretty(&out).unwrap_or_else(|_| out.to_string())
        };
        if text.len() <= max_chars {
            return Ok(text);
        }

        // Oversized output is never silently cut: the model gets an explicit
        // marker with the total size and how to obtain the rest.
        let value = first_chars(&text, max_chars);
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "truncated": true,
            "total_chars": text.len(),
            "returned_chars": value.len(),
            "value": value,
            "next_steps": [
                "The script output exceeded the char guard; this value is only a prefix.",
                "Prefer rewriting the script to aggregate/summarize or select only the fields you need, then call again.",
                "Only if you genuinely need the whole payload, retry with a larger max_output_chars."
            ]
        }))
        .unwrap_or_else(|_| "(script output too large; rewrite to summarize)".to_string()))
    }
}

/// Guard defaults: sane output by default, absolute platform ceiling above.
const DEFAULT_MAX_OUTPUT_CHARS: usize = 12_000;
const MIN_OUTPUT_CHARS: usize = 1_000;
const MAX_OUTPUT_CHARS_LIMIT: usize = 64_000;

fn first_chars(s: &str, max: usize) -> &str {
    let mut boundary = max.min(s.len());
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &s[..boundary]
}

pub fn register(registry: &mut raisfast_agent::ToolRegistry, plugins: &Arc<PluginManager>) {
    for runtime in ["js", "lua", "rhai"] {
        registry.register(RunCodeTool::new(runtime, plugins.clone()));
    }
}
