//! Content type Schema data structures and TOML parsing

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::config::app::RuleEngineConfig;
use crate::errors::app_error::AppError;

/// Protocol reference: simple string or object with configuration
///
/// ```toml
/// implements = ["sortable"]
/// implements = [{ name = "sortable", field = "priority", direction = "desc" }]
/// ```
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ProtocolRef {
    Simple(String),
    WithConfig {
        name: String,
        #[serde(flatten)]
        config: HashMap<String, String>,
    },
}

impl ProtocolRef {
    pub fn name(&self) -> &str {
        match self {
            ProtocolRef::Simple(s) => s,
            ProtocolRef::WithConfig { name, .. } => name,
        }
    }

    pub fn config(&self) -> &HashMap<String, String> {
        match self {
            ProtocolRef::Simple(_) => &EMPTY_MAP,
            ProtocolRef::WithConfig { config, .. } => config,
        }
    }
}

static EMPTY_MAP: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);

impl std::fmt::Display for ProtocolRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Content type kind
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "export-types", ts(rename_all = "lowercase"))]
pub enum ContentKind {
    /// Collection type (default): multiple records, full CRUD
    #[default]
    Collection,
    /// Single type: only one record, GET/PUT only, auto upsert
    Single,
}

/// Content type definition
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentTypeSchema {
    /// Display name (e.g., "Post")
    pub name: String,
    /// Singular identifier (e.g., "post"), used for API paths and registry key
    pub singular: String,
    /// Plural identifier (e.g., "posts"), used for API paths
    pub plural: String,
    /// Database table name
    pub table: String,
    /// Kind: collection (multiple records) or single (only one record)
    #[serde(default)]
    pub kind: ContentKind,
    /// Description
    #[serde(default)]
    pub description: String,
    /// Field list
    pub fields: Vec<FieldSchema>,
    /// Which field to auto-generate slug from
    pub slug_field: Option<String>,
    /// Whether this is a built-in content type (built-in CTs don't inject default fields; all fields are explicitly defined)
    #[serde(default)]
    pub builtin: bool,
    /// Declared Protocol list (e.g., ["versionable", "soft_deletable"])
    #[serde(default)]
    #[cfg_attr(
        feature = "export-types",
        ts(type = "Array<string | Record<string, string>>")
    )]
    pub implements: Vec<ProtocolRef>,
    /// Index definitions
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    /// API access control configuration
    #[serde(default)]
    pub api: ApiConfig,
    /// Pre-computed SELECT column name list (populated at registration, not serialized)
    #[serde(skip)]
    pub cached_column_names: Option<Vec<String>>,
    /// Protocol-provided column name list (populated at registration, not serialized)
    #[serde(skip)]
    pub cached_protocol_column_names: Option<Vec<String>>,
    /// Protocol-provided behavior capability list (populated at registration, not serialized)
    #[serde(skip)]
    pub cached_behaviors: Option<Vec<String>>,
    /// Protocol aggregated declaration (populated at registration, not serialized)
    #[serde(skip)]
    pub cached_declaration: Option<crate::protocols::ProtocolDeclaration>,
    /// Pre-parsed API Rules (populated at registration, not serialized)
    #[serde(skip)]
    pub cached_rules: Option<CachedRules>,
}

/// Pre-parsed Rules for each API endpoint
#[derive(Debug, Clone)]
pub struct CachedEndpointRules {
    pub filter: Option<super::rule_engine::Rule>,
    pub filter_auth: Option<super::rule_engine::Rule>,
}

/// Pre-parsed Rules for all API endpoints
#[derive(Debug, Clone)]
pub struct CachedRules {
    pub list: CachedEndpointRules,
    pub get: CachedEndpointRules,
    pub create: CachedEndpointRules,
    pub update: CachedEndpointRules,
    pub delete: CachedEndpointRules,
}

/// Field definition
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// Field name
    pub name: String,
    /// Field type
    pub field_type: FieldType,
    /// Whether required
    #[serde(default)]
    pub required: bool,
    /// Whether unique
    #[serde(default)]
    pub unique: bool,
    /// Default value
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub default: Option<serde_json::Value>,
    /// Private field, hidden in public API responses (only returned by admin API)
    #[serde(default)]
    pub private: bool,
    /// Immutable after creation
    #[serde(default)]
    pub immutable: bool,
    /// Admin UI display label
    pub label: Option<String>,
    /// Field description
    pub description: Option<String>,
    /// Maximum length (text/email/password)
    pub max_length: Option<usize>,
    /// Minimum value (numeric types)
    pub min: Option<f64>,
    /// Maximum value (numeric types)
    pub max: Option<f64>,
    /// Regex validation (text/email)
    pub pattern: Option<String>,
    /// Relation configuration (relation type only)
    pub relation: Option<RelationConfig>,
    /// Media configuration (media type only)
    pub media_config: Option<MediaConfig>,
    /// Enum values list (enum type only)
    pub enum_values: Option<Vec<String>>,
}

/// Field type enum
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "export-types", ts(rename_all = "snake_case"))]
pub enum FieldType {
    Text,
    RichText,
    Integer,
    BigInt,
    Decimal,
    Float,
    Boolean,
    Date,
    DateTime,
    Time,
    Email,
    Password,
    Enum,
    Uid,
    Json,
    Media,
    Relation,
}

/// Relation type
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "export-types", ts(rename_all = "snake_case"))]
pub enum RelationType {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
    OneWay,
    ManyWay,
}

/// Relation configuration
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationConfig {
    pub relation_type: RelationType,
    /// Target content type name
    pub target: String,
    /// Many-to-many junction table name
    pub through: Option<String>,
    /// Foreign key column name (defaults to "{target}_id")
    pub foreign_key: Option<String>,
}

/// Media field configuration
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Accepted MIME types
    #[serde(default)]
    pub accept: Vec<String>,
    /// Maximum file count
    #[serde(default = "default_media_max_count")]
    pub max_count: usize,
}

fn default_media_max_count() -> usize {
    1
}

/// Index definition
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// Fields included in the index
    pub fields: Vec<String>,
    /// Whether this is a unique index
    #[serde(default)]
    pub unique: bool,
}

/// API access level
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "export-types", ts(rename_all = "lowercase"))]
pub enum ApiAccess {
    /// Fully denied
    None,
    /// Public access, no authentication required
    #[default]
    Public,
    /// Requires login (any role)
    Member,
    /// Requires admin role
    Admin,
}

/// Access control configuration for a single API endpoint
///
/// TOML syntax:
/// ```toml
/// [api.list]
/// access = "public"
/// filter = 'status = "published"'
/// filter_auth = 'status = "published" || author_id = @request.auth.id'
/// cache = true
/// ```
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpointConfig {
    /// Access level: none / public / member / admin
    #[serde(default)]
    pub access: ApiAccess,
    /// Data filter expression (applies to all requests passing access check)
    pub filter: Option<String>,
    /// Additional filter for authenticated users (ORed with filter)
    pub filter_auth: Option<String>,
    /// Whether to enable server-side caching (default false)
    #[serde(default)]
    pub cache: bool,
    /// API return field whitelist (default empty = return all non-private fields)
    /// Only effective for list/get endpoints; create/update/delete ignore this setting
    #[serde(default)]
    pub fields: Option<Vec<String>>,
}

impl Default for ApiEndpointConfig {
    fn default() -> Self {
        Self {
            access: ApiAccess::Public,
            filter: None,
            filter_auth: None,
            cache: false,
            fields: None,
        }
    }
}

/// API endpoint access configuration
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// List query (GET /cms/{plural})
    #[serde(default)]
    pub list: ApiEndpointConfig,
    /// Single query (GET /cms/{plural}/{id})
    #[serde(default)]
    pub get: ApiEndpointConfig,
    /// Create (POST /cms/{plural})
    #[serde(default = "api_endpoint_member")]
    pub create: ApiEndpointConfig,
    /// Update (PUT /cms/{plural}/{id})
    #[serde(default = "api_endpoint_member")]
    pub update: ApiEndpointConfig,
    /// Delete (DELETE /cms/{plural}/{id})
    #[serde(default = "api_endpoint_admin")]
    pub delete: ApiEndpointConfig,
}

fn api_endpoint_member() -> ApiEndpointConfig {
    ApiEndpointConfig {
        access: ApiAccess::Member,
        filter: None,
        filter_auth: None,
        cache: true,
        fields: None,
    }
}

fn api_endpoint_admin() -> ApiEndpointConfig {
    ApiEndpointConfig {
        access: ApiAccess::Admin,
        filter: None,
        filter_auth: None,
        cache: true,
        fields: None,
    }
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            list: ApiEndpointConfig::default(),
            get: ApiEndpointConfig::default(),
            create: api_endpoint_member(),
            update: api_endpoint_member(),
            delete: api_endpoint_admin(),
        }
    }
}

/// Check whether the specified access level allows the current request to pass
pub fn check_api_access(
    access: ApiAccess,
    auth: &crate::middleware::auth::AuthUser,
) -> Result<(), crate::errors::app_error::AppError> {
    match access {
        ApiAccess::None => Err(crate::errors::app_error::AppError::Forbidden),
        ApiAccess::Public => Ok(()),
        ApiAccess::Member => {
            if auth.is_authenticated() {
                Ok(())
            } else {
                Err(crate::errors::app_error::AppError::Unauthorized)
            }
        }
        ApiAccess::Admin => {
            if auth.is_admin() {
                Ok(())
            } else if auth.is_authenticated() {
                Err(crate::errors::app_error::AppError::Forbidden)
            } else {
                Err(crate::errors::app_error::AppError::Unauthorized)
            }
        }
    }
}

/// Top-level structure for TOML parsing
#[derive(Debug, Deserialize)]
struct ContentTypeToml {
    content_type: ContentTypeHeader,
    fields: toml::Table,
    indexes: Option<Vec<IndexDef>>,
    api: Option<ApiConfig>,
}

#[derive(Debug, Deserialize)]
struct ContentTypeHeader {
    name: String,
    singular: String,
    plural: String,
    table: String,
    #[serde(default)]
    description: String,
    slug_field: Option<String>,
    #[serde(default)]
    kind: ContentKind,
    #[serde(default)]
    builtin: bool,
    #[serde(default)]
    implements: Vec<ProtocolRef>,
}

impl ContentTypeSchema {
    /// Parse from TOML file
    pub fn parse_from_file(path: &Path) -> Result<Self, AppError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot read {path:?}: {e}")))?;
        Self::parse_from_str(&content)
    }

    /// Parse from TOML string
    pub fn parse_from_str(content: &str) -> Result<Self, AppError> {
        let toml: ContentTypeToml = toml::from_str(content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("TOML parse error: {e}")))?;

        let mut fields = Vec::new();
        for (raw_name, value) in &toml.fields {
            let name = crate::db::driver::sanitize_identifier(raw_name).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "field name '{raw_name}' contains invalid characters (only alphanumeric and underscore allowed)"
                ))
            })?;
            let field_toml = value.as_table().ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("field '{name}' must be a table"))
            })?;

            let field_type_str =
                field_toml
                    .get("type")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::Internal(anyhow::anyhow!("field '{name}' missing 'type'"))
                    })?;

            let field_type = match field_type_str {
                "text" => FieldType::Text,
                "richtext" => FieldType::RichText,
                "integer" => FieldType::Integer,
                "bigint" => FieldType::BigInt,
                "decimal" => FieldType::Decimal,
                "float" => FieldType::Float,
                "boolean" => FieldType::Boolean,
                "date" => FieldType::Date,
                "datetime" => FieldType::DateTime,
                "time" => FieldType::Time,
                "email" => FieldType::Email,
                "password" => FieldType::Password,
                "enum" => FieldType::Enum,
                "uid" => FieldType::Uid,
                "json" => FieldType::Json,
                "media" => FieldType::Media,
                "relation" => FieldType::Relation,
                other => {
                    return Err(AppError::Internal(anyhow::anyhow!(
                        "unknown field type '{other}' for field '{name}'"
                    )));
                }
            };

            let relation = if field_type == FieldType::Relation {
                Some(parse_relation_config(field_toml)?)
            } else {
                None
            };

            let media_config = if field_type == FieldType::Media {
                Some(parse_media_config(field_toml))
            } else {
                None
            };

            let default = field_toml.get("default").map(toml_value_to_json);

            fields.push(FieldSchema {
                name: name.to_string(),
                field_type,
                required: field_toml
                    .get("required")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                unique: field_toml
                    .get("unique")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                default,
                private: field_toml
                    .get("private")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                immutable: field_toml
                    .get("immutable")
                    .and_then(toml::Value::as_bool)
                    .unwrap_or(false),
                label: field_toml
                    .get("label")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                description: field_toml
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                max_length: field_toml
                    .get("max_length")
                    .and_then(toml::Value::as_integer)
                    .map(|v| v as usize),
                min: field_toml
                    .get("min")
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))),
                max: field_toml
                    .get("max")
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))),
                pattern: field_toml
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                relation,
                media_config,
                enum_values: field_toml
                    .get("enum_values")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    }),
            });
        }

        Ok(ContentTypeSchema {
            name: toml.content_type.name.trim().to_string(),
            singular: Self::validate_identifier(&toml.content_type.singular, "singular")?,
            plural: Self::validate_identifier(&toml.content_type.plural, "plural")?,
            table: Self::validate_table_name(&toml.content_type.table)?,
            description: toml.content_type.description,
            kind: toml.content_type.kind,
            fields,
            slug_field: toml.content_type.slug_field,
            builtin: toml.content_type.builtin,
            implements: toml.content_type.implements,
            indexes: Self::validate_indexes(toml.indexes.unwrap_or_default())?,
            api: toml.api.unwrap_or_default(),
            cached_column_names: None,
            cached_protocol_column_names: None,
            cached_behaviors: None,
            cached_declaration: None,
            cached_rules: None,
        })
    }

    /// Cache protocol-provided column names, behavior capabilities, and declaration (must be called before cache_select_columns)
    pub fn cache_protocol_columns(&mut self, registry: &crate::protocols::ProtocolRegistry) {
        let names: Vec<String> = self
            .implements
            .iter()
            .map(|p| p.name().to_string())
            .collect();
        let mut columns = registry.columns_for(&names);
        let field_names: Vec<&str> = self.fields.iter().map(|f| f.name.as_str()).collect();
        columns.retain(|c| !field_names.contains(&c.name.as_str()));
        self.cached_protocol_column_names = Some(columns.iter().map(|c| c.name.clone()).collect());
        let behaviors: Vec<String> = self
            .implements
            .iter()
            .filter_map(|p| registry.get(p.name()))
            .flat_map(|proto| proto.behaviors())
            .map(|b| b.to_string())
            .collect();
        self.cached_behaviors = Some(behaviors);
        let mut decl = registry.declaration_for(&names);
        let protocol_cols: Vec<String> = self
            .cached_protocol_column_names
            .clone()
            .unwrap_or_default();
        let all_columns: Vec<&str> = self
            .fields
            .iter()
            .map(|f| f.name.as_str())
            .chain(protocol_cols.iter().map(|s| s.as_str()))
            .chain(["id"])
            .collect();
        registry.apply_config_for(&self.implements, &mut decl, &all_columns);
        self.cached_declaration = Some(decl);
    }

    /// Pre-compute and cache SELECT column name list (must be called after cache_protocol_columns)
    pub fn cache_select_columns(&mut self) {
        self.cached_column_names = Some(crate::content_type::repository::build_column_names(
            self, None, true,
        ));
    }

    /// Get protocol-provided column names (must call `cache_protocol_columns` first)
    pub fn protocol_column_names(&self) -> Vec<&str> {
        self.cached_protocol_column_names
            .as_ref()
            .map(|names| names.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Check if a column name is provided by a protocol
    pub fn is_protocol_column(&self, name: &str) -> bool {
        self.protocol_column_names().contains(&name)
    }

    /// Get the aggregated protocol declaration (must call `cache_protocol_columns` first)
    pub fn declaration(&self) -> crate::protocols::ProtocolDeclaration {
        self.cached_declaration.clone().unwrap_or_default()
    }

    /// Get the protocol-declared query filter conditions
    pub fn query_filters(&self) -> Vec<(String, String)> {
        self.declaration().query_filters
    }

    /// Whether soft delete is enabled
    pub fn is_soft_delete(&self) -> bool {
        self.declaration().is_soft_delete()
    }

    /// Whether version history routes are provided
    pub fn has_revision_routes(&self) -> bool {
        self.declaration().revision_routes
    }

    fn validate_identifier(name: &str, label: &str) -> Result<String, AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "{label} must not be empty"
            )));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AppError::Internal(anyhow::anyhow!(
                "{label} '{name}' contains invalid characters (only alphanumeric and underscore allowed)"
            )));
        }
        Ok(name.to_string())
    }

    /// Validate that the table name only contains safe characters to prevent SQL injection.
    fn validate_table_name(name: &str) -> Result<String, AppError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Internal(anyhow::anyhow!(
                "table name must not be empty"
            )));
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(AppError::Internal(anyhow::anyhow!(
                "table name '{name}' contains invalid characters (only alphanumeric and underscore allowed)"
            )));
        }
        Ok(name.to_string())
    }

    fn validate_indexes(mut indexes: Vec<IndexDef>) -> Result<Vec<IndexDef>, AppError> {
        for idx in &mut indexes {
            for field in &mut idx.fields {
                *field = crate::db::driver::sanitize_identifier(field)
                    .ok_or_else(|| {
                        AppError::Internal(anyhow::anyhow!(
                            "index field '{field}' contains invalid characters"
                        ))
                    })?
                    .to_string();
            }
        }
        Ok(indexes)
    }

    /// Pre-parse API Rule expressions, called once during schema registration
    pub fn cache_rules(&mut self, config: &RuleEngineConfig) {
        self.cached_rules = Some(CachedRules {
            list: self.parse_endpoint_rules(&self.api.list, config),
            get: self.parse_endpoint_rules(&self.api.get, config),
            create: self.parse_endpoint_rules(&self.api.create, config),
            update: self.parse_endpoint_rules(&self.api.update, config),
            delete: self.parse_endpoint_rules(&self.api.delete, config),
        });
    }

    fn parse_endpoint_rules(
        &self,
        config: &ApiEndpointConfig,
        rule_config: &RuleEngineConfig,
    ) -> CachedEndpointRules {
        CachedEndpointRules {
            filter: config
                .filter
                .as_deref()
                .and_then(|s| super::rule_engine::Rule::parse(s, rule_config).ok()),
            filter_auth: config
                .filter_auth
                .as_deref()
                .and_then(|s| super::rule_engine::Rule::parse(s, rule_config).ok()),
        }
    }

    /// Get column name list
    ///
    /// When `include_private=false`, filters out `private` fields (for public API).
    /// When `include_private=true`, returns all fields (for admin API).
    pub fn column_names(&self, requested: Option<&[String]>, include_private: bool) -> Vec<String> {
        if include_private
            && requested.is_none()
            && let Some(ref cached) = self.cached_column_names
        {
            return cached.clone();
        }
        crate::content_type::repository::build_column_names(self, requested, include_private)
    }

    /// Get non-private field list
    #[must_use]
    pub fn public_fields(&self) -> Vec<&FieldSchema> {
        self.fields.iter().filter(|f| !f.private).collect()
    }

    /// Get relation field list
    #[must_use]
    pub fn relation_fields(&self) -> Vec<&FieldSchema> {
        self.fields
            .iter()
            .filter(|f| f.field_type == FieldType::Relation)
            .collect()
    }

    /// Get all column names that store Snowflake IDs.
    ///
    /// Includes: primary key (`id`), relation FK columns, and protocol ID columns
    /// (`created_by`, `updated_by`, `deleted_by`, `parent_id`).
    #[must_use]
    pub fn id_column_set(&self) -> std::collections::HashSet<&str> {
        let mut set = std::collections::HashSet::new();
        set.insert(crate::constants::COL_ID);
        for f in self.relation_fields() {
            if let Some(RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay) =
                f.relation.as_ref().map(|r| &r.relation_type)
            {
                let fk = f
                    .relation
                    .as_ref()
                    .and_then(|r| r.foreign_key.as_deref())
                    .unwrap_or(&f.name);
                set.insert(fk);
            }
        }
        for name in self.protocol_column_names() {
            match name {
                crate::constants::COL_CREATED_BY
                | crate::constants::COL_UPDATED_BY
                | crate::constants::COL_DELETED_BY
                | crate::constants::COL_PARENT_ID => {
                    set.insert(name);
                }
                _ => {}
            }
        }
        set
    }

    /// Find field definition by field name
    #[must_use]
    pub fn get_field(&self, name: &str) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Whether this is a Single Type
    #[must_use]
    pub fn is_single(&self) -> bool {
        self.kind == ContentKind::Single
    }

    /// Whether this is a Collection Type
    #[must_use]
    pub fn is_collection(&self) -> bool {
        self.kind == ContentKind::Collection
    }

    /// Whether it implements the specified Protocol
    #[must_use]
    pub fn implements_protocol(&self, name: &str) -> bool {
        self.implements.iter().any(|p| p.name() == name)
    }

    /// Get the UID field (used for slug)
    #[must_use]
    pub fn uid_field(&self) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.field_type == FieldType::Uid)
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, AppError> {
        let mut header = toml::Table::new();
        header.insert("name".into(), toml::Value::String(self.name.clone()));
        header.insert(
            "singular".into(),
            toml::Value::String(self.singular.clone()),
        );
        header.insert("plural".into(), toml::Value::String(self.plural.clone()));
        header.insert("table".into(), toml::Value::String(self.table.clone()));
        if !self.description.is_empty() {
            header.insert(
                "description".into(),
                toml::Value::String(self.description.clone()),
            );
        }
        if let Some(ref sf) = self.slug_field {
            header.insert("slug_field".into(), toml::Value::String(sf.clone()));
        }
        if self.builtin {
            header.insert("builtin".into(), toml::Value::Boolean(true));
        }
        if !self.implements.is_empty() {
            header.insert(
                "implements".into(),
                toml::Value::Array(
                    self.implements
                        .iter()
                        .map(|p| match p {
                            ProtocolRef::Simple(s) => toml::Value::String(s.clone()),
                            ProtocolRef::WithConfig { name, config } => {
                                let mut table = toml::Table::new();
                                table.insert("name".into(), toml::Value::String(name.clone()));
                                for (k, v) in config {
                                    table.insert(k.clone(), toml::Value::String(v.clone()));
                                }
                                toml::Value::Table(table)
                            }
                        })
                        .collect(),
                ),
            );
        }
        if self.kind == ContentKind::Single {
            header.insert("kind".into(), toml::Value::String("single".into()));
        }

        let mut fields_table = toml::Table::new();
        for field in &self.fields {
            fields_table.insert(field.name.clone(), field_to_toml(field));
        }

        let mut root = toml::Table::new();
        root.insert("content_type".into(), toml::Value::Table(header));
        root.insert("fields".into(), toml::Value::Table(fields_table));

        if !self.indexes.is_empty() {
            let indexes: Vec<toml::Value> = self
                .indexes
                .iter()
                .map(|idx| {
                    let mut t = toml::Table::new();
                    t.insert(
                        "fields".into(),
                        toml::Value::Array(
                            idx.fields
                                .iter()
                                .map(|f| toml::Value::String(f.clone()))
                                .collect(),
                        ),
                    );
                    if idx.unique {
                        t.insert("unique".into(), toml::Value::Boolean(true));
                    }
                    toml::Value::Table(t)
                })
                .collect();
            root.insert("indexes".into(), toml::Value::Array(indexes));
        }

        toml::to_string_pretty(&root)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("TOML serialize error: {e}")))
    }

    /// Save TOML file to the specified directory
    pub fn save_to_dir(&self, dir: &Path) -> Result<(), AppError> {
        std::fs::create_dir_all(dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("cannot create content_types dir: {e}"))
        })?;
        let path = dir.join(format!("{}.toml", self.singular));
        let content = self.to_toml()?;
        std::fs::write(&path, content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot write {:?}: {e}", path)))?;
        Ok(())
    }
}

fn field_to_toml(field: &FieldSchema) -> toml::Value {
    let mut t = toml::Table::new();
    t.insert(
        "type".into(),
        toml::Value::String(format!("{:?}", field.field_type).to_lowercase()),
    );

    if field.required {
        t.insert("required".into(), toml::Value::Boolean(true));
    }
    if field.unique {
        t.insert("unique".into(), toml::Value::Boolean(true));
    }
    if let Some(ref label) = field.label {
        t.insert("label".into(), toml::Value::String(label.clone()));
    }
    if let Some(ref desc) = field.description {
        t.insert("description".into(), toml::Value::String(desc.clone()));
    }
    if let Some(max_len) = field.max_length {
        t.insert("max_length".into(), toml::Value::Integer(max_len as i64));
    }
    if let Some(min) = field.min {
        t.insert("min".into(), toml::Value::Float(min));
    }
    if let Some(max) = field.max {
        t.insert("max".into(), toml::Value::Float(max));
    }
    if let Some(ref pattern) = field.pattern {
        t.insert("pattern".into(), toml::Value::String(pattern.clone()));
    }
    if let Some(ref default) = field.default {
        t.insert("default".into(), json_to_toml(default));
    }
    if field.private {
        t.insert("private".into(), toml::Value::Boolean(true));
    }
    if field.immutable {
        t.insert("immutable".into(), toml::Value::Boolean(true));
    }
    if let Some(ref vals) = field.enum_values {
        t.insert(
            "enum_values".into(),
            toml::Value::Array(
                vals.iter()
                    .map(|v| toml::Value::String(v.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(ref rel) = field.relation {
        t.insert(
            "relation_type".into(),
            toml::Value::String(format!("{:?}", rel.relation_type).to_lowercase()),
        );
        t.insert("target".into(), toml::Value::String(rel.target.clone()));
        if let Some(ref through) = rel.through {
            t.insert("through".into(), toml::Value::String(through.clone()));
        }
        if let Some(ref fk) = rel.foreign_key {
            t.insert("foreign_key".into(), toml::Value::String(fk.clone()));
        }
    }
    if let Some(ref mc) = field.media_config {
        let mut mt = toml::Table::new();
        if !mc.accept.is_empty() {
            mt.insert(
                "accept".into(),
                toml::Value::Array(
                    mc.accept
                        .iter()
                        .map(|a| toml::Value::String(a.clone()))
                        .collect(),
                ),
            );
        }
        if mc.max_count != 1 {
            mt.insert(
                "max_count".into(),
                toml::Value::Integer(mc.max_count as i64),
            );
        }
        for (k, v) in mt {
            t.insert(k, v);
        }
    }

    toml::Value::Table(t)
}

fn json_to_toml(v: &serde_json::Value) -> toml::Value {
    match v {
        serde_json::Value::String(s) => toml::Value::String(s.clone()),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                toml::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                toml::Value::Float(f)
            } else {
                toml::Value::String(n.to_string())
            }
        }
        serde_json::Value::Bool(b) => toml::Value::Boolean(*b),
        serde_json::Value::Null => toml::Value::String("null".into()),
        other => toml::Value::String(other.to_string()),
    }
}

fn parse_relation_config(table: &toml::Table) -> Result<RelationConfig, AppError> {
    let relation_type_str = table
        .get("relation_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("relation field missing 'relation_type'"))
        })?;

    let relation_type = match relation_type_str {
        "one_to_one" => RelationType::OneToOne,
        "one_to_many" => RelationType::OneToMany,
        "many_to_one" => RelationType::ManyToOne,
        "many_to_many" => RelationType::ManyToMany,
        "one_way" => RelationType::OneWay,
        "many_way" => RelationType::ManyWay,
        other => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "unknown relation_type '{other}'"
            )));
        }
    };

    let raw_target = table.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let target = crate::db::driver::sanitize_identifier(raw_target).ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "relation target '{raw_target}' contains invalid characters"
        ))
    })?;

    let foreign_key = table
        .get("foreign_key")
        .and_then(|v| v.as_str())
        .map(|raw_fk| {
            crate::db::driver::sanitize_identifier(raw_fk).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "relation foreign_key '{raw_fk}' contains invalid characters"
                ))
            })
        })
        .transpose()?
        .map(|s| s.to_string());

    let through = table
        .get("through")
        .and_then(|v| v.as_str())
        .map(|raw_through| {
            crate::db::driver::sanitize_identifier(raw_through).ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!(
                    "relation through '{raw_through}' contains invalid characters"
                ))
            })
        })
        .transpose()?
        .map(|s| s.to_string());

    Ok(RelationConfig {
        relation_type,
        target: target.to_string(),
        through,
        foreign_key,
    })
}

fn parse_media_config(table: &toml::Table) -> MediaConfig {
    MediaConfig {
        accept: table
            .get("accept")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        max_count: table
            .get("max_count")
            .and_then(toml::Value::as_integer)
            .unwrap_or(1) as usize,
    }
}

fn toml_value_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::String(s) => serde_json::Value::String(s.clone()),
        toml::Value::Integer(i) => serde_json::Value::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        toml::Value::Boolean(b) => serde_json::Value::Bool(*b),
        toml::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(toml_value_to_json).collect())
        }
        toml::Value::Table(tbl) => {
            let map: serde_json::Map<String, serde_json::Value> = tbl
                .iter()
                .map(|(k, v)| (k.clone(), toml_value_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
        toml::Value::Datetime(dt) => serde_json::Value::String(dt.to_string()),
    }
}

/// Create form fields (for API)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContentTypeRequest {
    pub name: String,
    pub singular: String,
    pub plural: String,
    pub table: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: ContentKind,
    pub slug_field: Option<String>,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub implements: Vec<ProtocolRef>,
    #[serde(default)]
    pub fields: Vec<FieldSchema>,
}

/// Update form fields (for API)
///
/// Unlike `CreateContentTypeRequest`, all fields are optional;
/// only provided fields are updated. `fields` replaces the entire field list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateContentTypeRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub slug_field: Option<Option<String>>,
    #[serde(default)]
    pub implements: Option<Vec<ProtocolRef>>,
    #[serde(default)]
    pub fields: Option<Vec<FieldSchema>>,
    #[serde(default)]
    pub indexes: Option<Vec<IndexDef>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_type::ContentTypeRegistry;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn parse_minimal_content_type() {
        let toml = r#"
[content_type]
name = "Page"
singular = "page"
plural = "pages"
table = "pages"

[fields.title]
type = "text"
required = true
max_length = 200
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.name, "Page");
        assert_eq!(ct.singular, "page");
        assert_eq!(ct.plural, "pages");
        assert_eq!(ct.table, "pages");
        assert_eq!(ct.fields.len(), 1);
        assert_eq!(ct.fields[0].name, "title");
        assert_eq!(ct.fields[0].field_type, FieldType::Text);
        assert!(ct.fields[0].required);
        assert_eq!(ct.fields[0].max_length, Some(200));
    }

    #[test]
    fn parse_full_content_type() {
        let toml = r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
description = "Blog posts"
slug_field = "title"

[fields.title]
type = "text"
required = true
max_length = 200
label = "Title"

[fields.slug]
type = "uid"
target_field = "title"
unique = true

[fields.content]
type = "richtext"
required = true

[fields.status]
type = "enum"
enum_values = ["draft", "published", "archived"]
default = "draft"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "user"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tag"
through = "posts_tags"

[fields.view_count]
type = "integer"
default = 0
private = true

[fields.is_pinned]
type = "boolean"
default = false

[[indexes]]
fields = ["slug"]
unique = true
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.name, "Post");
        assert_eq!(ct.slug_field, Some("title".into()));
        assert_eq!(ct.fields.len(), 8);
        assert_eq!(ct.indexes.len(), 1);

        let slug = ct.uid_field().unwrap();
        assert_eq!(slug.name, "slug");
        assert!(slug.unique);

        let rel_fields = ct.relation_fields();
        assert_eq!(rel_fields.len(), 2);

        let public = ct.public_fields();
        assert_eq!(public.len(), 7); // view_count is private

        let status = ct.get_field("status").unwrap();
        assert_eq!(
            status.enum_values,
            Some(vec!["draft".into(), "published".into(), "archived".into()])
        );
    }

    #[test]
    fn parse_all_field_types() {
        let toml = r#"
[content_type]
name = "Test"
singular = "test"
plural = "tests"
table = "tests"

[fields.f_text]
type = "text"

[fields.f_richtext]
type = "richtext"

[fields.f_integer]
type = "integer"

[fields.f_bigint]
type = "bigint"

[fields.f_decimal]
type = "decimal"

[fields.f_float]
type = "float"

[fields.f_boolean]
type = "boolean"

[fields.f_date]
type = "date"

[fields.f_datetime]
type = "datetime"

[fields.f_time]
type = "time"

[fields.f_email]
type = "email"

[fields.f_password]
type = "password"

[fields.f_enum]
type = "enum"
enum_values = ["a", "b"]

[fields.f_uid]
type = "uid"

[fields.f_json]
type = "json"

[fields.f_media]
type = "media"
accept = ["image/*"]
max_count = 5

[fields.f_relation]
type = "relation"
relation_type = "many_to_one"
target = "user"
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.fields.len(), 17);
        assert_eq!(ct.get_field("f_text").unwrap().field_type, FieldType::Text);
        assert_eq!(
            ct.get_field("f_richtext").unwrap().field_type,
            FieldType::RichText
        );
        assert_eq!(
            ct.get_field("f_relation").unwrap().field_type,
            FieldType::Relation
        );
        assert_eq!(
            ct.get_field("f_media").unwrap().field_type,
            FieldType::Media
        );
        assert_eq!(ct.get_field("f_json").unwrap().field_type, FieldType::Json);
        assert_eq!(ct.get_field("f_enum").unwrap().field_type, FieldType::Enum);
    }

    #[test]
    fn parse_from_file() {
        let mut f = NamedTempFile::new().unwrap();
        write!(
            f,
            r#"
[content_type]
name = "Demo"
singular = "demo"
plural = "demos"
table = "demos"

[fields.name]
type = "text"
required = true
"#
        )
        .unwrap();

        let ct = ContentTypeSchema::parse_from_file(f.path()).unwrap();
        assert_eq!(ct.name, "Demo");
    }

    #[test]
    fn registry_load_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let path1 = dir.path().join("article.toml");
        let path2 = dir.path().join("page.toml");
        std::fs::write(
            &path1,
            r#"
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        std::fs::write(
            &path2,
            r#"
[content_type]
name = "Document"
singular = "document"
plural = "documents"
table = "documents"

[fields.title]
type = "text"
"#,
        )
        .unwrap();

        let reserved = crate::config::app::BuiltinsConfig::default().reserved_route_segments();
        let mut test_reg = crate::protocols::ProtocolRegistry::new();
        test_reg.register(crate::protocols::ownable::OwnableProtocol);
        test_reg.register(crate::protocols::timestampable::TimestampableProtocol);
        test_reg.register(crate::protocols::soft_deletable::SoftDeletableProtocol);
        test_reg.register(crate::protocols::versionable::VersionableProtocol);
        let reg = ContentTypeRegistry::load_from_dir(
            dir.path(),
            &crate::config::app::RuleEngineConfig::default(),
            &reserved,
            &["ownable", "timestampable", "soft_deletable", "versionable"],
            &test_reg,
        )
        .unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.get("article").is_some());
        assert!(reg.get("document").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert!(reg.get_by_table("articles").is_some());
    }

    #[test]
    fn parse_error_missing_type() {
        let toml = r#"
[content_type]
name = "Bad"
singular = "bad"
plural = "bads"
table = "bads"

[fields.title]
required = true
"#;
        let result = ContentTypeSchema::parse_from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_error_unknown_type() {
        let toml = r#"
[content_type]
name = "Bad"
singular = "bad"
plural = "bads"
table = "bads"

[fields.title]
type = "unknown_type"
"#;
        let result = ContentTypeSchema::parse_from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_single_type() {
        let toml = r#"
[content_type]
name = "SiteSetting"
singular = "site_setting"
plural = "site_settings"
table = "site_settings"
kind = "single"

[fields.site_title]
type = "text"
default = "My Site"

[fields.site_description]
type = "text"
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.name, "SiteSetting");
        assert!(ct.is_single());
        assert!(!ct.is_collection());
        assert_eq!(ct.kind, ContentKind::Single);

        let serialized = ct.to_toml().unwrap();
        assert!(serialized.contains("kind = \"single\""));

        let reparsed = ContentTypeSchema::parse_from_str(&serialized).unwrap();
        assert!(reparsed.is_single());
    }

    #[test]
    fn parse_collection_type_default() {
        let toml = r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert!(ct.is_collection());
        assert!(!ct.is_single());
        assert_eq!(ct.kind, ContentKind::Collection);
    }

    #[test]
    fn check_api_access_none_rejects() {
        let auth = crate::middleware::auth::AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            None,
        );
        assert!(check_api_access(ApiAccess::None, &auth).is_err());
    }

    #[test]
    fn check_api_access_public_allows_all() {
        let anon = crate::middleware::auth::AuthUser::from_parts(
            None,
            crate::models::user::UserRole::Reader,
            None,
        );
        assert!(check_api_access(ApiAccess::Public, &anon).is_ok());
    }

    #[test]
    fn check_api_access_member_requires_auth() {
        let anon = crate::middleware::auth::AuthUser::from_parts(
            None,
            crate::models::user::UserRole::Reader,
            None,
        );
        assert!(check_api_access(ApiAccess::Member, &anon).is_err());
        let user = crate::middleware::auth::AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Reader,
            None,
        );
        assert!(check_api_access(ApiAccess::Member, &user).is_ok());
    }

    #[test]
    fn check_api_access_admin_requires_admin_role() {
        let anon = crate::middleware::auth::AuthUser::from_parts(
            None,
            crate::models::user::UserRole::Reader,
            None,
        );
        assert!(check_api_access(ApiAccess::Admin, &anon).is_err());
        let user = crate::middleware::auth::AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Reader,
            None,
        );
        let err = check_api_access(ApiAccess::Admin, &user).unwrap_err();
        assert!(matches!(err, AppError::Forbidden));
        let admin = crate::middleware::auth::AuthUser::from_parts(
            Some(1),
            crate::models::user::UserRole::Admin,
            None,
        );
        assert!(check_api_access(ApiAccess::Admin, &admin).is_ok());
    }

    #[test]
    fn validate_identifier_empty() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = ""
plural = "xs"
table = "xs"

[fields.a]
type = "text"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_identifier_invalid_chars() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "bad name!"
plural = "xs"
table = "xs"

[fields.a]
type = "text"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_table_name_empty() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = ""

[fields.a]
type = "text"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_table_name_invalid_chars() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "drop table users"

[fields.a]
type = "text"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn validate_indexes_invalid_field() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"

[[indexes]]
fields = ["a; DROP TABLE"]
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn protocol_ref_simple() {
        let pr = ProtocolRef::Simple("sortable".into());
        assert_eq!(pr.name(), "sortable");
        assert!(pr.config().is_empty());
        assert_eq!(format!("{pr}"), "sortable");
    }

    #[test]
    fn protocol_ref_with_config() {
        let pr = ProtocolRef::WithConfig {
            name: "sortable".into(),
            config: {
                let mut m = HashMap::new();
                m.insert("field".into(), "priority".into());
                m
            },
        };
        assert_eq!(pr.name(), "sortable");
        assert_eq!(pr.config().get("field").unwrap(), "priority");
    }

    #[test]
    fn content_kind_default_is_collection() {
        assert_eq!(ContentKind::default(), ContentKind::Collection);
    }

    #[test]
    fn api_config_defaults() {
        let api = ApiConfig::default();
        assert_eq!(api.list.access, ApiAccess::Public);
        assert_eq!(api.get.access, ApiAccess::Public);
        assert_eq!(api.create.access, ApiAccess::Member);
        assert_eq!(api.update.access, ApiAccess::Member);
        assert_eq!(api.delete.access, ApiAccess::Admin);
    }

    #[test]
    fn api_endpoint_config_default() {
        let ep = ApiEndpointConfig::default();
        assert_eq!(ep.access, ApiAccess::Public);
        assert!(ep.filter.is_none());
        assert!(ep.filter_auth.is_none());
        assert!(!ep.cache);
        assert!(ep.fields.is_none());
    }

    #[test]
    fn to_toml_round_trip_with_implements_and_indexes() {
        let toml = r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
description = "Blog posts"
slug_field = "title"
builtin = true

[fields.title]
type = "text"
required = true

[[indexes]]
fields = ["title"]
unique = true
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        let serialized = ct.to_toml().unwrap();
        assert!(serialized.contains("description"));
        assert!(serialized.contains("slug_field"));
        assert!(serialized.contains("builtin"));
        assert!(serialized.contains("[[indexes]]"));
        let reparsed = ContentTypeSchema::parse_from_str(&serialized).unwrap();
        assert_eq!(reparsed.name, ct.name);
        assert_eq!(reparsed.fields.len(), ct.fields.len());
        assert_eq!(reparsed.indexes.len(), ct.indexes.len());
    }

    #[test]
    fn save_to_dir_creates_file() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Demo"
singular = "demo"
plural = "demos"
table = "demos"

[fields.name]
type = "text"
"#,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        ct.save_to_dir(dir.path()).unwrap();
        let path = dir.path().join("demo.toml");
        assert!(path.exists());
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("Demo"));
    }

    #[test]
    fn parse_field_not_table() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = 42
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_relation_missing_relation_type() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.author]
type = "relation"
target = "user"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn parse_relation_unknown_relation_type() {
        let result = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.author]
type = "relation"
relation_type = "has_many"
target = "user"
"#,
        );
        assert!(result.is_err());
    }

    #[test]
    fn get_field_nonexistent() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"
"#,
        )
        .unwrap();
        assert!(ct.get_field("b").is_none());
    }

    #[test]
    fn uid_field_none_when_no_uid() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"
"#,
        )
        .unwrap();
        assert!(ct.uid_field().is_none());
    }

    #[test]
    fn implements_protocol_check() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"
implements = ["timestampable"]

[fields.a]
type = "text"
"#,
        )
        .unwrap();
        assert!(ct.implements_protocol("timestampable"));
        assert!(!ct.implements_protocol("soft_deletable"));
    }

    #[test]
    fn column_names_public_excludes_private() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"

[fields.secret]
type = "text"
private = true
"#,
        )
        .unwrap();
        let public = ct.column_names(None, false);
        assert!(public.contains(&"a".to_string()));
        assert!(!public.contains(&"secret".to_string()));
        let all = ct.column_names(None, true);
        assert!(all.contains(&"a".to_string()));
        assert!(all.contains(&"secret".to_string()));
    }

    #[test]
    fn column_names_requested_filter() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"

[fields.b]
type = "text"
"#,
        )
        .unwrap();
        let filtered = ct.column_names(Some(&["a".to_string()]), false);
        assert!(filtered.contains(&"a".to_string()));
        assert!(!filtered.contains(&"b".to_string()));
    }

    #[test]
    fn parse_from_file_not_found() {
        let result = ContentTypeSchema::parse_from_file(Path::new("/nonexistent/path.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_with_api_config() {
        let toml = r#"
[content_type]
name = "X"
singular = "x"
plural = "xs"
table = "xs"

[fields.a]
type = "text"

[api.list]
access = "admin"
cache = true

[api.get]
access = "member"
filter = 'status = "published"'
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.api.list.access, ApiAccess::Admin);
        assert!(ct.api.list.cache);
        assert_eq!(ct.api.get.access, ApiAccess::Member);
        assert_eq!(ct.api.get.filter.as_deref(), Some("status = \"published\""));
    }
}
