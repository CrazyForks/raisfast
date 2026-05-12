//! Content Type CLI subcommand.
//!
//! Provides `ct new` (generate template), `ct check` (validate TOML schema), `ct types` (generate TypeScript types).

use std::path::PathBuf;

use raisfast::config::app::AppConfig;
use raisfast::content_type::schema::{ContentTypeSchema, FieldType};

pub fn create_new(config: &AppConfig, name: &str) -> anyhow::Result<()> {
    let ct_dir = PathBuf::from(&config.content_type_dir);
    if !ct_dir.exists() {
        std::fs::create_dir_all(&ct_dir)?;
    }

    let singular = name.to_lowercase().replace(' ', "_");
    let plural = format!("{singular}s");
    let table = plural.clone();
    let file_path = ct_dir.join(format!("{singular}.toml"));

    if file_path.exists() {
        anyhow::bail!("content type file already exists: {}", file_path.display());
    }

    let ct_name = name
        .split('_')
        .map(|s| {
            let mut c = s.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let toml_content = format!(
        r#"[content_type]
name = "{ct_name}"
singular = "{singular}"
plural = "{plural}"
table = "{table}"
description = ""
implements = ["ownable", "timestampable"]

[fields.name]
type = "text"
required = true
max_length = 200
label = "Name"

[api]
list = "public"
get = "public"
create = "admin"
update = "admin"
delete = "admin"
"#
    );

    std::fs::write(&file_path, &toml_content)?;

    println!("✓ content type created: {}", file_path.display());
    println!();
    println!("  {singular}.toml");
    println!();
    println!("edit the file and restart the server to apply.");

    Ok(())
}

pub fn check(config: &AppConfig, target: Option<&str>) -> anyhow::Result<()> {
    let ct_dir = match target {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(&config.content_type_dir),
    };

    if !ct_dir.exists() {
        anyhow::bail!("directory not found: {}", ct_dir.display());
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut count = 0usize;

    for entry in std::fs::read_dir(&ct_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            count += 1;
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            print!("checking: {file_name} ... ");

            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let doc = content.parse::<toml::Value>();
                    match doc {
                        Ok(val) => {
                            let mut e = 0usize;
                            let mut w = 0usize;

                            if val.get("content_type").is_none() {
                                println!("✗ missing [content_type] section");
                                e += 1;
                            } else {
                                let ct = &val["content_type"];
                                for required in &["name", "singular", "plural", "table"] {
                                    if ct.get(required).is_none() {
                                        println!("✗ missing content_type.{required}");
                                        e += 1;
                                    }
                                }
                                if ct.get("singular").is_some_and(|v| v.as_str() == Some("")) {
                                    println!("⚠ singular is empty");
                                    w += 1;
                                }
                                if ct.get("table").is_some_and(|v| v.as_str() == Some("")) {
                                    println!("⚠ table is empty");
                                    w += 1;
                                }
                            }

                            if val.get("fields").is_none() {
                                println!("⚠ no [fields] defined");
                                w += 1;
                            }

                            if val.get("api").is_none() {
                                println!("⚠ no [api] rules defined");
                                w += 1;
                            }

                            errors += e;
                            warnings += w;

                            if e == 0 && w == 0 {
                                println!("✓");
                            }
                        }
                        Err(err) => {
                            println!("✗ parse error: {err}");
                            errors += 1;
                        }
                    }
                }
                Err(err) => {
                    println!("✗ read error: {err}");
                    errors += 1;
                }
            }
        }
    }

    if count == 0 {
        anyhow::bail!("no .toml files found in: {}", ct_dir.display());
    }

    println!();
    if errors > 0 {
        println!("✗ found {errors} error(s), {warnings} warning(s)");
        anyhow::bail!("validation failed");
    } else if warnings > 0 {
        println!("✓ check passed with {warnings} warning(s)");
    } else {
        println!("✓ all {count} content type(s) passed");
    }

    Ok(())
}

/// Scan all TOML files in the content type directory and generate TypeScript type declarations.
///
/// Each TOML is parsed as a `ContentTypeSchema`, fields are mapped to TS types,
/// and protocol-injected system fields are appended.
pub fn generate_types(
    config: &AppConfig,
    singular: Option<&str>,
    output: Option<&str>,
) -> anyhow::Result<()> {
    let ct_dir = PathBuf::from(&config.content_type_dir);
    if !ct_dir.exists() {
        anyhow::bail!("directory not found: {}", ct_dir.display());
    }

    let target_file = singular.map(|s| format!("{s}.toml"));
    let mut schemas: Vec<ContentTypeSchema> = Vec::new();
    for entry in std::fs::read_dir(&ct_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            if target_file
                .as_ref()
                .is_some_and(|t| path.file_name().is_some_and(|n| n != t.as_str()))
            {
                continue;
            }
            let ct = ContentTypeSchema::parse_from_file(&path)?;
            schemas.push(ct);
        }
    }

    if let Some(s) = singular {
        if schemas.is_empty() {
            anyhow::bail!("content type '{s}' not found (looked for {s}.toml)");
        }
    } else if schemas.is_empty() {
        anyhow::bail!("no .toml files found in: {}", ct_dir.display());
    }

    schemas.sort_by(|a, b| a.singular.cmp(&b.singular));

    let mut out = String::from("// Auto-generated by `ct types`\n// DO NOT EDIT MANUALLY\n\n");

    for ct in &schemas {
        out.push_str(&schema_to_ts(ct));
        out.push('\n');
    }

    match output {
        Some(path) => {
            let out_path = PathBuf::from(path);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out_path, &out)?;
            println!("✓ generated {} type(s) to {}", schemas.len(), path);
        }
        None => print!("{out}"),
    }

    Ok(())
}

fn schema_to_ts(ct: &ContentTypeSchema) -> String {
    let name = pascal_case(&ct.singular);
    let mut lines = Vec::new();
    lines.push(format!("export interface {name} {{"));

    lines.push("  id: string;".into());

    for field in &ct.fields {
        let ts_type = field_type_to_ts(field);
        let nullable = if field.required { "" } else { " | null" };
        lines.push(format!("  {}: {}{nullable};", field.name, ts_type));
    }

    let has_timestamps = ct
        .implements
        .iter()
        .any(|p| p.name() == "timestampable" || p.name() == "ownable");
    if has_timestamps || !ct.builtin {
        lines.push("  created_at: string;".into());
        lines.push("  updated_at: string;".into());
    }

    if ct.implements.iter().any(|p| p.name() == "ownable")
        && !ct.fields.iter().any(|f| f.name == "created_by")
    {
        lines.push("  created_by: string | null;".into());
    }

    if ct.implements.iter().any(|p| p.name() == "ownable")
        && !ct.fields.iter().any(|f| f.name == "updated_by")
    {
        lines.push("  updated_by: string | null;".into());
    }
    if ct.is_soft_delete() && !ct.fields.iter().any(|f| f.name == "deleted_at") {
        lines.push("  deleted_at: string | null;".into());
    }

    if ct.has_revision_routes()
        && !ct
            .fields
            .iter()
            .any(|f| f.name == raisfast::constants::COL_VERSION)
    {
        lines.push("  version: number;".into());
    }

    if ct.implements_protocol("lockable")
        && !ct
            .fields
            .iter()
            .any(|f| f.name == raisfast::constants::COL_LOCK_VERSION)
    {
        lines.push("  lock_version: number;".into());
    }

    lines.push("}".into());
    lines.join("\n")
}

fn field_type_to_ts(field: &raisfast::content_type::schema::FieldSchema) -> String {
    match &field.field_type {
        FieldType::Text | FieldType::RichText | FieldType::Uid => "string".into(),
        FieldType::Email => "string".into(),
        FieldType::Password => "string".into(),
        FieldType::Integer | FieldType::BigInt => "number".into(),
        FieldType::Decimal | FieldType::Float => "number".into(),
        FieldType::Boolean => "boolean".into(),
        FieldType::Date | FieldType::DateTime | FieldType::Time => "string".into(),
        FieldType::Enum => match &field.enum_values {
            Some(vals) if !vals.is_empty() => vals
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(" | "),
            _ => "string".into(),
        },
        FieldType::Json => "Record<string, unknown>".into(),
        FieldType::Media => "string".into(),
        FieldType::Relation => "string".into(),
    }
}

fn pascal_case(s: &str) -> String {
    s.split('_')
        .map(|part| {
            let mut c = part.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect()
}
