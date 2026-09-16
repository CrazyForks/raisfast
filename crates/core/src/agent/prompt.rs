//! System prompt assembly with **versioned templates** + stable `system_hash`.
//!
//! Framework sections are built by a versioned builder (currently v1) from
//! source-tree prompt files (`src/agent/prompts/*.md`, embedded via
//! `include_str!` + debug hot-read — see `utils/prompt_file.rs`). The
//! assembled text is stable (no timestamps). `system_hash` = SHA-256 of
//! `version + model + text + sorted tool list`, so any prompt/model/tool change
//! yields a new hash — the anchor for replay/regression/cache grouping.
//! Full design: `prompt-engineering.md §2/§8/§11`.

use crate::agent::models::ai_agent::AiAgent;
use crate::utils::prompt_file::prompt_file;
use sha2::{Digest, Sha256};

/// Active framework prompt template version. Bump when framework sections
/// change so old turns stay distinguishable by hash/version.
pub const PROMPT_TEMPLATE_VERSION: u32 = 1;

/// Assembled system prompt for one turn.
#[derive(Debug, Clone)]
pub struct AssembledPrompt {
    pub text: String,
    /// Stable fingerprint of version + model + system text + tool list.
    pub hash: String,
    /// The template version that produced `text`.
    pub version: u32,
    /// Total chars of the rendered system prompt (context budget observability).
    pub system_chars: usize,
    /// Chars contributed by the injected skills section, 0 when none.
    pub skills_chars: usize,
}

/// Versioned prompt template registry.
///
/// Each version may eventually carry its own section builder; unknown/legacy
/// versions fall back to the current builder (turn:meta still records the hash
/// they were built under).
#[derive(Debug, Clone, Copy, Default)]
pub struct PromptRegistry;

impl PromptRegistry {
    pub fn current_version(&self) -> u32 {
        PROMPT_TEMPLATE_VERSION
    }

    /// Assemble with the current (active) template.
    pub fn assemble_current(&self, agent: &AiAgent, tools: &[String]) -> AssembledPrompt {
        assemble_v1(agent, tools, None)
    }
}

/// Build the system prompt (and its stable hash) for a turn with the active
/// template version.
pub fn assemble(agent: &AiAgent, tools: &[String]) -> AssembledPrompt {
    assemble_impl(agent, tools, None)
}

/// Like [`assemble`], but appends an optional skills section (rendered skills)
/// before hashing, so skills changes are covered by `system_hash`.
pub fn assemble_with_skills(
    agent: &AiAgent,
    tools: &[String],
    skills_section: Option<&str>,
) -> AssembledPrompt {
    assemble_impl(agent, tools, skills_section)
}

fn assemble_impl(
    agent: &AiAgent,
    tools: &[String],
    skills_section: Option<&str>,
) -> AssembledPrompt {
    assemble_v1(agent, tools, skills_section)
}

fn assemble_v1(agent: &AiAgent, tools: &[String], skills_section: Option<&str>) -> AssembledPrompt {
    let mut tools = tools.to_vec();
    tools.sort();
    tools.dedup();

    let mut sections: Vec<String> = Vec::new();

    sections.push(prompt_file!("src/agent/prompts/role.md").replace("{name}", &agent.name));

    sections.push(prompt_file!("src/agent/prompts/task.md"));

    sections.push(prompt_file!("src/agent/prompts/safety.md"));

    if !tools.is_empty() {
        sections.push(format!(
            "# Permissions\n本回合可用工具：{}。",
            tools.join(", ")
        ));
    }

    if !agent.system_prompt.trim().is_empty() {
        sections.push(format!(
            "## Agent instructions\n{}",
            agent.system_prompt.trim()
        ));
    }

    if let Some(skills) = skills_section {
        sections.push(skills.to_string());
    }

    let text = sections.join("\n\n");
    let version = PROMPT_TEMPLATE_VERSION;
    let hash = system_hash(version, &agent.model, &text, &tools);
    AssembledPrompt {
        skills_chars: skills_section.map_or(0, str::len),
        system_chars: text.len(),
        text,
        hash,
        version,
    }
}

/// SHA-256 hex of version + model + system text + sorted tool list.
fn system_hash(version: u32, model: &str, text: &str, tools: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("v{version}\n").as_bytes());
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(text.as_bytes());
    for tool in tools {
        hasher.update(b"\ntool:");
        hasher.update(tool.as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(model: &str, name: &str, system_prompt: &str) -> AiAgent {
        serde_json::from_value(serde_json::json!({
            "id": "1",
            "tenant_id": "t",
            "name": name,
            "model": model,
            "system_prompt": system_prompt,
            "provider": "openai_compat",
            "temperature": null,
            "max_iterations": 10,
            "tools": [],
            "memory_enabled": true,
            "created_at": "2026-09-03T00:00:00Z",
            "updated_at": "2026-09-03T00:00:00Z"
        }))
        .expect("deserialize agent fixture")
    }

    #[test]
    fn hash_is_stable_for_same_input() {
        let a = agent("m1", "helper", "be nice");
        let tools = vec!["b".to_string(), "a".to_string()];
        let p1 = assemble(&a, &tools);
        let p2 = assemble(&a, &["a".to_string(), "b".to_string()]);
        assert_eq!(p1.hash, p2.hash, "tool order must not matter");
        assert_eq!(p1.version, PROMPT_TEMPLATE_VERSION);
        assert!(p1.text.contains("# Role"));
        assert!(p1.text.contains("be nice"));
    }

    #[test]
    fn role_section_substitutes_agent_name() {
        let a = agent("m", "小助手", "p");
        let p = assemble(&a, &[]);
        assert!(p.text.contains("「小助手」"));
        assert!(
            !p.text.contains("{name}"),
            "placeholder must be substituted"
        );
    }

    #[test]
    fn stats_capture_skills_budget_signal() {
        let a = agent("m", "a", "p");
        let tools = vec!["list_posts".to_string()];
        let plain = assemble(&a, &tools);
        assert_eq!(
            plain.skills_chars, 0,
            "no skills section -> no skill budget"
        );
        assert!(plain.system_chars > 0);

        let skill_body = "# skill body for budget observability";
        let with_skills = assemble_with_skills(&a, &tools, Some(skill_body));
        assert_eq!(with_skills.skills_chars, skill_body.len());
        assert!(with_skills.system_chars > with_skills.skills_chars);
        assert_ne!(
            plain.hash, with_skills.hash,
            "skill content covered by hash"
        );
    }

    #[test]
    fn hash_changes_on_any_component_change() {
        let tools = vec!["list_posts".to_string()];
        let base = assemble(&agent("m", "a", "p"), &tools);
        let other_model = assemble(&agent("m2", "a", "p"), &tools);
        let other_tools = assemble(&agent("m", "a", "p"), &["other".to_string()]);
        let other_prompt = assemble(&agent("m", "a", "p2"), &tools);
        assert_ne!(base.hash, other_model.hash);
        assert_ne!(base.hash, other_tools.hash);
        assert_ne!(base.hash, other_prompt.hash);
    }
}
