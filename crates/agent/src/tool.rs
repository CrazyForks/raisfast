use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// A tool's result is display text fed back to the model.
pub type ToolExecution = Result<String, String>;

/// Schema visible to the model.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolSpec {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// A tool the agent can invoke. The engine only knows `name + args -> text`.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> ToolExecution;
}

/// Named collection of tools.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.push(Arc::new(tool));
    }

    pub fn push(&mut self, tool: Arc<dyn Tool>) {
        self.tools.push(tool);
    }

    /// Keep only tools whose name satisfies `keep` (in place).
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.tools.retain(|t| keep(t.name()));
    }

    /// Schemas for every registered tool, in registration order.
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.tools
            .iter()
            .map(|t| ToolSpec::new(t.name(), t.description(), t.parameters_schema()))
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
