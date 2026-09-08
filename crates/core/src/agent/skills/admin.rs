//! Admin management of the on-disk skills store (list / write / delete).
//!
//! Skills live as directories under `skills_root()` (skills.md §2/§12-A):
//! `platform/<name>/SKILL.md` and `tenants/<tenant>/<name>/SKILL.md`. The
//! directory name is the identity used by `ai_agents.params.skill_bundles`;
//! the SKILL.md frontmatter carries the metadata. Writing preserves
//! unrecognized-by-form frontmatter fields (license/author/version/category/
//! tags) by round-tripping the existing document.

use std::fs;
use std::path::PathBuf;

use raisfast_agent::{SkillDocument, SkillFrontmatter};
use serde::Serialize;

use crate::errors::app_error::{AppError, AppResult};

/// One skill directory as seen by the admin listing.
#[derive(Debug, Serialize, Clone)]
pub struct AdminSkill {
    /// Directory name (the bundle identity agents reference).
    pub name: String,
    /// `platform` or `tenant`.
    pub layer: String,
    pub description: String,
    pub always: bool,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    /// SKILL.md body (instructions).
    pub instructions: String,
    /// False when SKILL.md failed to parse; `error` carries the reason.
    pub valid: bool,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

/// Editable payload shared by create and update.
#[derive(Debug, Clone)]
pub struct SkillWrite {
    pub description: String,
    pub instructions: String,
    pub always: bool,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// Validate a skill directory name: ascii slug, max 64 chars. This is the
/// path-traversal guard for every write/delete below.
pub fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn layer_dir(root: &std::path::Path, layer: &str, tenant: Option<&str>) -> AppResult<PathBuf> {
    match layer {
        "platform" => Ok(root.join("platform")),
        "tenant" => Ok(root.join("tenants").join(tenant.unwrap_or("default"))),
        _ => Err(AppError::BadRequest(
            "invalid scope: expected platform|tenant".to_string(),
        )),
    }
}

/// Scan both layers for the tenant. Malformed SKILL.md files are listed with
/// `valid: false` (so they can be repaired) instead of being skipped.
pub fn list_skills(root: &std::path::Path, tenant: Option<&str>) -> AppResult<Vec<AdminSkill>> {
    let mut out = Vec::new();
    for layer in ["platform", "tenant"] {
        let Ok(entries) = fs::read_dir(layer_dir(root, layer, tenant)?) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let (doc, error) = match fs::read_to_string(dir.join("SKILL.md"))
                .map_err(|e| e.to_string())
                .and_then(|raw| SkillDocument::parse(&raw).map_err(|e| e.to_string()))
            {
                Ok(doc) => (Some(doc), None),
                Err(e) => (None, Some(e)),
            };
            let updated_at = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| {
                    chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                        .map(|dt| dt.to_rfc3339())
                        .unwrap_or_default()
                })
                .filter(|s| !s.is_empty());
            out.push(AdminSkill {
                name,
                layer: layer.to_string(),
                description: doc
                    .as_ref()
                    .map(|d| d.frontmatter.description.clone())
                    .unwrap_or_default(),
                always: doc.as_ref().is_some_and(|d| d.frontmatter.always),
                tools: doc
                    .as_ref()
                    .map(|d| d.frontmatter.tools.clone())
                    .unwrap_or_default(),
                disallowed_tools: doc
                    .as_ref()
                    .map(|d| d.frontmatter.disallowed_tools.clone())
                    .unwrap_or_default(),
                instructions: doc.map(|d| d.body).unwrap_or_default(),
                valid: error.is_none(),
                error,
                updated_at,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Create a new skill directory + SKILL.md. Fails with 409 when it exists.
pub fn create_skill(
    root: &std::path::Path,
    tenant: Option<&str>,
    layer: &str,
    name: &str,
    write: &SkillWrite,
) -> AppResult<()> {
    if !valid_skill_name(name) {
        return Err(AppError::BadRequest(
            "invalid skill name: use ascii letters, digits, '-' or '_' (max 64)".to_string(),
        ));
    }
    let dir = layer_dir(root, layer, tenant)?.join(name);
    if dir.join("SKILL.md").exists() {
        return Err(AppError::Conflict("duplicate_entry".to_string()));
    }
    fs::create_dir_all(&dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create skill dir: {e}")))?;
    write_skill_doc(&dir, name, write, None)
}

/// Overwrite the SKILL.md of an existing skill, preserving frontmatter fields
/// not managed by the form (license/author/version/category/tags).
pub fn update_skill(
    root: &std::path::Path,
    tenant: Option<&str>,
    layer: &str,
    name: &str,
    write: &SkillWrite,
) -> AppResult<()> {
    if !valid_skill_name(name) {
        return Err(AppError::BadRequest(
            "invalid skill name: use ascii letters, digits, '-' or '_' (max 64)".to_string(),
        ));
    }
    let dir = layer_dir(root, layer, tenant)?.join(name);
    let manifest = dir.join("SKILL.md");
    if !manifest.exists() {
        return Err(AppError::not_found("skill"));
    }
    let existing = fs::read_to_string(&manifest)
        .ok()
        .and_then(|raw| SkillDocument::parse(&raw).ok());
    write_skill_doc(&dir, name, write, existing.as_ref())
}

/// Delete a skill directory (SKILL.md plus any imported scripts/assets).
pub fn delete_skill(
    root: &std::path::Path,
    tenant: Option<&str>,
    layer: &str,
    name: &str,
) -> AppResult<()> {
    if !valid_skill_name(name) {
        return Err(AppError::BadRequest(
            "invalid skill name: use ascii letters, digits, '-' or '_' (max 64)".to_string(),
        ));
    }
    let dir = layer_dir(root, layer, tenant)?.join(name);
    if !dir.join("SKILL.md").exists() {
        return Err(AppError::not_found("skill"));
    }
    fs::remove_dir_all(&dir)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("delete skill dir: {e}")))
}

fn write_skill_doc(
    dir: &std::path::Path,
    name: &str,
    write: &SkillWrite,
    existing: Option<&SkillDocument>,
) -> AppResult<()> {
    if write.description.trim().is_empty() {
        return Err(AppError::BadRequest(
            "skill description must not be empty".to_string(),
        ));
    }
    // Preserve unmanaged frontmatter fields from the previous document.
    let (license, author, version, category, tags) = match existing {
        Some(doc) => (
            doc.frontmatter.license.clone(),
            doc.frontmatter.author.clone(),
            doc.frontmatter.version.clone(),
            doc.frontmatter.category.clone(),
            doc.frontmatter.tags.clone(),
        ),
        None => (None, None, None, None, Vec::new()),
    };
    let doc = SkillDocument {
        frontmatter: SkillFrontmatter {
            name: name.to_string(),
            description: write.description.trim().to_string(),
            license,
            author,
            version,
            category,
            tags,
            always: write.always,
            tools: write.tools.clone(),
            disallowed_tools: write.disallowed_tools.clone(),
        },
        body: write.instructions.clone(),
    };
    fs::write(dir.join("SKILL.md"), doc.serialize())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("write SKILL.md: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(_name: &str, desc: &str) -> SkillWrite {
        SkillWrite {
            description: desc.to_string(),
            instructions: "do the thing".to_string(),
            always: false,
            tools: vec!["list_posts".to_string()],
            disallowed_tools: vec![],
        }
    }

    #[test]
    fn create_update_delete_roundtrip_preserves_extra_fields() {
        let root = tempfile::tempdir().unwrap();
        create_skill(
            root.path(),
            Some("t1"),
            "tenant",
            "fmt",
            &write("fmt", "Format."),
        )
        .unwrap();

        let listed = list_skills(root.path(), Some("t1")).unwrap();
        let s = listed.iter().find(|s| s.name == "fmt").unwrap();
        assert!(s.valid);
        assert_eq!(s.layer, "tenant");
        assert_eq!(s.description, "Format.");
        assert_eq!(s.tools, vec!["list_posts"]);

        // Inject an unmanaged field, then update via admin write.
        let manifest = root.path().join("tenants/t1/fmt/SKILL.md");
        let raw = fs::read_to_string(&manifest).unwrap();
        let with_license = raw.replace("---\nname:", "---\nlicense: MIT\nname:");
        fs::write(&manifest, with_license).unwrap();

        let mut w2 = write("fmt", "Format v2.");
        w2.always = true;
        update_skill(root.path(), Some("t1"), "tenant", "fmt", &w2).unwrap();

        let raw = fs::read_to_string(&manifest).unwrap();
        assert!(raw.contains("license: MIT"), "unmanaged field preserved");
        assert!(raw.contains("always: true"));
        let doc = SkillDocument::parse(&raw).unwrap();
        assert_eq!(doc.frontmatter.description, "Format v2.");
        assert!(doc.body.trim_end() == "do the thing");

        delete_skill(root.path(), Some("t1"), "tenant", "fmt").unwrap();
        assert!(
            list_skills(root.path(), Some("t1"))
                .unwrap()
                .iter()
                .all(|s| s.name != "fmt")
        );
    }

    #[test]
    fn rejects_traversal_and_duplicates_and_missing() {
        let root = tempfile::tempdir().unwrap();
        assert!(create_skill(root.path(), None, "platform", "../evil", &write("x", "d")).is_err());
        assert!(create_skill(root.path(), None, "bad-layer", "ok", &write("x", "d")).is_err());
        create_skill(root.path(), None, "platform", "dupe", &write("x", "d")).unwrap();
        assert!(matches!(
            create_skill(root.path(), None, "platform", "dupe", &write("x", "d")),
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            update_skill(root.path(), None, "platform", "ghost", &write("x", "d")),
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            delete_skill(root.path(), None, "platform", "ghost"),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn lists_invalid_skills_for_repair() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("platform/broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "no frontmatter").unwrap();

        let listed = list_skills(root.path(), None).unwrap();
        let broken = listed.iter().find(|s| s.name == "broken").unwrap();
        assert!(!broken.valid);
        assert!(broken.error.is_some());
    }
}
