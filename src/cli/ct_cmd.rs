//! Content Type CLI 子命令。
//!
//! 提供 `ct new`（生成模板）和 `ct check`（校验 TOML schema）。

use std::path::PathBuf;

use rust_blog::config::app::AppConfig;

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
draft_publish = false
timestamps = true
soft_delete = false

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
