//! `raisfast app pack` — validate a bundle directory, generate the
//! `META/manifest.sha256` hash manifest and zip it into a `.rafapp`
//! (app-bundle.md §12, MVP: no signing).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::apps::manifest::AppBundleManifest;

/// Files that never ship in a package.
const EXCLUDED: &[&str] = &[".DS_Store", ".gitignore", ".git"];

/// Pack `dir` into `output` (.rafapp). Returns the file count packed.
pub fn pack(dir: &Path, output: &Path) -> anyhow::Result<usize> {
    let app_toml = dir.join("app.toml");
    let content = std::fs::read_to_string(&app_toml)
        .map_err(|e| anyhow::anyhow!("app.toml missing in {}: {e}", dir.display()))?;
    AppBundleManifest::parse_and_validate(&content)
        .map_err(|errors| anyhow::anyhow!("app.toml invalid: {}", errors.join("; ")))?;

    let files = collect_files(dir)?;
    if files.is_empty() {
        anyhow::bail!("no files to pack under {}", dir.display());
    }

    // Hash manifest: every shipped file except META/.
    let mut manifest = String::new();
    for (rel, path) in &files {
        let bytes = std::fs::read(path)?;
        manifest.push_str(&format!(
            "{}  {}\n",
            crate::apps::package::hash_bytes(&bytes),
            rel.replace('\\', "/")
        ));
    }

    let file = std::fs::File::create(output)
        .map_err(|e| anyhow::anyhow!("cannot create {}: {e}", output.display()))?;
    let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(file));
    let opts: zip::write::SimpleFileOptions = Default::default();

    for (rel, path) in &files {
        writer.start_file(rel.replace('\\', "/"), opts)?;
        std::io::copy(&mut std::fs::File::open(path)?, &mut writer)?;
    }
    writer.start_file("META/manifest.sha256", opts)?;
    writer.write_all(manifest.as_bytes())?;
    writer.finish()?;

    Ok(files.len())
}

/// All packable files (relative path → absolute path), `META/` skipped,
/// sorted for deterministic archives.
fn collect_files(dir: &Path) -> anyhow::Result<BTreeMap<String, PathBuf>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if EXCLUDED.contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                if name == "META" && current == dir {
                    continue; // regenerated on every pack
                }
                stack.push(path);
                continue;
            }
            let rel = path.strip_prefix(dir)?.to_string_lossy().replace('\\', "/");
            out.insert(rel, path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_produces_verifiable_archive() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("app.toml"),
            "[app]\nid = \"demo-app\"\nname = \"Demo\"\nversion = \"0.1.0\"\n",
        )
        .expect("write");
        std::fs::create_dir(dir.path().join("content-types")).expect("mkdir");
        std::fs::write(
            dir.path().join("content-types/item.toml"),
            "[content_type]\nname=\"Item\"\nsingular=\"item\"\nplural=\"items\"\n\
             table=\"demo_app_items\"\ngroup=\"demo-app\"\n[fields.title]\ntype=\"text\"\n",
        )
        .expect("write");
        // Stale META must not leak into the fresh manifest.
        std::fs::create_dir(dir.path().join("META")).expect("mkdir");
        std::fs::write(dir.path().join("META/manifest.sha256"), "stale").expect("write");

        let out = dir.path().join("demo.rafapp");
        let count = pack(dir.path(), &out).expect("pack");
        assert_eq!(count, 2);

        let unpack_dir = tempfile::tempdir().expect("tempdir");
        let bytes = std::fs::read(&out).expect("read");
        let pkg =
            crate::apps::package::AppPackage::unpack(&bytes, unpack_dir.path()).expect("unpack");
        assert_eq!(pkg.manifest.app.id, "demo-app");
        assert_eq!(pkg.content_types.len(), 1);
    }

    #[test]
    fn pack_rejects_invalid_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("app.toml"),
            "[app]\nid = \"admin\"\nname = \"x\"\nversion = \"not-semver\"\n",
        )
        .expect("write");
        let err = pack(dir.path(), &dir.path().join("x.rafapp")).expect_err("invalid");
        assert!(err.to_string().contains("app.toml invalid"));
    }
}
