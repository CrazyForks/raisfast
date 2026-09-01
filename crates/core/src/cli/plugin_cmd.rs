//! Plugin CLI subcommand.
//!
//! Provides `plugin new` (generate template), `plugin check` (validate manifest),
//! and `plugin types` (generate TypeScript types from `[[routes]]` declarations).

use std::path::PathBuf;

use raisfast::config::app::AppConfig;
use raisfast::plugins::{PluginManifest, RouteOutputField, RouteParam};

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
        include_str!(concat!(
            env!("RAISFAST_ROOT"),
            "/templates/plugin/plugin_manifest.toml"
        )),
    )?;

    let manifest = tera.render("plugin_manifest.toml", &ctx)?;
    std::fs::write(plugin_dir.join("manifest.toml"), &manifest)?;

    match runtime {
        "lua" => {
            std::fs::write(
                plugin_dir.join("init.lua"),
                include_str!(concat!(
                    env!("RAISFAST_ROOT"),
                    "/templates/plugin/plugin_init.lua"
                )),
            )?;
        }
        "js" => {
            std::fs::write(
                plugin_dir.join("main.js"),
                include_str!(concat!(
                    env!("RAISFAST_ROOT"),
                    "/templates/plugin/plugin_main.js"
                )),
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
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(err) => {
            println!("✗ read error: {err}");
            *errors += 1;
            return;
        }
    };

    // Typed parse (same as the plugin loader / `plugin types`): toml::Value
    // rejects manifests containing array-of-table sections ([[jobs]]/[[routes]]).
    let manifest: PluginManifest = match toml::from_str(&content) {
        Ok(m) => m,
        Err(err) => {
            println!("✗ parse error: {err}");
            *errors += 1;
            return;
        }
    };

    let mut e = 0usize;
    let mut w = 0usize;

    let entry = manifest.plugin.entry.as_str();
    if !entry.is_empty() && !dir.join(entry).exists() {
        println!("✗ entry file not found: {entry}");
        e += 1;
    }

    if !["js", "lua", "wasm", "rhai"].contains(&manifest.plugin.runtime.as_str()) {
        println!("⚠ unknown runtime: {}", manifest.plugin.runtime);
        w += 1;
    }

    for job in &manifest.jobs {
        if job.job_type.trim().is_empty() || job.handler.trim().is_empty() {
            println!("✗ [[jobs]] requires non-empty job_type + handler");
            e += 1;
        }
    }

    for route in &manifest.routes {
        if route.handler.trim().is_empty() {
            println!("✗ [[routes]] requires a handler ({})", route.path);
            e += 1;
        }
    }

    *errors += e;
    *warnings += w;

    if e == 0 && w == 0 {
        println!("✓");
    }
}

/// Scan plugin manifests and generate TypeScript types from their `[[routes]]`
/// declarations. Mirrors `ct types` for the plugin route contract: each route's
/// declared `input` params (path/query/header → Params, body → Body) and
/// `output` fields (→ Data) become typed interfaces, collected in a
/// `{Prefix}RouteMap` that a generic schema-driven client can consume.
///
/// Routes without `input`/`output` declarations still appear in the route map
/// with `undefined`/`unknown` payload types — the manifest is the contract, and
/// authors opt into full typing by declaring route params/fields.
pub fn generate_types(
    config: &AppConfig,
    id: Option<&str>,
    output: Option<&str>,
) -> anyhow::Result<()> {
    let plugin_base = config
        .plugin_dir
        .as_deref()
        .unwrap_or("./extensions/plugins");
    let base = PathBuf::from(plugin_base);
    if !base.exists() {
        anyhow::bail!("directory not found: {}", base.display());
    }

    let mut plugin_dirs: Vec<(String, PathBuf)> = Vec::new();
    match id {
        Some(id) => {
            let dir = base.join(id);
            if !dir.join("manifest.toml").exists() {
                anyhow::bail!(
                    "plugin '{id}' not found (looked for {})",
                    dir.join("manifest.toml").display()
                );
            }
            plugin_dirs.push((id.to_string(), dir));
        }
        None => {
            for entry in std::fs::read_dir(&base)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() && entry.path().join("manifest.toml").exists() {
                    let pid = entry.file_name().to_string_lossy().to_string();
                    plugin_dirs.push((pid, entry.path()));
                }
            }
        }
    }

    if plugin_dirs.is_empty() {
        anyhow::bail!("no plugins found in: {}", base.display());
    }

    let mut out =
        String::from("// Auto-generated by `raisfast plugin types`\n// DO NOT EDIT MANUALLY\n\n");

    let mut emitted = 0usize;
    let mut total_routes = 0usize;
    for (pid, dir) in &plugin_dirs {
        let manifest_path = dir.join("manifest.toml");
        let content = std::fs::read_to_string(&manifest_path)?;
        let manifest: PluginManifest = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", manifest_path.display()))?;
        if manifest.routes.is_empty() {
            continue;
        }
        let (section, n) = routes_to_ts(&manifest, pid);
        out.push_str(&section);
        emitted += 1;
        total_routes += n;
    }

    if emitted == 0 {
        anyhow::bail!("no plugins with [[routes]] found in: {}", base.display());
    }

    match output {
        Some(path) => {
            let out_path = PathBuf::from(path);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out_path, &out)?;
            println!(
                "✓ generated types for {emitted} plugin(s) ({total_routes} route(s)) to {path}"
            );
        }
        None => print!("{out}"),
    }

    Ok(())
}

fn routes_to_ts(manifest: &PluginManifest, pid: &str) -> (String, usize) {
    let prefix = pascal_case(pid);
    let mut out = String::new();
    out.push_str(&format!(
        "// ─── {prefix} ({pid}) — {} ───\n\n",
        manifest.plugin.name
    ));

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut map_lines: Vec<String> = Vec::new();
    let mut count = 0usize;

    for route in &manifest.routes {
        let base = unique_name(&route.handler, &mut used);
        let params: Vec<&RouteParam> = route.input.iter().filter(|p| p.r#in != "body").collect();
        let body: Vec<&RouteParam> = route.input.iter().filter(|p| p.r#in == "body").collect();

        let mut doc = format!(
            "/** {} {} ({})",
            route.method.to_uppercase(),
            route.path,
            route.handler
        );
        if let Some(d) = &route.description {
            doc.push_str(&format!("\n * {d}"));
        }
        doc.push_str(" */");
        out.push_str(&doc);
        out.push('\n');

        if !params.is_empty() {
            out.push_str(&interface_from_params(
                &format!("{prefix}{base}Params"),
                &params,
            ));
            out.push('\n');
        }
        if !body.is_empty() {
            out.push_str(&interface_from_params(
                &format!("{prefix}{base}Body"),
                &body,
            ));
            out.push('\n');
        }

        let data_ty = if !route.output.fields.is_empty() {
            out.push_str(&interface_from_fields(
                &format!("{prefix}{base}Data"),
                &route.output.fields,
            ));
            out.push('\n');
            format!("{prefix}{base}Data")
        } else if route.output.content_type.is_some() {
            // Content-type reference: shape lives in the CT TOML — generate with
            // `ct types` and map the row type onto `data` at the call site.
            "Record<string, unknown>".to_string()
        } else {
            "unknown".to_string()
        };

        let params_ty = if params.is_empty() {
            "undefined".to_string()
        } else {
            format!("{prefix}{base}Params")
        };
        let body_ty = if body.is_empty() {
            "undefined".to_string()
        } else {
            format!("{prefix}{base}Body")
        };

        map_lines.push(format!(
            "  \"{} {}\": {{ params: {params_ty}; body: {body_ty}; data: {data_ty} }};",
            route.method.to_uppercase(),
            route.path
        ));
        count += 1;
    }

    out.push_str(&format!("export interface {prefix}RouteMap {{\n"));
    for l in map_lines {
        out.push_str(&l);
        out.push('\n');
    }
    out.push_str("}\n\n");

    (out, count)
}

fn interface_from_params(name: &str, params: &[&RouteParam]) -> String {
    let mut out = String::from("export interface ");
    out.push_str(name);
    out.push_str(" {\n");
    for p in params {
        let ts_type = param_type_to_ts(p);
        let opt = if p.required { "" } else { "?" };
        out.push_str(&format!("  {}{opt}: {ts_type};\n", p.name));
    }
    out.push_str("}\n");
    out
}

fn interface_from_fields(name: &str, fields: &[RouteOutputField]) -> String {
    let mut out = String::from("export interface ");
    out.push_str(name);
    out.push_str(" {\n");
    for f in fields {
        let ts_type = field_type_to_ts(&f.r#type);
        out.push_str(&format!("  {}: {ts_type};\n", f.name));
    }
    out.push_str("}\n");
    out
}

/// Map a declared route input type to a TS type. `path`/`header` params are URL
/// segments → always `string`; everything else follows the `ct types` mapping.
fn param_type_to_ts(p: &RouteParam) -> String {
    if p.r#in == "path" || p.r#in == "header" {
        return "string".to_string();
    }
    field_type_to_ts(&p.r#type)
}

/// Map a declaration type string to a TS type (mirrors `ct_cmd::field_type_to_ts`).
fn field_type_to_ts(ty: &str) -> String {
    match ty {
        "string" | "text" | "email" | "uid" | "date" | "datetime" | "time" => "string".to_string(),
        "integer" | "bigint" | "number" | "float" | "decimal" => "number".to_string(),
        "boolean" | "bool" => "boolean".to_string(),
        "json" | "object" => "Record<string, unknown>".to_string(),
        "array" => "unknown[]".to_string(),
        "" => "string".to_string(),
        _ => "unknown".to_string(),
    }
}

/// PascalCase a plugin id / handler into an identifier prefix. Non-alphanumeric
/// characters (`.`, `-`, `_`) split words; each word is capitalized.
fn pascal_case(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect()
}

fn unique_name(handler: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base = pascal_case(handler);
    let mut candidate = base.clone();
    let mut n = 2u32;
    while !used.insert(candidate.clone()) {
        candidate = format!("{base}{n}");
        n += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> PluginManifest {
        let toml = r#"
[plugin]
id = "com.example.chat"
name = "Chat"
version = "0.1.0"
runtime = "js"
entry = "main.js"

[[routes]]
method = "POST"
path = "/api/v1/plugins/chat/conversations/:id/messages"
handler = "sendMessage"
description = "Send a message"

[[routes.input]]
name = "id"
in = "path"
type = "string"
required = true

[[routes.input]]
name = "body"
in = "body"
type = "string"
required = true

[[routes.input]]
name = "private"
in = "body"
type = "boolean"

[[routes.input]]
name = "page"
in = "query"
type = "integer"

[routes.output]
description = "Created message"

[[routes.output.fields]]
name = "id"
type = "string"

[[routes]]
method = "GET"
path = "/api/v1/plugins/chat/conversations"
handler = "listConversations"

[routes.output]
content_type = "chat/conversations"
"#;
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn routes_to_ts_emits_typed_interfaces() {
        let manifest = sample_manifest();
        let (out, count) = routes_to_ts(&manifest, "com.example.chat");
        assert_eq!(count, 2);
        // Interface names are derived from the handler, PascalCased with the
        // plugin-id prefix; path params are strings, query/body follow type mapping.
        assert!(out.contains("export interface ComExampleChatSendMessageParams {"));
        assert!(out.contains("  id: string;"));
        assert!(out.contains("  page?: number;"));
        assert!(out.contains("export interface ComExampleChatSendMessageBody {"));
        assert!(out.contains("  body: string;"));
        assert!(out.contains("  private?: boolean;"));
        assert!(out.contains("export interface ComExampleChatSendMessageData {"));
        assert!(out.contains("  id: string;"));
        // content_type reference routes fall back to Record<string, unknown>.
        assert!(out.contains("data: Record<string, unknown>"));
        assert!(out.contains("export interface ComExampleChatRouteMap {"));
        assert!(out.contains(
            "\"POST /api/v1/plugins/chat/conversations/:id/messages\": { params: ComExampleChatSendMessageParams; body: ComExampleChatSendMessageBody; data: ComExampleChatSendMessageData };"
        ));
    }

    #[test]
    fn routes_without_declarations_emit_unknown_payloads() {
        let manifest: PluginManifest = toml::from_str(
            r#"
[plugin]
id = "chat"
name = "Chat"
version = "0.1.0"
runtime = "js"
entry = "main.js"

[[routes]]
method = "GET"
path = "/api/v1/plugins/chat/ping"
handler = "ping"
"#,
        )
        .unwrap();
        let (out, count) = routes_to_ts(&manifest, "chat");
        assert_eq!(count, 1);
        assert!(!out.contains("export interface ChatPingData"));
        assert!(out.contains("data: unknown"));
        assert!(out.contains(
            "\"GET /api/v1/plugins/chat/ping\": { params: undefined; body: undefined; data: unknown };"
        ));
    }
}
