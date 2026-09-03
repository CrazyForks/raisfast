//! Blog post tools (published posts listing, …).

use async_trait::async_trait;
use raisfast_agent::tool::ToolExecution;
use raisfast_agent::{Tool, ToolRegistry};
use serde_json::Value;

use crate::AppState;
use crate::middleware::auth::AuthUser;

/// Register post domain tools onto the shared per-turn registry.
pub fn register(registry: &mut ToolRegistry, state: &AppState, auth: &AuthUser) {
    registry.register(ListPostsTool {
        service: state.post_service.clone(),
        auth: auth.clone(),
    });
}

// ─────────────────────────── list_posts ─────────────────────────────────────

struct ListPostsTool {
    service: std::sync::Arc<dyn crate::services::post::PostService>,
    auth: AuthUser,
}

#[async_trait]
impl Tool for ListPostsTool {
    fn name(&self) -> &str {
        "list_posts"
    }

    fn description(&self) -> &str {
        "List blog posts (published). Supports keyword search and pagination."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "page": { "type": "integer", "minimum": 1, "default": 1 },
                "page_size": { "type": "integer", "minimum": 1, "maximum": 100, "default": 10 },
                "q": { "type": "string", "description": "optional keyword search" }
            }
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let page = args.get("page").and_then(Value::as_i64).unwrap_or(1).max(1);
        let page_size = args
            .get("page_size")
            .and_then(Value::as_i64)
            .unwrap_or(10)
            .clamp(1, 100);
        let q = args.get("q").and_then(Value::as_str);

        let (posts, total) = self
            .service
            .list(&self.auth, page, page_size, None, None, q)
            .await
            .map_err(|e| format!("list_posts failed: {e}"))?;

        if posts.is_empty() {
            return Ok("(no posts found)".to_string());
        }
        let mut out = format!("found {total} posts:\n");
        for p in &posts {
            out.push_str(&format!(
                "- [{:?}] {} (id={}, author={})\n",
                p.status,
                p.title,
                p.id.0,
                p.author_name.as_deref().unwrap_or(""),
            ));
        }
        Ok(out.trim_end().to_string())
    }
}
