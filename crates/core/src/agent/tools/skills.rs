//! Skill tools: `read_skill(name)` loads a skill's SKILL.md body on demand
//! (Compact mode). Ported semantics from zeroclaw `tools/skill_tool.rs`.
//!
//! M5-A scope: skills from SKILL.md are instruction-only (no `skill:tool`
//! declarations), so only `read_skill` is registered. Availability/no-op notes
//! per skills.md §12-B/C.

use async_trait::async_trait;
use raisfast_agent::Tool;
use raisfast_agent::tool::ToolExecution;
use serde_json::Value;
use std::path::PathBuf;

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
    use crate::agent::skills;
    use std::io::Write;
    use std::path::Path;

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
