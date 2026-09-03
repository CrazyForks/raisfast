//! Skill tools: `read_skill(name)` loads a skill's SKILL.md body on demand
//! (Compact mode) and composed `skill__<tool>` wrappers over declared platform
//! tools. Ported semantics from zeroclaw `tools/skill_tool.rs`.
//!
//! M5-B scope: skills may declare optional frontmatter `tools:`/`disallowed-`
//! `tools:`. For each declared tool that exists in the current (post-allowlist)
//! registry a composed wrapper is registered; missing or disallowed tools get
//! no execution surface and the skill degrades to pure-instruction
//! (§12-B/C). External skills without the key keep instruction-only behavior.

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::ToolRegistry;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent::skills::LoadedSkill;

const MAX_TOOL_NAME_LEN: usize = 64;

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// FNV-1a 64-bit hex, only used to disambiguate sanitized names (ported from
/// zeroclaw `tools/skill_tool.rs`).
fn short_hash(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Composed tool name `{skill}__{tool}`, sanitized to a provider-safe
/// `[a-zA-Z0-9_-]{1,64}` name (port of zeroclaw `composed_tool_name`).
fn composed_tool_name(skill_name: &str, tool_name: &str) -> String {
    sanitize_tool_name(&format!("{skill_name}__{tool_name}"))
}

fn sanitize_tool_name(raw: &str) -> String {
    let already_valid =
        !raw.is_empty() && raw.len() <= MAX_TOOL_NAME_LEN && raw.chars().all(is_name_char);
    if already_valid {
        return raw.to_string();
    }
    let mapped: String = raw
        .chars()
        .map(|c| if is_name_char(c) { c } else { '_' })
        .collect();
    let suffix = format!("_{}", short_hash(raw));
    let budget = MAX_TOOL_NAME_LEN - suffix.len();
    let head: String = mapped.chars().take(budget).collect();
    format!("{head}{suffix}")
}

/// Register a `skill__<tool>` wrapper for every declared, available platform
/// tool of each enabled skill. Availability = the tool is present in the
/// registry at call time (allowlist already applied) and not listed in
/// `disallowed-tools`. Declarations that miss keep the skill pure-instruction.
pub(crate) fn register_skill_composed(registry: &mut ToolRegistry, skills: &[LoadedSkill]) {
    for skill in skills {
        if skill.tools.is_empty() {
            continue;
        }
        for tool in &skill.tools {
            if skill.disallowed_tools.iter().any(|d| d == tool) {
                continue;
            }
            let Some(target) = registry.get(tool) else {
                continue;
            };
            registry.register(SkillComposedTool {
                name: composed_tool_name(&skill.name, tool),
                description: format!("{} (from skill {})", target.description(), skill.name),
                target,
            });
        }
    }
}

/// Namespaced delegation wrapper: same schema/execution as the platform tool,
/// advertised under the skill-qualified composed name.
pub struct SkillComposedTool {
    name: String,
    description: String,
    target: Arc<dyn Tool>,
}

#[async_trait]
impl Tool for SkillComposedTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        self.target.parameters_schema()
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        self.target.execute(args).await
    }
}

pub struct ReadSkillTool {
    root: PathBuf,
    tenant: Option<String>,
    enabled: Vec<String>,
}

impl ReadSkillTool {
    pub(crate) fn new(root: PathBuf, tenant: Option<String>, enabled: Vec<String>) -> Self {
        Self {
            root,
            tenant,
            enabled,
        }
    }
}

#[async_trait]
impl Tool for ReadSkillTool {
    fn name(&self) -> &str {
        "read_skill"
    }

    fn description(&self) -> &str {
        "Load the full SKILL.md instructions for an available skill by name."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "skill name from the Available Skills list" } },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: Value) -> ToolExecution {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or("name required")?;
        match crate::agent::skills::skill_text(
            &self.root,
            self.tenant.as_deref(),
            &self.enabled,
            name,
        ) {
            Some(text) => Ok(text),
            None => Ok(format!("(skill '{name}' not found or disabled)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;

    struct DummyTool {
        name: String,
    }

    #[async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "platform tool"
        }
        fn parameters_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }
        async fn execute(&self, args: Value) -> ToolExecution {
            Ok(format!("{}:{}", self.name, args))
        }
    }

    fn loaded_skill(name: &str, tools: &[&str], disallowed: &[&str]) -> LoadedSkill {
        LoadedSkill {
            name: name.to_string(),
            description: "d".to_string(),
            instructions: "do it".to_string(),
            always: false,
            tools: tools.iter().map(|s| s.to_string()).collect(),
            disallowed_tools: disallowed.iter().map(|s| s.to_string()).collect(),
            dir: PathBuf::from("/tmp"),
        }
    }

    #[tokio::test]
    async fn composed_registered_for_declared_available_tool_and_delegates() {
        let mut registry = ToolRegistry::new();
        registry.register(DummyTool {
            name: "list_posts".into(),
        });
        registry.register(DummyTool {
            name: "memory_store".into(),
        });
        let skills = vec![loaded_skill(
            "content-ops",
            &["list_posts", "memory_store"],
            &[],
        )];
        register_skill_composed(&mut registry, &skills);

        let names = registry.names();
        assert!(names.contains(&"content-ops__list_posts".to_string()));
        assert!(names.contains(&"content-ops__memory_store".to_string()));

        let composed = registry.get("content-ops__list_posts").unwrap();
        assert_eq!(
            composed.description(),
            "platform tool (from skill content-ops)"
        );
        assert_eq!(
            composed
                .execute(serde_json::json!({"page": 1}))
                .await
                .unwrap(),
            "list_posts:{\"page\":1}"
        );
    }

    #[test]
    fn availability_miss_and_disallowed_keep_pure_instruction() {
        let mut registry = ToolRegistry::new();
        registry.register(DummyTool {
            name: "memory_store".into(),
        });
        let skills = vec![loaded_skill(
            "a",
            &["search_posts", "memory_store", "blocked"],
            &["blocked"],
        )];
        register_skill_composed(&mut registry, &skills);
        let names = registry.names();
        assert!(names.contains(&"memory_store".to_string()));
        assert!(!names.contains(&"search_posts".to_string()), "missing tool");
        assert!(
            !names.contains(&"blocked".to_string()),
            "disallowed removed"
        );
        assert!(names.contains(&"a__memory_store".to_string()));
        assert_eq!(names.len(), 2, "no wrapper for unavailable/blocked");
    }

    #[test]
    fn composed_name_sanitized_provider_safe() {
        assert_eq!(composed_tool_name("my-skill", "run"), "my-skill__run");
        let long = "x".repeat(80);
        let name = composed_tool_name(&long, "tool");
        assert!(name.len() <= 64);
        assert!(name.chars().all(is_name_char));
        let dotted = composed_tool_name("app:web", "run.lint");
        assert!(!dotted.contains(':') && !dotted.contains('.'));
    }

    fn make_skill(root: &Path, tenant: Option<&str>, name: &str, body: &str) {
        let dir = match tenant {
            Some(t) => root.join("tenants").join(t).join(name),
            None => root.join("platform").join(name),
        };
        std::fs::create_dir_all(&dir).unwrap();
        let mut f = std::fs::File::create(dir.join("SKILL.md")).unwrap();
        let _ =
            f.write_all(format!("---\nname: {name}\ndescription: desc\n---\n{body}\n").as_bytes());
    }

    #[tokio::test]
    async fn read_skill_returns_body_for_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        make_skill(tmp.path(), Some("t1"), "deploy", "1. run tests\n2. ship");
        let tool = ReadSkillTool::new(
            tmp.path().to_path_buf(),
            Some("t1".into()),
            vec!["deploy".into()],
        );
        let out = tool
            .execute(serde_json::json!({ "name": "deploy" }))
            .await
            .unwrap();
        assert!(out.contains("1. run tests"));
    }

    #[tokio::test]
    async fn read_skill_misses_when_not_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        make_skill(tmp.path(), None, "secret", "do not show");
        let tool = ReadSkillTool::new(tmp.path().to_path_buf(), None, vec!["other".into()]);
        let out = tool
            .execute(serde_json::json!({ "name": "secret" }))
            .await
            .unwrap();
        assert!(out.contains("not found or disabled"));
    }
}
