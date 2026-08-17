//! App CLI subcommand.
//!
//! Provides `app new` (generate project directory from a template).
//!
//! The binary embeds only the `base` skeleton via `rust-embed`:
//!
//! ```text
//! templates/app/
//! └── base/  — common skeleton (.env.example, .gitignore, README, dir layout)
//! ```
//!
//! Concrete templates (blog, ecommerce, community templates) live in remote
//! repositories. `--template <name>` selects the overlay layer inside a
//! remote tar.gz archive fetched via `--url` (e.g. a GitHub codeload URL);
//! the archive may also ship its own `base/` layer. If the fetch fails and
//! the requested template is `blank`, creation falls back to the embedded
//! base skeleton. All files go through `{{ name }}` / `{{ description }}`
//! placeholder rendering.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use rust_embed::Embed;

/// Embedded app templates (only the `base` skeleton ships with the binary).
#[derive(Embed)]
#[folder = "$RAISFAST_ROOT/templates/app"]
struct TemplateAssets;

/// Built-in template names shipped with the binary.
const EMBEDDED_TEMPLATES: &[&str] = &["blank"];

/// Template meta file: rendered to stdout as post-create instructions,
/// never written into the project directory.
const NEXT_STEPS_FILE: &str = "NEXT_STEPS.txt";

/// `app new` — create a new project directory from a template.
///
/// `template` selects the overlay layer (`blank` = embedded base skeleton
/// only; anything else requires `--url`). `url` points to a remote tar.gz
/// archive containing `base/` and/or `{template}/` layers.
pub async fn create_new(name: &str, template: &str, url: Option<&str>) -> anyhow::Result<()> {
    validate_template_name(template)?;

    let project_dir = PathBuf::from(name);
    if project_dir.exists() {
        anyhow::bail!("directory already exists: {}", project_dir.display());
    }
    std::fs::create_dir_all(&project_dir)?;

    if let Err(err) = create_new_inner(&project_dir, name, template, url).await {
        // Don't leave a half-written project behind.
        let _ = std::fs::remove_dir_all(&project_dir);
        return Err(err);
    }

    Ok(())
}

async fn create_new_inner(
    project_dir: &Path,
    name: &str,
    template: &str,
    url: Option<&str>,
) -> anyhow::Result<()> {
    let mut next_steps: Option<String> = None;

    let source = match url {
        Some(url) => match copy_remote(url, project_dir, name, template, &mut next_steps).await {
            Ok(()) => "remote",
            Err(err) if template == "blank" => {
                println!(
                    "⚠ failed to fetch remote template ({err:#}), \
                     using built-in base skeleton"
                );
                copy_embedded(project_dir, template, &mut next_steps)?;
                "embedded"
            }
            Err(err) => {
                anyhow::bail!(
                    "failed to fetch remote template '{template}': {err:#} \
                     (no built-in fallback for '{template}')"
                );
            }
        },
        None => {
            if !EMBEDDED_TEMPLATES.contains(&template) {
                anyhow::bail!(
                    "template '{template}' is not built-in (built-in: {}). \
                     Fetch it from a remote repository with --url <tar.gz>",
                    EMBEDDED_TEMPLATES.join(", ")
                );
            }
            copy_embedded(project_dir, template, &mut next_steps)?;
            "embedded"
        }
    };

    println!("✓ project created: {}", project_dir.display());
    println!("  template: {template} (source: {source})");
    println!();
    if let Some(steps) = next_steps {
        print!("{steps}");
    } else {
        println!("  next steps:");
        println!("    cd {}", project_dir.display());
    }

    Ok(())
}

/// Reject template names that could escape the template root (`..`, `/`, `\`).
fn validate_template_name(template: &str) -> anyhow::Result<()> {
    let valid = !template.is_empty()
        && template
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        anyhow::bail!("invalid template name: {template:?} (allowed: letters, digits, '-', '_')");
    }
    Ok(())
}

/// Human-readable description injected into rendered templates.
fn description_for(template: &str) -> &'static str {
    match template {
        "blog" => "A blog project built with raisfast",
        "ecommerce" => "An e-commerce project built with raisfast",
        _ => "A raisfast project",
    }
}

fn render_template(template: &str, name: &str, description: &str) -> String {
    template
        .replace("{{ name }}", name)
        .replace("{{ description }}", description)
}

/// Write one template file, rendering placeholders and creating parent dirs.
/// `.keep` markers only ensure directory presence (dirs are created, no file
/// is written). Returns the rendered content instead of writing when the
/// destination is the `NEXT_STEPS.txt` meta file.
fn write_template_file(
    project_dir: &Path,
    rel: &Path,
    bytes: &[u8],
    name: &str,
    template: &str,
) -> anyhow::Result<Option<String>> {
    let content = render_template(
        &String::from_utf8_lossy(bytes),
        name,
        description_for(template),
    );
    if rel.file_name().is_some_and(|f| f == NEXT_STEPS_FILE) {
        return Ok(Some(content));
    }
    let dest = project_dir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if rel.file_name().is_some_and(|f| f == ".keep") {
        return Ok(None);
    }
    std::fs::write(dest, content.as_bytes())?;
    Ok(None)
}

/// Ensure a template-relative path is safe to join under the project dir.
fn safe_rel_path(rel: &str) -> Option<PathBuf> {
    let path = Path::new(rel);
    // Only normal components allowed — no `..`, `.`, absolute paths, prefixes.
    path.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| path.to_path_buf())
}

/// Copy the embedded `base` skeleton into the project directory.
fn copy_embedded(
    project_dir: &Path,
    template: &str,
    next_steps: &mut Option<String>,
) -> anyhow::Result<()> {
    if !EMBEDDED_TEMPLATES.contains(&template) {
        anyhow::bail!(
            "template '{template}' is not built-in (built-in: {})",
            EMBEDDED_TEMPLATES.join(", ")
        );
    }

    let prefix = "base/";
    let mut paths: Vec<String> = TemplateAssets::iter()
        .map(|p| p.to_string())
        .filter(|p| p.starts_with(prefix))
        .collect();
    paths.sort();

    for rel in paths {
        let Some(asset) = TemplateAssets::get(&rel) else {
            continue;
        };
        let sub = &rel[prefix.len()..];
        if let Some(rel_path) = safe_rel_path(sub)
            && let Some(steps) =
                write_template_file(project_dir, &rel_path, &asset.data, "", template)?
        {
            *next_steps = Some(steps);
        }
    }
    Ok(())
}

/// Download a remote tar.gz template and extract it into the project dir.
async fn copy_remote(
    url: &str,
    project_dir: &Path,
    name: &str,
    template: &str,
    next_steps: &mut Option<String>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()?;
    let resp = client.get(url).send().await?.error_for_status()?;
    let bytes = resp.bytes().await?;
    extract_template_tarball(&bytes, project_dir, name, template, next_steps)
}

/// One file extracted from a template archive: source layer + project-relative path.
struct TemplateFile {
    layer: String,
    rel: PathBuf,
    data: Vec<u8>,
}

/// Extract an in-memory tar.gz template archive.
///
/// Entries are matched by locating a known layer component (`base` or the
/// requested template name) anywhere in their path, so archives both with
/// (`repo-main/base/...`) and without (`base/...`) a wrapper directory work.
/// Files after the matched layer component are rendered into the project dir.
fn extract_template_tarball(
    bytes: &[u8],
    project_dir: &Path,
    name: &str,
    template: &str,
    next_steps: &mut Option<String>,
) -> anyhow::Result<()> {
    let layers = ["base", template];
    let mut files: Vec<TemplateFile> = Vec::new();

    let gz = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.to_path_buf();
        if path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            anyhow::bail!("unsafe path in template archive: {}", path.display());
        }
        let comps: Vec<_> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_os_string()),
                _ => None,
            })
            .collect();
        let Some(idx) = comps
            .iter()
            .position(|c| layers.iter().any(|l| c.eq(std::ffi::OsStr::new(l))))
        else {
            continue; // not under any known layer
        };
        if idx + 1 >= comps.len() {
            continue; // layer component is the file name itself
        }
        let layer = layers
            .iter()
            .find(|l| comps[idx].eq(std::ffi::OsStr::new(l)))
            .unwrap_or(&"base");
        let rel: PathBuf = comps[idx + 1..].iter().collect();
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        files.push(TemplateFile {
            layer: (*layer).to_string(),
            rel,
            data,
        });
    }

    if !files.iter().any(|f| f.layer == "base") {
        anyhow::bail!("template archive contains no base/ layer");
    }
    if template != "blank" && !files.iter().any(|f| f.layer == template) {
        anyhow::bail!("template archive contains no {template}/ layer");
    }
    files.sort_by(|a, b| a.rel.cmp(&b.rel));

    for file in files {
        if let Some(steps) =
            write_template_file(project_dir, &file.rel, &file.data, name, template)?
        {
            *next_steps = Some(steps);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add_file<W: std::io::Write>(builder: &mut tar::Builder<W>, path: &str, data: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, data)
            .expect("append");
    }

    fn build_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let gz = flate2::write::GzEncoder::new(&mut bytes, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        for (path, data) in entries {
            add_file(&mut builder, path, data);
        }
        builder
            .into_inner()
            .expect("tar finish")
            .finish()
            .expect("gz flush");
        bytes
    }

    #[test]
    fn embedded_base_skeleton_present() {
        assert!(TemplateAssets::get("base/.env.example").is_some());
        assert!(TemplateAssets::get("base/.gitignore").is_some());
        assert!(TemplateAssets::get("base/README.md").is_some());
        assert!(TemplateAssets::get("base/extensions/content_types/.keep").is_some());
    }

    #[test]
    fn embedded_layers_are_base_only() {
        let unexpected: Vec<String> = TemplateAssets::iter()
            .map(|p| p.to_string())
            .filter(|p| !p.starts_with("base/"))
            .collect();
        assert!(
            unexpected.is_empty(),
            "unexpected template layers: {unexpected:?}"
        );
    }

    #[tokio::test]
    async fn embedded_blank_creates_skeleton() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");
        std::fs::create_dir_all(&project).expect("mkdir");

        create_new_inner(&project, "demo", "blank", None)
            .await
            .expect("create");

        assert!(project.join(".env.example").is_file());
        assert!(project.join(".gitignore").is_file());
        assert!(project.join("README.md").is_file());
        assert!(project.join("extensions/content_types").is_dir());
        assert!(project.join("extensions/plugins").is_dir());
        assert!(project.join("storage/db").is_dir());
        assert!(project.join("storage/uploads").is_dir());
        // Meta file must be printed, not written into the project.
        assert!(!project.join(NEXT_STEPS_FILE).exists());
    }

    #[tokio::test]
    async fn non_builtin_template_without_url_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");
        std::fs::create_dir_all(&project).expect("mkdir");

        let err = create_new_inner(&project, "demo", "blog", None)
            .await
            .expect_err("should require --url");
        assert!(err.to_string().contains("--url"));
    }

    #[test]
    fn tarball_extraction_renders_and_filters_layers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");
        std::fs::create_dir_all(&project).expect("mkdir");

        let tar_bytes = build_tar(&[
            ("repo-main/base/.env.example", b"APP_NAME={{ name }}"),
            ("repo-main/base/README.md", b"{{ description }}"),
            ("repo-main/base/NEXT_STEPS.txt", b"run {{ name }} now"),
            (
                "repo-main/blog/extensions/content_types/article.toml",
                b"[article]",
            ),
            ("repo-main/other/ignored.txt", b"x"),
        ]);

        let mut next_steps = None;
        extract_template_tarball(&tar_bytes, &project, "demo", "blog", &mut next_steps)
            .expect("extract");

        assert_eq!(
            std::fs::read_to_string(project.join(".env.example")).expect("env"),
            "APP_NAME=demo"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("README.md")).expect("readme"),
            "A blog project built with raisfast"
        );
        assert_eq!(next_steps.as_deref(), Some("run demo now"));
        assert!(!project.join(NEXT_STEPS_FILE).exists());
        assert!(
            project
                .join("extensions/content_types/article.toml")
                .is_file()
        );
        assert!(!project.join("other").exists());
    }

    #[test]
    fn tarball_without_wrapper_dir_works() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");
        std::fs::create_dir_all(&project).expect("mkdir");

        let tar_bytes = build_tar(&[
            ("base/.env.example", b"X={{ name }}"),
            ("shop/extensions/plugins/shop.js", b"// shop"),
        ]);

        let mut next_steps = None;
        extract_template_tarball(&tar_bytes, &project, "demo", "shop", &mut next_steps)
            .expect("extract");
        assert_eq!(
            std::fs::read_to_string(project.join(".env.example")).expect("env"),
            "X=demo"
        );
        assert!(project.join("extensions/plugins/shop.js").is_file());
    }

    #[test]
    fn tarball_missing_template_layer_errors() {
        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");

        let tar_bytes = build_tar(&[("repo-main/base/.env.example", b"")]);
        let err = extract_template_tarball(&tar_bytes, &project, "demo", "blog", &mut None)
            .expect_err("missing layer");
        assert!(err.to_string().contains("blog/ layer"));
    }

    #[test]
    fn tarball_path_traversal_rejected() {
        use std::io::Write as _;

        let dir = tempfile::tempdir().expect("tempdir");
        let project = dir.path().join("demo");

        // Hand-build a tar entry whose name contains `..`, bypassing the
        // tar builder's write-side validation (readers must stay safe).
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_mode(0o644);
        let name = b"repo-main/base/../evil.sh\0";
        header.as_old_mut().name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        gz.write_all(header.as_bytes()).expect("write header");
        gz.write_all(&[0u8; 1024]).expect("write trailer");

        let tar_bytes = gz.finish().expect("gz flush");
        assert!(
            extract_template_tarball(&tar_bytes, &project, "demo", "blank", &mut None).is_err()
        );
    }

    #[test]
    fn rejects_path_traversal_in_template_name() {
        assert!(validate_template_name("blog").is_ok());
        assert!(validate_template_name("../etc").is_err());
        assert!(validate_template_name("").is_err());
    }
}
