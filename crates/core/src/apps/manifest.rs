//! App Bundle manifest (`app.toml`) parsing and semantic validation
//! (app-bundle.md §2.2-2.3).

use serde::{Deserialize, Serialize};

/// Reserved app ids that may never be installed (host namespaces).
pub const RESERVED_APP_IDS: &[&str] = &[
    "core",
    "admin",
    "api",
    "raisfast",
    "apps",
    "app",
    "integration",
    "system",
    "public",
    "cms",
    "media",
    "plugins",
];

/// Permission domains an app may request (app-bundle.md §2.3).
pub const PERMISSION_DOMAINS: &[&str] = &[
    "content-types",
    "http",
    "cron",
    "ingress",
    "vault",
    "admin-pages",
];

/// Top-level `app.toml` structure.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppBundleManifest {
    pub app: AppSection,
    #[serde(default)]
    pub dependencies: DependenciesSection,
    #[serde(default)]
    pub install: InstallSection,
}

/// `[app]` section — identity and the permission request list.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppSection {
    /// Globally unique, kebab-case, immutable after install.
    pub id: String,
    pub name: String,
    /// Strict semver — the upgrade discriminator.
    pub version: String,
    /// Platform version constraint (`"1.2"`, `">=1.2"`, `">1.2"`, `"<1.2"`).
    #[serde(rename = "requires-raisfast", default)]
    pub requires_raisfast: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Permission request list shown to the installer (`domain[:resource]`).
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// `[dependencies]` — hard dependencies only (app-bundle.md §7).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DependenciesSection {
    pub requires: Option<RequireDecl>,
}

/// One hard dependency: `{ app, version }`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RequireDecl {
    pub app: String,
    pub version: Option<String>,
}

/// `[install]` — install-time behavior knobs.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstallSection {
    /// Optional short table prefix (e.g. `"cw"` → `cw_conversations`).
    /// Defaults to the app id with `-` → `_`.
    #[serde(rename = "table-prefix", default)]
    pub table_prefix: Option<String>,
    /// `global` | `per-tenant`.
    #[serde(rename = "tenant-scope", default = "default_tenant_scope")]
    pub tenant_scope: String,
    /// Uninstall keeps data by default; the wizard may override.
    #[serde(rename = "uninstall-keep-data", default = "default_keep_data")]
    pub uninstall_keep_data: bool,
    /// `migrate` | `reset` (upgrade strategy — B2).
    #[serde(rename = "upgrade-strategy", default = "default_upgrade_strategy")]
    pub upgrade_strategy: String,
}

fn default_tenant_scope() -> String {
    "global".into()
}

fn default_keep_data() -> bool {
    true
}

fn default_upgrade_strategy() -> String {
    "migrate".into()
}

impl Default for InstallSection {
    fn default() -> Self {
        Self {
            table_prefix: None,
            tenant_scope: default_tenant_scope(),
            uninstall_keep_data: default_keep_data(),
            upgrade_strategy: default_upgrade_strategy(),
        }
    }
}

impl AppBundleManifest {
    /// Parse and validate `app.toml` content. Returns every validation error
    /// at once (precheck philosophy: full list, no drip-feed).
    pub fn parse_and_validate(content: &str) -> Result<Self, Vec<String>> {
        let manifest: Self =
            toml::from_str(content).map_err(|e| vec![format!("app.toml parse error: {e}")])?;
        let errors = manifest.validate();
        if errors.is_empty() {
            Ok(manifest)
        } else {
            Err(errors)
        }
    }

    /// Semantic validation (§2.3). Collects all violations.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        let id = self.app.id.trim();
        if id.is_empty()
            || !id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            || !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            errors.push(format!("app.id '{id}' must match ^[a-z][a-z0-9-]*$"));
        }
        if RESERVED_APP_IDS.contains(&id) {
            errors.push(format!("app.id '{id}' is a reserved word"));
        }

        if parse_semver(&self.app.version).is_none() {
            errors.push(format!(
                "app.version '{}' is not strict semver (major.minor.patch)",
                self.app.version
            ));
        }

        if let Some(req) = &self.app.requires_raisfast
            && parse_requirement(req).is_none()
        {
            errors.push(format!(
                "requires-raisfast '{req}' is invalid (supported: x.y, =x.y, >x.y, >=x.y, <x.y)"
            ));
        }

        for perm in &self.app.permissions {
            if let Err(reason) = validate_permission(perm) {
                errors.push(reason);
            }
        }

        match self.install.tenant_scope.as_str() {
            "global" | "per-tenant" => {}
            other => errors.push(format!(
                "install.tenant-scope '{other}' must be 'global' or 'per-tenant'"
            )),
        }

        if !matches!(self.install.upgrade_strategy.as_str(), "migrate" | "reset") {
            errors.push(format!(
                "install.upgrade-strategy '{}' must be 'migrate' or 'reset'",
                self.install.upgrade_strategy
            ));
        }

        if let Some(prefix) = &self.install.table_prefix
            && !crate::db::driver::sanitize_identifier(prefix).is_some_and(|p| !p.is_empty())
        {
            errors.push(format!(
                "install.table-prefix '{prefix}' must be alphanumeric/underscore"
            ));
        }

        errors
    }

    /// Effective CT table prefix: declared short prefix or the app id
    /// normalized to a snake_case identifier (e.g. `chatwoot-lite` →
    /// `chatwoot_lite_`). Every app CT table must start with it (§9).
    #[must_use]
    pub fn table_prefix(&self) -> String {
        let base = self
            .install
            .table_prefix
            .clone()
            .unwrap_or_else(|| self.app.id.replace('-', "_"));
        format!("{base}_")
    }

    /// Effective plugin id for a packaged plugin name: `{app_id}/{name}`.
    #[must_use]
    pub fn plugin_id(&self, plugin_name: &str) -> String {
        format!("{}/{}", self.app.id, plugin_name)
    }
}

/// Validate one permission string: `domain[:resource]`, domain-level `*`
/// wildcard allowed, resource may be `*`.
fn validate_permission(perm: &str) -> Result<(), String> {
    let perm = perm.trim();
    let (domain, resource) = match perm.split_once(':') {
        Some((d, r)) => (d, Some(r)),
        None => (perm, None),
    };
    if !PERMISSION_DOMAINS.contains(&domain) {
        return Err(format!(
            "permission '{perm}': unknown domain '{domain}' (allowed: {PERMISSION_DOMAINS:?})"
        ));
    }
    if let Some(res) = resource
        && res.is_empty()
    {
        return Err(format!("permission '{perm}': empty resource after ':'"));
    }
    Ok(())
}

/// Check whether a required permission is covered by the declared list
/// (exact match, or domain wildcard `domain:*`).
#[must_use]
pub fn permission_covers(declared: &[String], needed: &str) -> bool {
    let needed_domain = needed.split(':').next().unwrap_or(needed);
    declared.iter().any(|d| {
        let d = d.trim();
        d == needed || d.strip_suffix(":*").is_some_and(|dom| dom == needed_domain)
    })
}

/// A parsed semver triple.
fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim();
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// A version requirement: `(op, (major, minor))`.
fn parse_requirement(req: &str) -> Option<(RequirementOp, (u64, u64))> {
    let req = req.trim();
    let (op, version) = if let Some(rest) = req.strip_prefix(">=") {
        (RequirementOp::Gte, rest)
    } else if let Some(rest) = req.strip_prefix('>') {
        (RequirementOp::Gt, rest)
    } else if let Some(rest) = req.strip_prefix("<=") {
        (RequirementOp::Lte, rest)
    } else if let Some(rest) = req.strip_prefix('<') {
        (RequirementOp::Lt, rest)
    } else if let Some(rest) = req.strip_prefix('=') {
        (RequirementOp::Eq, rest)
    } else {
        (RequirementOp::Gte, req)
    };
    let version = version.trim();
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor: u64 = parts.next().map_or(Ok(0), str::parse).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((op, (major, minor)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequirementOp {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
}

/// Check a requirement like `">=1.2"` against an actual version string.
/// Minor segment defaults to 0 when omitted (`"1"` ≡ `"1.0"`).
#[must_use]
pub fn check_requirement(req: &str, actual: &str) -> bool {
    let Some((op, (r_major, r_minor))) = parse_requirement(req) else {
        return false;
    };
    let Some((a_major, a_minor, _)) = parse_semver(actual) else {
        return false;
    };
    let ord = (a_major, a_minor).cmp(&(r_major, r_minor));
    match op {
        RequirementOp::Gt => ord == std::cmp::Ordering::Greater,
        RequirementOp::Gte => ord != std::cmp::Ordering::Less,
        RequirementOp::Lt => ord == std::cmp::Ordering::Less,
        RequirementOp::Lte => ord != std::cmp::Ordering::Greater,
        RequirementOp::Eq => ord == std::cmp::Ordering::Equal,
    }
}

/// Compare two semver strings (`Err` on malformed input).
pub fn cmp_semver(a: &str, b: &str) -> Result<std::cmp::Ordering, String> {
    let pa = parse_semver(a).ok_or_else(|| format!("invalid semver '{a}'"))?;
    let pb = parse_semver(b).ok_or_else(|| format!("invalid semver '{b}'"))?;
    Ok(pa.cmp(&pb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_toml(id: &str, version: &str) -> String {
        format!(
            "[app]\nid = \"{id}\"\nname = \"Test\"\nversion = \"{version}\"\npermissions = \
             [\"content-types:rw\"]\n"
        )
    }

    #[test]
    fn parses_minimal_manifest() {
        let m = AppBundleManifest::parse_and_validate(&manifest_toml("chatwoot-lite", "0.3.1"))
            .expect("valid");
        assert_eq!(m.app.id, "chatwoot-lite");
        assert!(m.install.uninstall_keep_data);
        assert_eq!(m.install.tenant_scope, "global");
        assert_eq!(m.table_prefix(), "chatwoot_lite_");
    }

    #[test]
    fn rejects_reserved_and_malformed_ids() {
        let errors = AppBundleManifest::parse_and_validate(&manifest_toml("admin", "0.1.0"))
            .expect_err("reserved");
        assert!(errors.iter().any(|e| e.contains("reserved")));

        let errors = AppBundleManifest::parse_and_validate(&manifest_toml("1bad-ID", "0.1.0"))
            .expect_err("bad id");
        assert!(errors.iter().any(|e| e.contains("must match")));

        let errors = AppBundleManifest::parse_and_validate(&manifest_toml("good-id", "0.1"))
            .expect_err("bad version");
        assert!(errors.iter().any(|e| e.contains("semver")));
    }

    #[test]
    fn rejects_unknown_permission_domain() {
        let toml =
            manifest_toml("demo-app", "1.0.0").replace("\"content-types:rw\"", "\"fs:/etc/*\"");
        let errors = AppBundleManifest::parse_and_validate(&toml).expect_err("bad perm");
        assert!(errors.iter().any(|e| e.contains("unknown domain")));
    }

    #[test]
    fn permission_covering_rules() {
        let declared = vec!["http:*".to_string(), "cron:*".to_string()];
        assert!(permission_covers(&declared, "http:api.dify.internal"));
        assert!(permission_covers(&declared, "cron:ingress.pull"));
        assert!(!permission_covers(&declared, "vault:secrets"));
    }

    #[test]
    fn requirement_checks() {
        assert!(check_requirement(">=1.2", "1.2.0"));
        assert!(check_requirement(">=1.2", "2.0.0"));
        assert!(!check_requirement(">=1.2", "1.1.9"));
        assert!(check_requirement("1.2", "1.2.5"));
        assert!(check_requirement(">1.2", "1.3.0"));
        assert!(!check_requirement(">1.2", "1.2.0"));
        assert!(check_requirement("<2", "1.9.0"));
        assert!(!check_requirement("<2", "2.0.0"));
    }

    #[test]
    fn table_prefix_override() {
        let toml = "[app]\nid = \"chatwoot-lite\"\nname = \"t\"\nversion = \"0.1.0\"\n\n[install]\ntable-prefix = \"cw\"\n";
        let m = AppBundleManifest::parse_and_validate(toml).expect("valid");
        assert_eq!(m.table_prefix(), "cw_");
    }
}
