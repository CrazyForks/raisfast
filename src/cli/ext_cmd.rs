//! Extension CLI 子命令。
//!
//! 提供 `ext new`（生成模板）、`ext check`（校验）、`ext routes`（预览路由）。

use std::path::{Path, PathBuf};

use rust_blog::config::app::AppConfig;

struct CheckResult {
    errors: usize,
    warnings: usize,
}

/// `ext new <id>` — 从模板生成扩展目录
pub fn create_new(config: &AppConfig, id: &str, runtime: &str) -> anyhow::Result<()> {
    let ext_dir = PathBuf::from(&config.extension_dir).join(id);
    if ext_dir.exists() {
        anyhow::bail!("extension directory already exists: {}", ext_dir.display());
    }

    std::fs::create_dir_all(ext_dir.join("content_types"))?;
    std::fs::create_dir_all(ext_dir.join("plugin"))?;

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
    tera.add_raw_template("extension.toml", include_str!("templates/extension.toml"))?;
    tera.add_raw_template(
        "plugin_manifest.toml",
        include_str!("templates/plugin_manifest.toml"),
    )?;
    tera.add_raw_template("ct_example.toml", include_str!("templates/ct_example.toml"))?;

    let extension_toml = tera.render("extension.toml", &ctx)?;
    std::fs::write(ext_dir.join("extension.toml"), &extension_toml)?;

    let plugin_manifest = tera.render("plugin_manifest.toml", &ctx)?;
    std::fs::write(ext_dir.join("plugin/manifest.toml"), &plugin_manifest)?;

    let example_ct = tera.render("ct_example.toml", &ctx)?;
    std::fs::write(ext_dir.join("content_types/example.toml"), &example_ct)?;

    match runtime {
        "lua" => {
            std::fs::write(
                ext_dir.join("plugin/init.lua"),
                include_str!("templates/plugin_init.lua"),
            )?;
        }
        "js" => {
            std::fs::write(
                ext_dir.join("plugin/main.js"),
                include_str!("templates/plugin_main.js"),
            )?;
        }
        _ => {}
    }

    println!("✓ extension created: {}", ext_dir.display());
    println!();
    println!("  {}/", id);
    println!("  ├── extension.toml");
    println!("  ├── content_types/");
    println!("  │   └── example.toml");
    println!("  └── plugin/");
    println!("      ├── manifest.toml");
    println!("      └── {entry_name}");
    println!();
    println!("edit extension.toml and start building!");
    println!("edit extension.toml and start building!");

    Ok(())
}

/// `ext check [path]` — 校验扩展配置
pub fn check(config: &AppConfig, target: Option<&str>) -> anyhow::Result<()> {
    let ext_dir = match target {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(&config.extension_dir),
    };

    if !ext_dir.exists() {
        anyhow::bail!("directory not found: {}", ext_dir.display());
    }

    let mut total = CheckResult {
        errors: 0,
        warnings: 0,
    };

    if ext_dir.join("extension.toml").exists() {
        let r = check_single_extension(&ext_dir);
        total.errors += r.errors;
        total.warnings += r.warnings;
    } else {
        let mut count = 0;
        for entry in std::fs::read_dir(&ext_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() && entry.path().join("extension.toml").exists() {
                let r = check_single_extension(&entry.path());
                total.errors += r.errors;
                total.warnings += r.warnings;
                count += 1;
            }
        }
        if count == 0 {
            anyhow::bail!("no extensions found in: {}", ext_dir.display());
        }
    }

    println!();
    if total.errors > 0 {
        println!(
            "✗ found {} error(s), {} warning(s)",
            total.errors, total.warnings
        );
    } else if total.warnings > 0 {
        println!("✓ check passed with {} warning(s)", total.warnings);
    } else {
        println!("✓ all checks passed");
    }

    if total.errors > 0 {
        anyhow::bail!("validation failed");
    }

    Ok(())
}

fn check_single_extension(dir: &Path) -> CheckResult {
    let mut result = CheckResult {
        errors: 0,
        warnings: 0,
    };

    let ext_id = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    println!("checking: {ext_id}/");

    let manifest_path = dir.join("extension.toml");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            println!("  ✗ cannot read extension.toml: {e}");
            result.errors += 1;
            return result;
        }
    };

    let toml_val: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            println!("  ✗ extension.toml parse error: {e}");
            result.errors += 1;
            return result;
        }
    };

    let ext_table = match toml_val.get("extension") {
        Some(t) => t,
        None => {
            println!("  ✗ missing [extension] section");
            result.errors += 1;
            return result;
        }
    };

    for key in &["id", "name", "version"] {
        if ext_table.get(*key).is_none() {
            println!("  ✗ extension.{key} is required");
            result.errors += 1;
        }
    }

    if let Some(id_val) = ext_table.get("id").and_then(|v| v.as_str())
        && id_val != ext_id
    {
        println!("  ⚠ extension.id ({id_val}) != directory name ({ext_id})");
        result.warnings += 1;
    }

    if let Some(ct_dir_rel) = ext_table.get("content_types").and_then(|v| v.as_str()) {
        let ct_dir = dir.join(ct_dir_rel);
        if ct_dir.exists() {
            let r = check_content_types(&ct_dir);
            result.errors += r.errors;
            result.warnings += r.warnings;
        } else {
            println!("  ⚠ content_types directory not found: {ct_dir_rel}");
            result.warnings += 1;
        }
    }

    if let Some(plugin_rel) = ext_table.get("plugin").and_then(|v| v.as_str()) {
        let plugin_manifest = dir.join(plugin_rel);
        if plugin_manifest.exists() {
            let r = check_plugin_manifest(&plugin_manifest);
            result.errors += r.errors;
            result.warnings += r.warnings;
        } else {
            println!("  ⚠ plugin manifest not found: {plugin_rel}");
            result.warnings += 1;
        }
    }

    result
}

fn check_content_types(ct_dir: &Path) -> CheckResult {
    let mut result = CheckResult {
        errors: 0,
        warnings: 0,
    };
    let Ok(entries) = std::fs::read_dir(ct_dir) else {
        return result;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    println!("  ✗ cannot read {}: {e}", path.display());
                    result.errors += 1;
                    continue;
                }
            };

            match rust_blog::content_type::schema::ContentTypeSchema::parse_from_str(&content) {
                Ok(ct) => {
                    println!(
                        "  ✓ content_type: {} (table={}, fields={})",
                        ct.name,
                        ct.table,
                        ct.fields.len()
                    );
                }
                Err(e) => {
                    println!(
                        "  ✗ content_type parse error in {}: {e}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    result.errors += 1;
                }
            }
        }
    }

    result
}

fn check_plugin_manifest(path: &Path) -> CheckResult {
    let mut result = CheckResult {
        errors: 0,
        warnings: 0,
    };

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            println!("  ✗ cannot read {}: {e}", path.display());
            result.errors += 1;
            return result;
        }
    };

    match toml::from_str::<rust_blog::plugins::PluginManifest>(&content) {
        Ok(m) => {
            let route_count = m.routes.len();
            let hook_count = m.hooks.len();
            let db_perms = m.permissions.database.len();
            println!(
                "  ✓ plugin: {} (runtime={}, routes={}, hooks={}, db_perms={})",
                m.plugin.id, m.plugin.runtime, route_count, hook_count, db_perms,
            );

            for route in &m.routes {
                if route.method.is_empty() || route.path.is_empty() {
                    println!("  ✗ route has empty method or path");
                    result.errors += 1;
                }
                if route.handler.is_empty() {
                    println!(
                        "  ✗ route {} {}: handler is empty",
                        route.method, route.path
                    );
                    result.errors += 1;
                }
            }
        }
        Err(e) => {
            println!("  ✗ plugin manifest parse error: {e}");
            result.errors += 1;
        }
    }

    result
}

/// `ext routes [path]` — 预览扩展注册的路由
pub fn routes(config: &AppConfig, target: Option<&str>) -> anyhow::Result<()> {
    let ext_dir = match target {
        Some(t) => PathBuf::from(t),
        None => PathBuf::from(&config.extension_dir),
    };

    if !ext_dir.exists() {
        anyhow::bail!("directory not found: {}", ext_dir.display());
    }

    if ext_dir.join("extension.toml").exists() {
        show_extension_routes(&ext_dir);
    } else {
        let mut count = 0;
        for entry in std::fs::read_dir(&ext_dir)?.flatten() {
            if entry.file_type()?.is_dir() && entry.path().join("extension.toml").exists() {
                show_extension_routes(&entry.path());
                count += 1;
            }
        }
        if count == 0 {
            anyhow::bail!("no extensions found in: {}", ext_dir.display());
        }
    }

    Ok(())
}

fn show_extension_routes(dir: &Path) {
    let ext_id = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let ext_manifest = dir.join("extension.toml");
    let Ok(content) = std::fs::read_to_string(&ext_manifest) else {
        return;
    };
    let Ok(toml_val) = toml::from_str::<toml::Value>(&content) else {
        return;
    };

    let ext_table = match toml_val.get("extension") {
        Some(t) => t,
        None => return,
    };

    let ct_routes = collect_ct_routes(dir, ext_table);
    let plugin_routes = collect_plugin_routes(dir, ext_table);

    if ct_routes.is_empty() && plugin_routes.is_empty() {
        println!("{ext_id}: no routes");
        return;
    }

    println!("{ext_id}/");
    if !ct_routes.is_empty() {
        println!("  content type routes:");
        for r in &ct_routes {
            println!("    {r}");
        }
    }
    if !plugin_routes.is_empty() {
        println!("  plugin routes:");
        for r in &plugin_routes {
            println!("    {r}");
        }
    }
    println!();
}

fn collect_ct_routes(dir: &Path, ext_table: &toml::Value) -> Vec<String> {
    let mut routes = Vec::new();

    let Some(ct_dir_rel) = ext_table.get("content_types").and_then(|v| v.as_str()) else {
        return routes;
    };
    let ct_dir = dir.join(ct_dir_rel);
    let Ok(entries) = std::fs::read_dir(&ct_dir) else {
        return routes;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(ct) =
                rust_blog::content_type::schema::ContentTypeSchema::parse_from_str(&content)
            {
                let auth_label = |access: rust_blog::content_type::schema::ApiAccess| match access {
                    rust_blog::content_type::schema::ApiAccess::None => "none",
                    rust_blog::content_type::schema::ApiAccess::Public => "public",
                    rust_blog::content_type::schema::ApiAccess::Member => "member",
                    rust_blog::content_type::schema::ApiAccess::Admin => "admin",
                };
                routes.push(format!(
                    "GET    /cms/{}              [list, {}]",
                    ct.plural,
                    auth_label(ct.api.list.access)
                ));
                routes.push(format!(
                    "GET    /cms/{{}}/{}          [get, {}]",
                    ct.plural,
                    auth_label(ct.api.get.access)
                ));
                routes.push(format!(
                    "POST   /cms/{}              [create, {}]",
                    ct.plural,
                    auth_label(ct.api.create.access)
                ));
                routes.push(format!(
                    "PUT    /cms/{{}}/{}          [update, {}]",
                    ct.plural,
                    auth_label(ct.api.update.access)
                ));
                routes.push(format!(
                    "DELETE /cms/{{}}/{}          [delete, {}]",
                    ct.plural,
                    auth_label(ct.api.delete.access)
                ));
            }
        }
    }

    routes
}

fn collect_plugin_routes(dir: &Path, ext_table: &toml::Value) -> Vec<String> {
    let mut routes = Vec::new();

    let Some(plugin_rel) = ext_table.get("plugin").and_then(|v| v.as_str()) else {
        return routes;
    };
    let plugin_manifest = dir.join(plugin_rel);
    let Ok(content) = std::fs::read_to_string(&plugin_manifest) else {
        return routes;
    };
    let Ok(manifest) = toml::from_str::<rust_blog::plugins::PluginManifest>(&content) else {
        return routes;
    };

    for route in &manifest.routes {
        let auth_label = match route.auth {
            rust_blog::content_type::schema::ApiAccess::None => "none",
            rust_blog::content_type::schema::ApiAccess::Public => "public",
            rust_blog::content_type::schema::ApiAccess::Member => "member",
            rust_blog::content_type::schema::ApiAccess::Admin => "admin",
        };
        routes.push(format!(
            "{:6} {:40} → {} [{}]",
            route.method, route.path, route.handler, auth_label
        ));
    }

    routes
}
