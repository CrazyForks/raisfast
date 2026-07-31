//! Dynamic content type engine
//!
//! Provides the schema-driven content management system core:
//! - Parse content type definitions from TOML files
//! - Automatically generate database migrations
//! - Generic CRUD Repository and API Handler
//! - Field validation and relation resolution
//!
//! # Design Reference
//!
//! - Strapi v5 Content Type Builder
//!
//! # Usage Flow
//!
//! 1. Create TOML definition files in the `content_types/` directory
//! 2. At startup, `ContentTypeRegistry::load_from_dir()` loads all schemas
//! 3. `SchemaMigrator::migrate()` automatically creates tables/columns
//! 4. `register_content_routes()` automatically registers CRUD API endpoints
//!
//! # Runtime Hot-Reload
//!
//! `ContentTypeRegistry` uses `RwLock` internally, supporting runtime add/remove/update of schemas.
//! Newly added content types are handled via catch-all dynamic routes without server restart.

pub mod handler;
pub mod migration;
pub mod repository;
pub mod resolver;
pub mod rule_engine;
pub mod schema;
pub mod validation;

#[cfg(feature = "export-types")]
export_types!(
    schema::ContentKind,
    schema::ContentTypeSchema,
    schema::FieldSchema,
    schema::FieldType,
    schema::RelationType,
    schema::RelationConfig,
    schema::MediaConfig,
    schema::IndexDef,
    schema::ApiAccess,
    schema::ApiEndpointConfig,
    schema::ApiConfig,
);

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use crate::constants::COL_CREATED_BY;
use crate::errors::app_error::AppError;
use arc_swap::ArcSwap;
use schema::ApiAccess as ContentTypeApiAccess;
use schema::ContentTypeSchema;

/// Content type registry
///
/// Manages all registered content type schemas, providing lookup by name/table name.
/// Uses `ArcSwap` internally for lock-free reads and low-overhead writes, supporting runtime hot-reload.
/// All queries return `Arc<ContentTypeSchema>` to avoid deep copies.
#[derive(Debug, Default)]
pub struct ContentTypeRegistry {
    inner: ArcSwap<RegistryInner>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    /// Keyed by composite key: `"singular"` (no group) or `"group/singular"` (with group)
    types: indexmap::IndexMap<String, Arc<ContentTypeSchema>>,
    /// table name → composite key
    by_table: HashMap<String, String>,
    /// `"plural"` (no group) or `"group/plural"` (with group) → composite key
    by_plural: HashMap<String, String>,
    /// Set of all non-empty group names in use
    groups: HashSet<String>,
    protected_tables: Vec<String>,
}

impl ContentTypeRegistry {
    /// Create an empty registry
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Load all TOML definitions from a directory
    ///
    /// Scans all `*.toml` files under `dir`, parses them into `ContentTypeSchema`, and registers them.
    pub fn load_from_dir(
        dir: &Path,
        rule_config: &crate::config::app::RuleEngineConfig,
        reserved_segments: &[&str],
        valid_protocols: &[&str],
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<Self, AppError> {
        let registry = Self::new();
        if !dir.exists() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| {
                AppError::Internal(anyhow::anyhow!(
                    "cannot read content_types dir {dir:?}: {e}"
                ))
            })?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| {
            std::cmp::Reverse(
                e.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            )
        });

        for entry in entries {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let schema = match ContentTypeSchema::parse_from_file(&path) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!("skipping {file_name}: parse error: {e}");
                        continue;
                    }
                };
                tracing::info!(
                    "loaded content type: {} (group={}, table={})",
                    schema.name,
                    if schema.group.is_empty() {
                        "(default)"
                    } else {
                        &schema.group
                    },
                    schema.table
                );
                if let Err(e) = registry.register(
                    schema,
                    rule_config,
                    reserved_segments,
                    valid_protocols,
                    protocol_registry,
                ) {
                    tracing::warn!("skipping {file_name}: register error: {e}");
                }
            }
        }

        let count = registry.len();
        tracing::info!("loaded {} content type(s)", count);
        Ok(registry)
    }

    /// Register a single content type (thread-safe).
    ///
    /// Checks uniqueness:
    /// - `table` must be globally unique and not conflict with protected system tables
    /// - `(group, singular)` and `(group, plural)` must be unique within the same group
    /// - `singular`/`plural` must not conflict with built-in route segments (posts, categories, tags, etc.)
    ///   when group is empty
    pub fn register(
        &self,
        schema: ContentTypeSchema,
        rule_config: &crate::config::app::RuleEngineConfig,
        reserved_segments: &[&str],
        valid_protocols: &[&str],
        protocol_registry: &crate::protocols::ProtocolRegistry,
    ) -> Result<(), AppError> {
        let mut conflicts = Vec::new();

        let protected = {
            let guard = self.inner.load();
            guard.protected_tables.clone()
        };

        if crate::plugins::permissions::PermissionChecker::is_protected_table(
            &schema.table,
            &protected,
        ) {
            conflicts.push(format!(
                "table '{}' is a protected system table",
                schema.table
            ));
        }

        let key = schema.registry_key();
        let plural_key = ContentTypeSchema::make_key(&schema.group, &schema.plural);

        // Reserved segment checks only apply to flat (no group) types
        if schema.group.is_empty() {
            if reserved_segments.contains(&schema.singular.as_str()) {
                conflicts.push(format!(
                    "singular '{}' conflicts with a built-in route",
                    schema.singular
                ));
            }
            if reserved_segments.contains(&schema.plural.as_str()) {
                conflicts.push(format!(
                    "plural '{}' conflicts with a built-in route",
                    schema.plural
                ));
            }
        }

        {
            let guard = self.inner.load();

            if let Some(existing) = guard.types.get(&key)
                && (existing.table != schema.table || existing.plural != schema.plural)
            {
                conflicts.push(format!("key '{key}' already used by '{}'", existing.name));
            }
            if let Some(conflict_key) = guard.by_plural.get(&plural_key)
                && conflict_key != &key
            {
                let name = guard
                    .types
                    .get(conflict_key)
                    .map(|ct| ct.name.as_str())
                    .unwrap_or(conflict_key);
                conflicts.push(format!(
                    "plural '{}' in group '{}' already used by '{}'",
                    schema.plural,
                    if schema.group.is_empty() {
                        "(default)"
                    } else {
                        &schema.group
                    },
                    name
                ));
            }
            if let Some(conflict_key) = guard.by_table.get(&schema.table)
                && conflict_key != &key
            {
                let name = guard
                    .types
                    .get(conflict_key)
                    .map(|ct| ct.name.as_str())
                    .unwrap_or(conflict_key);
                conflicts.push(format!(
                    "table '{}' already used by '{}'",
                    schema.table, name
                ));
            }
        }

        if !conflicts.is_empty() {
            tracing::warn!(
                "content type '{}' registration failed: {}",
                schema.name,
                conflicts.join("; ")
            );
            return Err(AppError::Internal(anyhow::anyhow!(
                "content type '{}' registration failed: {}",
                schema.name,
                conflicts.join("; ")
            )));
        }

        let unknown: Vec<&str> = schema
            .implements
            .iter()
            .map(|s| s.name())
            .filter(|name| !valid_protocols.contains(name))
            .collect();
        if !unknown.is_empty() {
            return Err(AppError::BadRequest(format!(
                "content type '{}' references unknown protocol(s): {}",
                schema.name,
                unknown.join(", ")
            )));
        }

        let mut schema = schema;
        schema.cache_protocol_columns(protocol_registry);
        schema.cache_select_columns();

        // Validate: `owner` access requires a `created_by` column
        let needs_owner = [
            schema.api.list.access,
            schema.api.get.access,
            schema.api.create.access,
            schema.api.update.access,
            schema.api.delete.access,
        ]
        .contains(&ContentTypeApiAccess::Owner);
        if needs_owner
            && !schema.is_protocol_column(COL_CREATED_BY)
            && schema.get_field(COL_CREATED_BY).is_none()
        {
            return Err(AppError::BadRequest(format!(
                "content type '{}' uses `access = \"owner\"` but has no `created_by` column; add `implements = [\"ownable\"]` or `\"timestampable\"`",
                schema.name
            )));
        }

        schema.cache_rules(rule_config);
        let table = schema.table.clone();
        let group = schema.group.clone();
        let key_clone = key.clone();
        let plural_key_clone = plural_key.clone();
        let arc = Arc::new(schema);

        self.inner.rcu(|inner| {
            let mut new_inner = RegistryInner {
                types: inner.types.clone(),
                by_table: inner.by_table.clone(),
                by_plural: inner.by_plural.clone(),
                groups: inner.groups.clone(),
                protected_tables: inner.protected_tables.clone(),
            };
            new_inner.by_table.insert(table.clone(), key_clone.clone());
            new_inner
                .by_plural
                .insert(plural_key_clone.clone(), key_clone.clone());
            if !group.is_empty() {
                new_inner.groups.insert(group.clone());
            }
            new_inner.types.insert(key_clone.clone(), arc.clone());
            new_inner
        });

        Ok(())
    }

    /// Set the system protected tables list
    pub fn set_protected_tables(&self, tables: Vec<String>) {
        self.inner.rcu(|inner| RegistryInner {
            types: inner.types.clone(),
            by_table: inner.by_table.clone(),
            by_plural: inner.by_plural.clone(),
            groups: inner.groups.clone(),
            protected_tables: tables.clone(),
        });
    }

    /// Lookup by registry key (`"singular"` for no group, `"group/singular"` for grouped)
    #[must_use]
    pub fn get(&self, key: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard.types.get(key).cloned()
    }

    /// Lookup by `(group, singular)` pair
    #[must_use]
    pub fn get_in_group(&self, group: &str, singular: &str) -> Option<Arc<ContentTypeSchema>> {
        let key = ContentTypeSchema::make_key(group, singular);
        self.get(&key)
    }

    /// Lookup by table name (globally unique)
    #[must_use]
    pub fn get_by_table(&self, table: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard
            .by_table
            .get(table)
            .and_then(|key| guard.types.get(key).cloned())
    }

    /// Lookup by plural key (`"plural"` for no group, `"group/plural"` for grouped)
    #[must_use]
    pub fn get_by_plural(&self, plural_key: &str) -> Option<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard
            .by_plural
            .get(plural_key)
            .and_then(|key| guard.types.get(key).cloned())
    }

    /// Lookup by `(group, plural)` pair
    #[must_use]
    pub fn get_by_plural_in_group(
        &self,
        group: &str,
        plural: &str,
    ) -> Option<Arc<ContentTypeSchema>> {
        let plural_key = ContentTypeSchema::make_key(group, plural);
        self.get_by_plural(&plural_key)
    }

    /// Whether any content type uses the given group name
    #[must_use]
    pub fn has_group(&self, group: &str) -> bool {
        self.inner.load().groups.contains(group)
    }

    /// All registered group names (excluding empty)
    #[must_use]
    pub fn groups(&self) -> Vec<String> {
        self.inner.load().groups.iter().cloned().collect()
    }

    /// Get all registered content types
    #[must_use]
    pub fn all(&self) -> Vec<Arc<ContentTypeSchema>> {
        let guard = self.inner.load();
        guard.types.values().cloned().collect()
    }

    /// Number of registered items
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.load().types.len()
    }

    /// Whether the registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Unregister a single content type by its registry key (thread-safe)
    pub fn unregister(&self, key: &str) -> Option<Arc<ContentTypeSchema>> {
        let removed = {
            let guard = self.inner.load();
            guard.types.get(key).cloned()
        };

        if let Some(schema) = &removed {
            let table = schema.table.clone();
            let group = schema.group.clone();
            let plural_key = ContentTypeSchema::make_key(&group, &schema.plural);
            let key_owned = key.to_string();
            self.inner.rcu(|inner| {
                let mut new_inner = RegistryInner {
                    types: inner.types.clone(),
                    by_table: inner.by_table.clone(),
                    by_plural: inner.by_plural.clone(),
                    groups: inner.groups.clone(),
                    protected_tables: inner.protected_tables.clone(),
                };
                new_inner.types.shift_remove(&key_owned);
                new_inner.by_table.remove(&table);
                new_inner.by_plural.remove(&plural_key);
                // Remove group from set if no other type uses it
                if !group.is_empty()
                    && !inner
                        .types
                        .values()
                        .any(|ct| ct.group == group && ct.registry_key() != key_owned)
                {
                    new_inner.groups.remove(&group);
                }
                new_inner
            });
        }

        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::app::RuleEngineConfig;

    fn test_protocol_registry() -> crate::protocols::ProtocolRegistry {
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(crate::protocols::timestampable::TimestampableProtocol);
        reg.register(crate::protocols::soft_deletable::SoftDeletableProtocol);
        reg.register(crate::protocols::versionable::VersionableProtocol);
        reg
    }

    fn valid_protocols() -> Vec<&'static str> {
        vec!["ownable", "timestampable", "soft_deletable", "versionable"]
    }

    fn register_ct(
        reg: &ContentTypeRegistry,
        singular: &str,
        plural: &str,
        table: &str,
    ) -> Result<(), AppError> {
        let toml = format!(
            r#"
[content_type]
name = "{singular}"
singular = "{singular}"
plural = "{plural}"
table = "{table}"

[fields.title]
type = "text"
"#
        );
        let schema = schema::ContentTypeSchema::parse_from_str(&toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = ContentTypeRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn register_and_lookup() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        assert_eq!(reg.len(), 1);
        assert!(reg.get("product").is_some());
        assert!(reg.get_by_table("products").is_some());
        assert!(reg.get_by_plural("products").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn register_conflict_table() {
        let reg = ContentTypeRegistry::new();
        reg.set_protected_tables(vec!["users".into()]);
        let toml = r#"
[content_type]
name = "User"
singular = "custom_user"
plural = "custom_users"
table = "users"

[fields.name]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_conflict_reserved_singular() {
        let reg = ContentTypeRegistry::new();
        let toml = r#"
[content_type]
name = "Auth"
singular = "auth"
plural = "auths"
table = "auth_stuff"

[fields.name]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &["auth"],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_unknown_protocol() {
        let reg = ContentTypeRegistry::new();
        let toml = r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"
implements = ["nonexistent_protocol"]

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn unregister_removes_ct() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        let removed = reg.unregister("product").unwrap();
        assert_eq!(removed.singular, "product");
        assert!(reg.is_empty());
        assert!(reg.get("product").is_none());
        assert!(reg.get_by_table("products").is_none());
        assert!(reg.get_by_plural("products").is_none());
    }

    #[test]
    fn unregister_nonexistent_returns_none() {
        let reg = ContentTypeRegistry::new();
        assert!(reg.unregister("ghost").is_none());
    }

    #[test]
    fn all_returns_all_registered() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "a", "as", "table_a").unwrap();
        register_ct(&reg, "b", "bs", "table_b").unwrap();
        let all = reg.all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn set_protected_tables_updates() {
        let reg = ContentTypeRegistry::new();
        reg.set_protected_tables(vec!["posts".into(), "users".into()]);
        let toml = r#"
[content_type]
name = "P"
singular = "p"
plural = "ps"
table = "posts"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_duplicate_singular_different_table() {
        let reg = ContentTypeRegistry::new();
        register_ct(&reg, "product", "products", "products").unwrap();
        let toml = r#"
[content_type]
name = "Product2"
singular = "product"
plural = "product2s"
table = "product2s"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        let result = reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_grouped_same_singular_different_group() {
        let reg = ContentTypeRegistry::new();
        // Flat: singular=poll, table=polls
        register_ct(&reg, "poll", "polls", "polls").unwrap();
        // Grouped: group=forum, singular=poll, table=forum_polls — should succeed
        let toml = r#"
[content_type]
name = "Forum Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();

        // Both should be findable
        assert!(reg.get("poll").is_some()); // flat
        assert!(reg.get("forum/poll").is_some()); // grouped
        assert!(reg.get_in_group("forum", "poll").is_some());
        assert!(reg.has_group("forum"));
        assert!(!reg.has_group("shop"));
    }

    #[test]
    fn register_duplicate_table_across_groups() {
        let reg = ContentTypeRegistry::new();
        // Group 1: group=forum, singular=poll, table=polls
        let toml = r#"
[content_type]
name = "Forum Poll"
singular = "poll"
plural = "polls"
table = "shared_polls"
group = "forum"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();

        // Group 2: group=shop, singular=poll, table=shared_polls — table conflict
        let toml2 = r#"
[content_type]
name = "Shop Poll"
singular = "poll"
plural = "polls"
table = "shared_polls"
group = "shop"

[fields.title]
type = "text"
"#;
        let schema2 = schema::ContentTypeSchema::parse_from_str(toml2).unwrap();
        let result = reg.register(
            schema2,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn unregister_grouped_removes_group() {
        let reg = ContentTypeRegistry::new();
        let toml = r#"
[content_type]
name = "Forum Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();

        assert!(reg.has_group("forum"));
        let removed = reg.unregister("forum/poll");
        assert!(removed.is_some());
        assert!(!reg.has_group("forum"));
    }

    #[test]
    fn register_same_name_different_groups() {
        let reg = ContentTypeRegistry::new();
        // Both use display name "Poll" but different groups/tables — should succeed
        let toml_forum = r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"

[fields.title]
type = "text"
"#;
        let toml_shop = r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "shop_polls"
group = "shop"

[fields.title]
type = "text"
"#;
        let schema_forum = schema::ContentTypeSchema::parse_from_str(toml_forum).unwrap();
        let schema_shop = schema::ContentTypeSchema::parse_from_str(toml_shop).unwrap();
        reg.register(
            schema_forum,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();
        reg.register(
            schema_shop,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();

        // Both registered, both have same name but different keys
        let forum_ct = reg.get("forum/poll").unwrap();
        let shop_ct = reg.get("shop/poll").unwrap();
        assert_eq!(forum_ct.name, "Poll");
        assert_eq!(shop_ct.name, "Poll");
        assert_eq!(forum_ct.table, "forum_polls");
        assert_eq!(shop_ct.table, "shop_polls");
        assert_ne!(forum_ct.table, shop_ct.table);
    }

    #[test]
    fn register_same_name_flat_and_grouped() {
        let reg = ContentTypeRegistry::new();
        // Flat "Poll" + grouped forum "Poll" — should coexist
        register_ct(&reg, "poll", "polls", "polls").unwrap();
        let toml = r#"
[content_type]
name = "Poll"
singular = "poll"
plural = "polls"
table = "forum_polls"
group = "forum"

[fields.title]
type = "text"
"#;
        let schema = schema::ContentTypeSchema::parse_from_str(toml).unwrap();
        reg.register(
            schema,
            &RuleEngineConfig::default(),
            &[],
            &valid_protocols(),
            &test_protocol_registry(),
        )
        .unwrap();

        assert_eq!(reg.get("poll").unwrap().name, "poll");
        assert_eq!(reg.get("forum/poll").unwrap().name, "Poll");
    }
}
