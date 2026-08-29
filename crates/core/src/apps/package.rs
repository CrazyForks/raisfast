//! `.rafapp` package handling: unpack (zip-slip protected), hash-manifest
//! verification and structural parsing (app-bundle.md §2, §4.1 step 1-2).

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::apps::manifest::AppBundleManifest;
use crate::errors::app_error::{AppError, AppResult};
use crate::plugins::PluginManifest;

/// Hard cap on total uncompressed payload (zip-bomb guard).
const MAX_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024;

/// A fully verified and parsed app bundle.
#[derive(Debug)]
pub struct AppPackage {
    pub manifest: AppBundleManifest,
    /// Parsed content types with their raw TOML source (kept for
    /// `app_ct_refs` — the authoritative definition the app shipped).
    pub content_types: Vec<ContentTypePayload>,
    /// Parsed plugin manifests (one per `plugins/{name}/manifest.toml`).
    pub plugins: Vec<PluginPayload>,
    /// Channel seeds (`channels/*.json`).
    pub channel_seeds: Vec<Value>,
    /// API-client seeds (`api-clients/*.json`).
    pub api_client_seeds: Vec<Value>,
    /// Option seeds (`seeds/options.json`).
    pub option_seeds: Vec<Value>,
    /// Role seeds (`seeds/roles.json`).
    pub role_seeds: Vec<Value>,
    /// CT data seeds (`seeds/*.json` others): (table, rows).
    pub ct_seeds: Vec<CtSeedPayload>,
    /// Declared admin pages across plugins (informational; SPA pending).
    pub admin_page_count: usize,
}

/// One content type in the package.
#[derive(Debug, Clone)]
pub struct ContentTypePayload {
    pub schema: crate::content_type::schema::ContentTypeSchema,
    pub toml_source: String,
}

/// One plugin in the package (parsed manifest + its directory inside the
/// unpacked bundle).
#[derive(Debug, Clone)]
pub struct PluginPayload {
    pub manifest: PluginManifest,
    /// Directory inside the unpacked bundle (e.g. `.../plugins/router`).
    pub dir: PathBuf,
}

/// CT data seed file payload.
#[derive(Debug, Clone)]
pub struct CtSeedPayload {
    pub table: String,
    pub rows: Vec<Value>,
}

impl AppPackage {
    /// Unpack `bytes` (a `.rafapp` zip) into `dest` and parse everything.
    ///
    /// Verification (§4.1 step 2): manifest validation, hash-manifest
    /// verification, structural parse. All failures are aggregated — the
    /// wizard gets the full list, not a drip-feed.
    pub fn unpack(bytes: &[u8], dest: &Path) -> AppResult<Self> {
        Self::unpack_inner(bytes, dest).map_err(|errors| {
            AppError::BadRequest(format!("invalid app bundle: {}", errors.join("; ")))
        })
    }

    fn unpack_inner(bytes: &[u8], dest: &Path) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();

        let extracted = extract_zip(bytes, dest).map_err(|e| vec![e])?;
        verify_hash_manifest(&extracted, dest).unwrap_or_else(|e| errors.push(e));

        // ── app.toml ──────────────────────────────────────────────
        // Manifest problems are ACCUMULATED onto `errors` (early `?` here
        // would discard the hash-verification result above).
        let manifest_path = dest.join("app.toml");
        let manifest_content = match std::fs::read_to_string(&manifest_path) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("app.toml missing or unreadable: {e}"));
                return Err(errors);
            }
        };
        let manifest = match AppBundleManifest::parse_and_validate(&manifest_content) {
            Ok(m) => m,
            Err(errs) => {
                errors.extend(errs.iter().map(|e| format!("app.toml: {e}")));
                return Err(errors);
            }
        };

        // ── content-types/*.toml ──────────────────────────────────
        let mut content_types = Vec::new();
        for (rel, _) in extracted
            .iter()
            .filter(|(p, _)| p.starts_with("content-types/") && p.ends_with(".toml"))
        {
            let path = dest.join(rel);
            let source = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{rel}: unreadable: {e}"));
                    continue;
                }
            };
            match crate::content_type::schema::ContentTypeSchema::parse_from_str(&source) {
                Ok(schema) => content_types.push(ContentTypePayload {
                    schema,
                    toml_source: source,
                }),
                Err(e) => errors.push(format!("{rel}: {e}")),
            }
        }

        // ── plugins/*/manifest.toml ───────────────────────────────
        let mut plugin_dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
        for rel in extracted.keys() {
            let Some(rest) = rel.strip_prefix("plugins/") else {
                continue;
            };
            let Some(name) = rest.split('/').next() else {
                continue;
            };
            if rest == format!("{name}/manifest.toml") {
                plugin_dirs.insert(name.to_string(), dest.join(rel));
            }
        }
        let mut plugins = Vec::new();
        for (name, manifest_path) in &plugin_dirs {
            match std::fs::read_to_string(manifest_path)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
                .and_then(|c| {
                    toml::from_str::<PluginManifest>(&c)
                        .map_err(|e| AppError::Internal(anyhow::anyhow!("{e}")))
                }) {
                Ok(mut m) => {
                    if m.plugin.id.contains('/') {
                        errors.push(format!(
                            "plugins/{name}: plugin id '{}' must be a bare name (namespaced at \
                             materialization)",
                            m.plugin.id
                        ));
                        continue;
                    }
                    if m.plugin.id != *name {
                        // Directory name wins — materialization keys off it.
                        m.plugin.id = name.clone();
                    }
                    plugins.push(PluginPayload {
                        manifest: m,
                        dir: manifest_path.parent().unwrap_or(dest).to_path_buf(),
                    });
                }
                Err(e) => errors.push(format!("plugins/{name}/manifest.toml: {e}")),
            }
        }

        // ── channels / api-clients / seeds (JSON) ─────────────────
        let mut channel_seeds = Vec::new();
        let mut api_client_seeds = Vec::new();
        let mut option_seeds = Vec::new();
        let mut role_seeds = Vec::new();
        let mut ct_seeds = Vec::new();

        for (rel, _) in extracted.iter().filter(|(p, _)| p.ends_with(".json")) {
            let path = dest.join(rel);
            let parsed: Value = match std::fs::read_to_string(&path)
                .map_err(|e| format!("{e}"))
                .and_then(|c| serde_json::from_str(&c).map_err(|e| format!("{e}")))
            {
                Ok(v) => v,
                Err(e) => {
                    errors.push(format!("{rel}: invalid JSON: {e}"));
                    continue;
                }
            };
            if rel.starts_with("channels/") {
                validate_channel_seed(rel, &parsed).unwrap_or_else(|e| errors.push(e));
                channel_seeds.push(parsed);
            } else if rel.starts_with("api-clients/") {
                validate_api_client_seed(rel, &parsed).unwrap_or_else(|e| errors.push(e));
                api_client_seeds.push(parsed);
            } else if rel.starts_with("seeds/") {
                let file = rel.trim_start_matches("seeds/");
                match file {
                    "options.json" => match parsed.as_array() {
                        Some(items) => {
                            for item in items {
                                validate_option_seed(rel, item).unwrap_or_else(|e| errors.push(e));
                            }
                            option_seeds = items.clone();
                        }
                        None => errors.push(format!("{rel}: must be a JSON array")),
                    },
                    "roles.json" => match parsed.as_array() {
                        Some(items) => {
                            for item in items {
                                validate_role_seed(rel, item).unwrap_or_else(|e| errors.push(e));
                            }
                            role_seeds = items.clone();
                        }
                        None => errors.push(format!("{rel}: must be a JSON array")),
                    },
                    _ => match ct_seed_payload(rel, &parsed) {
                        Ok(payload) => ct_seeds.push(payload),
                        Err(e) => errors.push(e),
                    },
                }
            }
        }

        if errors.is_empty() {
            let admin_page_count: usize =
                plugins.iter().map(|p| p.manifest.admin_pages.len()).sum();
            Ok(Self {
                manifest,
                content_types,
                plugins,
                channel_seeds,
                api_client_seeds,
                option_seeds,
                role_seeds,
                ct_seeds,
                admin_page_count,
            })
        } else {
            Err(errors)
        }
    }
}

/// Extract a zip archive with zip-slip protection. Returns the sorted map of
/// relative path → size for every regular file extracted.
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<BTreeMap<String, u64>, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| format!("not a valid .rafapp zip: {e}"))?;

    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create unpack dir: {e}"))?;

    let mut files = BTreeMap::new();
    let mut total: u64 = 0;
    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|e| format!("zip entry {idx}: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let rel = safe_zip_path(&name)?;
        total += entry.size();
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(format!(
                "package exceeds the {MAX_UNCOMPRESSED_BYTES} byte uncompressed limit"
            ));
        }
        let out_path = dest.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create dir for {name}: {e}"))?;
        }
        let mut out =
            std::fs::File::create(&out_path).map_err(|e| format!("cannot write {name}: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| format!("cannot extract {name}: {e}"))?;
        files.insert(rel, entry.size());
    }
    Ok(files)
}

/// A zip entry name is safe when it is relative, `/`-separated, has no
/// parent/cur components and no windows drive prefix (zip-slip guard).
fn safe_zip_path(name: &str) -> Result<String, String> {
    if name.contains('\\') {
        return Err(format!("illegal path separator in '{name}'"));
    }
    let path = Path::new(name);
    if !path.is_relative() {
        return Err(format!("absolute path in archive: '{name}'"));
    }
    let mut rel = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::Normal(part) => rel.push(part),
            Component::CurDir => {}
            _ => return Err(format!("illegal path component in '{name}'")),
        }
    }
    Ok(rel
        .to_str()
        .ok_or_else(|| format!("non-utf8 path '{name}'"))?
        .to_string())
}

/// Verify `META/manifest.sha256`: every non-META file must be listed and
/// match; no extra entries allowed (D6: hash manifest replaces signatures).
fn verify_hash_manifest(files: &BTreeMap<String, u64>, dest: &Path) -> Result<(), String> {
    let manifest_path = dest.join("META/manifest.sha256");
    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "META/manifest.sha256 missing or unreadable: {e} (pack with \
             `raisfast app pack`)"
        )
    })?;

    let mut listed = std::collections::BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // sha256sum format: "<hex>  <path>" (two-space separator)
        let (hash, path) = line
            .split_once("  ")
            .ok_or_else(|| format!("malformed hash line: '{line}'"))?;
        if hash.len() != 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("malformed hash digest: '{hash}'"));
        }
        listed.insert(path.to_string(), hash.to_lowercase());
    }

    for rel in files.keys() {
        if rel.starts_with("META/") {
            continue;
        }
        let Some(expected) = listed.remove(rel) else {
            return Err(format!("file '{rel}' not covered by META/manifest.sha256"));
        };
        let bytes =
            std::fs::read(dest.join(rel)).map_err(|e| format!("cannot re-read '{rel}': {e}"))?;
        let actual = hash_bytes(&bytes);
        if actual != expected {
            return Err(format!(
                "hash mismatch for '{rel}': manifest {expected}, file {actual}"
            ));
        }
    }
    if let Some(extra) = listed.keys().next() {
        return Err(format!("META/manifest.sha256 lists unknown file '{extra}'"));
    }
    Ok(())
}

/// SHA-256 hex digest.
pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

fn validate_channel_seed(rel: &str, v: &Value) -> Result<(), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{rel}: channel seed must be an object"))?;
    for field in [
        "channel_key",
        "provider",
        "mode",
        "transport",
        "framing",
        "codec",
        "verify_kind",
        "target_type",
    ] {
        if !obj.contains_key(field) {
            return Err(format!("{rel}: channel seed missing '{field}'"));
        }
    }
    if obj.contains_key("credentials") && !v["credentials"].is_null() {
        return Err(format!(
            "{rel}: credentials must never ship in a package — use a placeholder and \
             configure the vault after install"
        ));
    }
    Ok(())
}

fn validate_api_client_seed(rel: &str, v: &Value) -> Result<(), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{rel}: api-client seed must be an object"))?;
    for field in ["client_key", "base_url", "ops"] {
        if !obj.contains_key(field) {
            return Err(format!("{rel}: api-client seed missing '{field}'"));
        }
    }
    if obj.contains_key("credentials") && !v["credentials"].is_null() {
        return Err(format!(
            "{rel}: credentials must never ship in a package — use a placeholder and \
             configure the vault after install"
        ));
    }
    Ok(())
}

fn validate_option_seed(rel: &str, v: &Value) -> Result<(), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{rel}: option seed must be an object"))?;
    if !obj.contains_key("key") || !obj.contains_key("value") {
        return Err(format!("{rel}: option seed needs 'key' and 'value'"));
    }
    Ok(())
}

fn validate_role_seed(rel: &str, v: &Value) -> Result<(), String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{rel}: role seed must be an object"))?;
    if !obj.contains_key("name") {
        return Err(format!("{rel}: role seed needs 'name'"));
    }
    if let Some(perms) = obj.get("permissions")
        && !perms.is_array()
    {
        return Err(format!("{rel}: role 'permissions' must be an array"));
    }
    Ok(())
}

fn ct_seed_payload(rel: &str, v: &Value) -> Result<CtSeedPayload, String> {
    let obj = v
        .as_object()
        .ok_or_else(|| format!("{rel}: CT seed must be an object {{\"table\", \"rows\"}}"))?;
    let table = obj
        .get("table")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{rel}: CT seed missing 'table'"))?;
    let rows = obj
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{rel}: CT seed missing 'rows' array"))?;
    for (i, row) in rows.iter().enumerate() {
        let has_key = row.as_object().is_some_and(|o| o.contains_key("seed_key"));
        if !has_key {
            return Err(format!(
                "{rel}: rows[{i}] missing 'seed_key' (idempotency key)"
            ));
        }
    }
    Ok(CtSeedPayload {
        table: table.to_string(),
        rows: rows.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buf);
            let opts: zip::write::SimpleFileOptions = Default::default();
            for (name, data) in entries {
                writer.start_file(*name, opts).expect("start");
                std::io::Write::write_all(&mut writer, data).expect("write");
            }
            writer.finish().expect("finish");
        }
        buf.into_inner()
    }

    fn hash_line(path: &str, data: &[u8]) -> String {
        format!("{}  {}\n", hash_bytes(data), path)
    }

    const APP_TOML: &str = "[app]\nid = \"demo-app\"\nname = \"Demo\"\nversion = \"0.1.0\"\n\
         permissions = [\"content-types:rw\"]\n";

    #[test]
    fn unpack_valid_bundle() {
        let dir = tempfile::tempdir().expect("tempdir");
        let manifest = hash_line("app.toml", APP_TOML.as_bytes());
        let ct = "[content_type]\nname = \"Item\"\nsingular = \"item\"\nplural = \"items\"\n\
             table = \"demo_app_items\"\ngroup = \"demo-app\"\n\n[fields.title]\ntype = \"text\"\n";
        let ct_hash = hash_line("content-types/item.toml", ct.as_bytes());
        let bytes = build_zip(&[
            ("app.toml", APP_TOML.as_bytes()),
            ("content-types/item.toml", ct.as_bytes()),
            (
                "META/manifest.sha256",
                format!("{manifest}{ct_hash}").as_bytes(),
            ),
        ]);
        let pkg = AppPackage::unpack(&bytes, dir.path()).expect("unpack");
        assert_eq!(pkg.manifest.app.id, "demo-app");
        assert_eq!(pkg.content_types.len(), 1);
        assert_eq!(pkg.content_types[0].schema.table, "demo_app_items");
    }

    #[test]
    fn rejects_zip_slip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let bytes = build_zip(&[("../evil.sh", b"boom")]);
        let err = AppPackage::unpack(&bytes, dir.path()).expect_err("zip-slip");
        assert!(err.to_string().contains("illegal path"));
        assert!(!dir.path().parent().unwrap().join("evil.sh").exists());
    }

    #[test]
    fn rejects_tampered_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = hash_line("app.toml", APP_TOML.as_bytes());
        let bytes = build_zip(&[
            ("app.toml", b"TAMPERED"),
            ("META/manifest.sha256", good.as_bytes()),
        ]);
        let err = AppPackage::unpack(&bytes, dir.path()).expect_err("tampered");
        assert!(
            err.to_string().contains("hash mismatch"),
            "expected hash mismatch, got: {err}"
        );
    }

    #[test]
    fn rejects_credentials_in_channel_seed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let channel = r#"{"channel_key":"w","provider":"p","mode":"push","transport":"http1","framing":"raw","codec":"json","verify_kind":"token","target_type":"t","credentials":{"secret":"x"}}"#;
        let manifest = format!(
            "{}{}",
            hash_line("app.toml", APP_TOML.as_bytes()),
            hash_line("channels/w.json", channel.as_bytes())
        );
        let bytes = build_zip(&[
            ("app.toml", APP_TOML.as_bytes()),
            ("channels/w.json", channel.as_bytes()),
            ("META/manifest.sha256", manifest.as_bytes()),
        ]);
        let err = AppPackage::unpack(&bytes, dir.path()).expect_err("credentials");
        assert!(err.to_string().contains("credentials must never ship"));
    }

    #[test]
    fn rejects_seed_row_without_seed_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seed = r#"{"table":"demo_app_items","rows":[{"title":"x"}]}"#;
        let manifest = format!(
            "{}{}",
            hash_line("app.toml", APP_TOML.as_bytes()),
            hash_line("seeds/items.json", seed.as_bytes())
        );
        let bytes = build_zip(&[
            ("app.toml", APP_TOML.as_bytes()),
            ("seeds/items.json", seed.as_bytes()),
            ("META/manifest.sha256", manifest.as_bytes()),
        ]);
        let err = AppPackage::unpack(&bytes, dir.path()).expect_err("seed_key");
        assert!(err.to_string().contains("seed_key"));
    }
}
