//! Import a SKILL.md skill directory into the local skills store.
//!
//! M5-A: copies the directory as-is (no rewriting), validates SKILL.md with the
//! shared parser, warns on issues (skills.md §1/§6, §12-F). Legacy
//! SKILL.toml/manifest.toml manifests are reported as warnings (not migrated).

use raisfast_agent::SkillDocument;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ImportOutcome {
    pub name: String,
    pub dest: PathBuf,
    pub warnings: Vec<String>,
}

/// Layer names accepted by `raisfast skills import`.
pub fn dest_dir(root: &Path, layer: &str, tenant: Option<&str>) -> Option<PathBuf> {
    match layer {
        "platform" => Some(root.join("platform")),
        "tenant" => Some(root.join("tenants").join(tenant.unwrap_or("default"))),
        _ => None,
    }
}

/// Validate + copy a skill directory. Never rewrites SKILL.md.
pub fn import_skill(
    source: &Path,
    root: &Path,
    layer: &str,
    tenant: Option<&str>,
    force: bool,
) -> anyhow::Result<ImportOutcome> {
    let mut warnings = Vec::new();

    let base = dest_dir(root, layer, tenant)
        .ok_or_else(|| anyhow::anyhow!("invalid layer '{layer}' (expected platform|tenant)"))?;

    let manifest = source.join("SKILL.md");
    if !manifest.is_file() {
        if source.join("SKILL.toml").exists() || source.join("manifest.toml").exists() {
            return Err(anyhow::anyhow!(
                "source has legacy manifest (SKILL.toml/manifest.toml); M5-A does not migrate it — \
                 convert it to SKILL.md first"
            ));
        }
        return Err(anyhow::anyhow!(
            "source is not a skill directory: missing SKILL.md at {}",
            manifest.display()
        ));
    }

    // Validate with the shared parser; report issues as warnings (still importable
    // when only optional/unknown-field concerns exist).
    let raw = fs::read_to_string(&manifest)?;
    let parsed = match SkillDocument::parse(&raw) {
        Ok(d) => d,
        Err(e) => {
            return Err(anyhow::anyhow!("SKILL.md invalid: {e}"));
        }
    };
    let name = if parsed.frontmatter.name.is_empty() {
        // Fall back to the directory name (Claude convention: name from dir).
        source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        parsed.frontmatter.name.clone()
    };
    if name.is_empty() {
        return Err(anyhow::anyhow!("cannot determine skill name"));
    }
    if parsed.frontmatter.description.is_empty() {
        warnings.push("skill has no description; model discovery will be weak".into());
    }

    let dest = base.join(&name);
    if dest.exists() {
        if !force {
            return Err(anyhow::anyhow!(
                "skill '{}' already exists at {} (use --force to overwrite)",
                name,
                dest.display()
            ));
        }
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    copy_dir_all(source, &dest)?;

    Ok(ImportOutcome {
        name,
        dest,
        warnings,
    })
}

fn copy_dir_all(src: &Path, dst: &Path) -> anyhow::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_skill(dir: &Path, content: &str) {
        fs::create_dir_all(dir).unwrap();
        let mut f = fs::File::create(dir.join("SKILL.md")).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    #[test]
    fn imports_and_copies_without_rewriting() {
        let src = tempfile::tempdir().unwrap();
        write_skill(
            src.path(),
            "---\nname: fmt\ndescription: Format code.\n---\nrun fmt\n",
        );
        fs::create_dir_all(src.path().join("scripts")).unwrap();
        fs::write(src.path().join("scripts/x.sh"), "#!/bin/sh\n").unwrap();

        let root = tempfile::tempdir().unwrap();
        let out = import_skill(src.path(), root.path(), "platform", None, false).unwrap();
        assert_eq!(out.name, "fmt");
        assert!(out.dest.join("SKILL.md").is_file());
        assert!(out.dest.join("scripts/x.sh").is_file());
        assert_eq!(
            fs::read_to_string(out.dest.join("SKILL.md")).unwrap(),
            fs::read_to_string(src.path().join("SKILL.md")).unwrap(),
            "SKILL.md untouched"
        );
    }

    #[test]
    fn refuses_duplicate_without_force_and_legacy_manifest() {
        let root = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        write_skill(src.path(), "---\nname: a\ndescription: d\n---\nbody\n");
        import_skill(src.path(), root.path(), "platform", None, false).unwrap();
        assert!(import_skill(src.path(), root.path(), "platform", None, false).is_err());

        let legacy = tempfile::tempdir().unwrap();
        fs::write(legacy.path().join("SKILL.toml"), "[skill]\n").unwrap();
        assert!(
            import_skill(legacy.path(), root.path(), "platform", None, false)
                .unwrap_err()
                .to_string()
                .contains("legacy manifest")
        );
    }
}
