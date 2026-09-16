//! Error types + model-facing operational strings for the agent engine.
//!
//! Single definition file mirroring the core crate's `src/errors/` module
//! organization [照抄 crates/core/src/errors/ 模块化组织]. The string
//! consts are English by convention (engine has no request locale;
//! user-facing i18n lives at core's HTTP boundary) and their prefixes are a
//! code-owned wire protocol — see `prompt-engineering.md §12`.

/// Model-facing output prefix when a registered tool returned an error.
pub const TOOL_ERROR_PREFIX: &str = "Tool error: ";
/// Model-facing output prefix when the model requested a tool that is not
/// registered.
pub const TOOL_NOT_FOUND_PREFIX: &str = "Tool not found: ";
/// Fallback final text when the iteration cap is hit with no answer.
pub const MAX_ITERATIONS_TEXT: &str = "Max iterations reached before the task converged.";

/// Whether a tool output string was produced by the engine's failure paths
/// (tool error / unknown tool). The wire contract behind `tool_success`.
#[must_use]
pub fn tool_output_failed(output: &str) -> bool {
    output.starts_with(TOOL_ERROR_PREFIX) || output.starts_with(TOOL_NOT_FOUND_PREFIX)
}

/// Errors raised by chat providers (HTTP / transport / parse).
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider config error: {0}")]
    Config(String),
    #[error("http {status}: {body}")]
    Http { status: u16, body: String },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("cannot parse provider response: {0}")]
    Parse(String),
}

/// Errors raised by the turn loop.
#[derive(Debug, thiserror::Error)]
pub enum TurnError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
}

/// Errors raised by memory backends.
#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("store: {0}")]
    Store(String),
    #[error("recall: {0}")]
    Recall(String),
    #[error("forget: {0}")]
    Forget(String),
}

/// Errors raised while parsing a SKILL.md document.
#[derive(Debug, thiserror::Error)]
pub enum SkillDocError {
    #[error("SKILL.md is missing the leading `---` frontmatter delimiter")]
    MissingFrontmatter,
    #[error("SKILL.md frontmatter is missing required field `{0}`")]
    MissingRequiredField(&'static str),
    #[error("io error reading skill file: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_output_failed_matches_only_engine_prefixes() {
        assert!(tool_output_failed("Tool error: boom"));
        assert!(tool_output_failed("Tool not found: nope"));
        assert!(!tool_output_failed("Stored nickname"));
        assert!(!tool_output_failed("Tool error")); // prefix includes ": "
    }
}
