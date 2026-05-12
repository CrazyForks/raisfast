//! Plugin CLI subcommand.
//!
//! Provides `plugin new` (generate template), `plugin check` (validate manifest).

use std::path::PathBuf;

use raisfast::config::app::AppConfig;

pub fn create_new(config: &AppConfig, id: &str, runtime: &str) -> anyhow::Result<()> {
    let plugin_base = config.plugin_dir.as_deref().unwrap_or("./plugins");
    let plugin_dir = PathBuf::from(plugin_base).join(id);

    if plugin_dir.exists() {
        anyhow::bail!("plugin directory already exists: {}", plugin_dir.display());
    }

    std::fs::create_dir_all(&plugin_dir)?;

    let entry_name = match runtime {
        "lua" => "init.lua",
        "js" => "main.js",
        _ => "plugin.wasm",
    };

    let mut ctx = tera::Context::new();
    ctx.insert("id", id);
    ctx.insert("name", id);
    ctx.insert("version", "0.1.0");
    ctx.insert("description", "");
    ctx.insert("author", "");
    ctx.insert("license", "MIT");
    ctx.insert("plugin_id", &format!("com.example.{id}"));
    ctx.insert("runtime", runtime);
    ctx.insert("entry", entry_name);
    ctx.insert("max_memory_mb", &16);
    ctx.insert("timeout_ms", &5000);

    let mut tera = tera::Tera::default();
    tera.add_raw_template(
        "plugin_manifest.toml",
        include_str!("../../templates/plugin/plugin_manifest.toml"),
    )?;

    let manifest = tera.render("plugin_manifest.toml", &ctx)?;
    std::fs::write(plugin_dir.join("manifest.toml"), &manifest)?;

    match runtime {
        "lua" => {
            std::fs::write(
                plugin_dir.join("init.lua"),
                include_str!("../../templates/plugin/plugin_init.lua"),
            )?;
        }
        "js" => {
            std::fs::write(
                plugin_dir.join("main.js"),
                include_str!("../../templates/plugin/plugin_main.js"),
            )?;
        }
        _ => {}
    }

    println!("✓ plugin created: {}", plugin_dir.display());
    println!();
    println!("  {id}/");
    println!("  ├── manifest.toml");
    println!("  └── {entry_name}");
    println!();
    println!("edit manifest.toml and start building!");

    Ok(())
}

pub fn check(config: &AppConfig, target: Option<&str>) -> anyhow::Result<()> {
    let plugin_base = config.plugin_dir.as_deref().unwrap_or("./plugins");
    let plugin_dir = match target {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(plugin_base),
    };

    if !plugin_dir.exists() {
        anyhow::bail!("directory not found: {}", plugin_dir.display());
    }

    let mut errors = 0usize;
    let mut warnings = 0usize;
    let mut count = 0usize;

    if plugin_dir.join("manifest.toml").exists() {
        count += 1;
        check_single_plugin(&plugin_dir, &mut errors, &mut warnings);
    } else {
        for entry in std::fs::read_dir(&plugin_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("manifest.toml").exists() {
                count += 1;
                check_single_plugin(&entry.path(), &mut errors, &mut warnings);
            }
        }
    }

    if count == 0 {
        anyhow::bail!("no plugins found in: {}", plugin_dir.display());
    }

    println!();
    if errors > 0 {
        println!("✗ found {errors} error(s), {warnings} warning(s)");
        anyhow::bail!("validation failed");
    } else if warnings > 0 {
        println!("✓ check passed with {warnings} warning(s)");
    } else {
        println!("✓ all {count} plugin(s) passed");
    }

    Ok(())
}

fn check_single_plugin(dir: &std::path::Path, errors: &mut usize, warnings: &mut usize) {
    let plugin_id = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    print!("checking: {plugin_id}/ ... ");

    let manifest_path = dir.join("manifest.toml");
    match std::fs::read_to_string(&manifest_path) {
        Ok(content) => match content.parse::<toml::Value>() {
            Ok(val) => {
                let mut e = 0usize;
                let mut w = 0usize;

                if val.get("plugin").is_none() {
                    println!("✗ missing [plugin] section");
                    e += 1;
                } else {
                    let p = &val["plugin"];
                    for required in &["id", "name", "runtime", "entry"] {
                        if p.get(required).is_none() {
                            println!("✗ missing plugin.{required}");
                            e += 1;
                        }
                    }
                }

                if val.get("permissions").is_none() {
                    println!("⚠ no [permissions] section (will use defaults)");
                    w += 1;
                }

                let runtime = val
                    .get("plugin")
                    .and_then(|p| p.get("runtime"))
                    .and_then(|r| r.as_str())
                    .unwrap_or("");
                let entry = val
                    .get("plugin")
                    .and_then(|p| p.get("entry"))
                    .and_then(|e| e.as_str())
                    .unwrap_or("");

                if !entry.is_empty() && !dir.join(entry).exists() {
                    println!("✗ entry file not found: {entry}");
                    e += 1;
                }

                if !["js", "lua", "wasm"].contains(&runtime) && !runtime.is_empty() {
                    println!("⚠ unknown runtime: {runtime}");
                    w += 1;
                }

                *errors += e;
                *warnings += w;

                if e == 0 && w == 0 {
                    println!("✓");
                }
            }
            Err(err) => {
                println!("✗ parse error: {err}");
                *errors += 1;
            }
        },
        Err(err) => {
            println!("✗ read error: {err}");
            *errors += 1;
        }
    }
}
