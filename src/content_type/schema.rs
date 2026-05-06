//! 内容类型 Schema 数据结构与 TOML 解析

use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
#[cfg(feature = "export-types")]
use ts_rs::TS;

use crate::config::app::RuleEngineConfig;
use crate::errors::app_error::AppError;

/// 协议引用：简单字符串或带配置的对象
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

/// 内容类型种类
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "export-types", ts(rename_all = "lowercase"))]
pub enum ContentKind {
    /// 集合类型（默认）：多条记录，完整 CRUD
    #[default]
    Collection,
    /// 单条类型：只有一条记录，仅 GET/PUT，自动 upsert
    Single,
}

/// 内容类型定义
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentTypeSchema {
    /// 显示名称（如 "Post"）
    pub name: String,
    /// 单数标识（如 "post"），用于 API 路径和注册表 key
    pub singular: String,
    /// 复数标识（如 "posts"），用于 API 路径
    pub plural: String,
    /// 数据库表名
    pub table: String,
    /// 类型：collection（多条记录）或 single（仅一条记录）
    #[serde(default)]
    pub kind: ContentKind,
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 字段列表
    pub fields: Vec<FieldSchema>,
    /// 自动从哪个字段生成 slug
    pub slug_field: Option<String>,
    /// 是否为内置 content type（内置 CT 不注入默认字段，字段全部显式定义）
    #[serde(default)]
    pub builtin: bool,
    /// 声明实现的 Protocol 列表（如 ["versionable", "cacheable"]）
    #[serde(default)]
    pub implements: Vec<ProtocolRef>,
    /// 索引定义
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    /// API 访问控制配置
    #[serde(default)]
    pub api: ApiConfig,
    /// 预计算 SELECT 列名列表（注册时填充，不序列化）
    #[serde(skip)]
    pub cached_column_names: Option<Vec<String>>,
    /// 协议提供的列名列表（注册时填充，不序列化）
    #[serde(skip)]
    pub cached_protocol_column_names: Option<Vec<String>>,
    /// 协议提供的行为能力列表（注册时填充，不序列化）
    #[serde(skip)]
    pub cached_behaviors: Option<Vec<String>>,
    /// 协议聚合声明（注册时填充，不序列化）
    #[serde(skip)]
    pub cached_declaration: Option<crate::protocols::ProtocolDeclaration>,
    /// 预解析的 API Rule（注册时填充，不序列化）
    #[serde(skip)]
    pub cached_rules: Option<CachedRules>,
}

/// 每个 API 端点的预解析 Rule
#[derive(Debug, Clone)]
pub struct CachedEndpointRules {
    pub filter: Option<super::rule_engine::Rule>,
    pub filter_auth: Option<super::rule_engine::Rule>,
}

/// 所有 API 端点的预解析 Rule
#[derive(Debug, Clone)]
pub struct CachedRules {
    pub list: CachedEndpointRules,
    pub get: CachedEndpointRules,
    pub create: CachedEndpointRules,
    pub update: CachedEndpointRules,
    pub delete: CachedEndpointRules,
}

/// 字段定义
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// 字段名
    pub name: String,
    /// 字段类型
    pub field_type: FieldType,
    /// 是否必填
    #[serde(default)]
    pub required: bool,
    /// 是否唯一
    #[serde(default)]
    pub unique: bool,
    /// 默认值
    #[serde(default)]
    #[cfg_attr(feature = "export-types", ts(type = "unknown"))]
    pub default: Option<serde_json::Value>,
    /// 私有字段，公开 API 响应中隐藏（仅 admin API 返回）
    #[serde(default)]
    pub private: bool,
    /// 创建后不可修改
    #[serde(default)]
    pub immutable: bool,
    /// Admin UI 显示标签
    pub label: Option<String>,
    /// 字段说明
    pub description: Option<String>,
    /// 最大长度（text/email/password）
    pub max_length: Option<usize>,
    /// 最小值（数值类型）
    pub min: Option<f64>,
    /// 最大值（数值类型）
    pub max: Option<f64>,
    /// 正则校验（text/email）
    pub pattern: Option<String>,
    /// 关系配置（仅 relation 类型）
    pub relation: Option<RelationConfig>,
    /// 媒体配置（仅 media 类型）
    pub media_config: Option<MediaConfig>,
    /// 枚举值列表（仅 enum 类型）
    pub enum_values: Option<Vec<String>>,
}

/// 字段类型枚举
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

/// 关系类型
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

/// 关系配置
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationConfig {
    pub relation_type: RelationType,
    /// 目标 content type 名称
    pub target: String,
    /// 多对多中间表名
    pub through: Option<String>,
    /// 外键列名（默认为 "{target}_id"）
    pub foreign_key: Option<String>,
}

/// 媒体字段配置
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// 接受的 MIME 类型
    #[serde(default)]
    pub accept: Vec<String>,
    /// 最大文件数量
    #[serde(default = "default_media_max_count")]
    pub max_count: usize,
}

fn default_media_max_count() -> usize {
    1
}

/// 索引定义
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// 索引包含的字段
    pub fields: Vec<String>,
    /// 是否唯一索引
    #[serde(default)]
    pub unique: bool,
}

/// API 访问级别
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "export-types", ts(rename_all = "lowercase"))]
pub enum ApiAccess {
    /// 完全禁止
    None,
    /// 公开访问，无需认证
    #[default]
    Public,
    /// 需要登录（任意角色）
    Member,
    /// 需要管理员角色
    Admin,
}

/// 单个 API 端点的访问控制配置
///
/// TOML 写法：
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
    /// 访问级别：none / public / member / admin
    #[serde(default)]
    pub access: ApiAccess,
    /// 数据过滤表达式（对所有通过 access 检查的请求生效）
    pub filter: Option<String>,
    /// 已登录用户的额外过滤（与 filter 取 OR）
    pub filter_auth: Option<String>,
    /// 是否启用服务端缓存（默认 false）
    #[serde(default)]
    pub cache: bool,
    /// API 返回字段白名单（默认空=返回全部非 private 字段）
    /// 仅对 list/get 端点有效，create/update/delete 忽略此配置
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

/// API 端点访问配置
#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 列表查询（GET /cms/{plural}）
    #[serde(default)]
    pub list: ApiEndpointConfig,
    /// 单条查询（GET /cms/{plural}/{id}）
    #[serde(default)]
    pub get: ApiEndpointConfig,
    /// 创建（POST /cms/{plural}）
    #[serde(default = "api_endpoint_member")]
    pub create: ApiEndpointConfig,
    /// 更新（PUT /cms/{plural}/{id}）
    #[serde(default = "api_endpoint_member")]
    pub update: ApiEndpointConfig,
    /// 删除（DELETE /cms/{plural}/{id}）
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

/// 检查指定访问级别是否允许当前请求通过
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

/// TOML 解析用的顶层结构
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
    /// 从 TOML 文件解析
    pub fn parse_from_file(path: &Path) -> Result<Self, AppError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("cannot read {path:?}: {e}")))?;
        Self::parse_from_str(&content)
    }

    /// 从 TOML 字符串解析
    pub fn parse_from_str(content: &str) -> Result<Self, AppError> {
        let toml: ContentTypeToml = toml::from_str(content)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("TOML parse error: {e}")))?;

        let mut fields = Vec::new();
        for (name, value) in &toml.fields {
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
                name: name.clone(),
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
            name: toml.content_type.name,
            singular: toml.content_type.singular,
            plural: toml.content_type.plural,
            table: Self::validate_table_name(&toml.content_type.table)?,
            description: toml.content_type.description,
            kind: toml.content_type.kind,
            fields,
            slug_field: toml.content_type.slug_field,
            builtin: toml.content_type.builtin,
            implements: toml.content_type.implements,
            indexes: toml.indexes.unwrap_or_default(),
            api: toml.api.unwrap_or_default(),
            cached_column_names: None,
            cached_protocol_column_names: None,
            cached_behaviors: None,
            cached_declaration: None,
            cached_rules: None,
        })
    }

    /// 缓存协议提供的列名、行为能力、声明（需在 cache_select_columns 之前调用）
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
            .chain(["id", "tenant_id", crate::constants::COL_META])
            .collect();
        registry.apply_config_for(&self.implements, &mut decl, &all_columns);
        self.cached_declaration = Some(decl);
    }

    /// 预计算并缓存 SELECT 列名列表（需在 cache_protocol_columns 之后调用）
    pub fn cache_select_columns(&mut self) {
        self.cached_column_names = Some(crate::content_type::repository::build_column_names(
            self, None, true,
        ));
    }

    /// 获取协议提供的列名（必须先调用 `cache_protocol_columns`）
    pub fn protocol_column_names(&self) -> Vec<&str> {
        self.cached_protocol_column_names
            .as_ref()
            .map(|names| names.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// 检查某个列名是否由协议提供
    pub fn is_protocol_column(&self, name: &str) -> bool {
        self.protocol_column_names().contains(&name)
    }

    /// 获取聚合后的协议声明（必须先调用 `cache_protocol_columns`）
    pub fn declaration(&self) -> crate::protocols::ProtocolDeclaration {
        self.cached_declaration.clone().unwrap_or_default()
    }

    /// 获取协议声明的查询过滤条件
    pub fn query_filters(&self) -> Vec<(String, String)> {
        self.declaration().query_filters
    }

    /// 是否启用软删除
    pub fn is_soft_delete(&self) -> bool {
        self.declaration().is_soft_delete()
    }

    /// 是否提供版本历史路由
    pub fn has_revision_routes(&self) -> bool {
        self.declaration().revision_routes
    }

    /// 校验表名只含安全字符，防止 SQL 注入。
    fn validate_table_name(name: &str) -> Result<String, AppError> {
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

    /// 预解析 API Rule 表达式，schema 注册时调用一次
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

    /// 获取列名列表
    ///
    /// `include_private=false` 时过滤掉 `private` 字段（公开 API 用）。
    /// `include_private=true` 时返回全部字段（admin API 用）。
    pub fn column_names(&self, requested: Option<&[String]>, include_private: bool) -> Vec<String> {
        if include_private
            && requested.is_none()
            && let Some(ref cached) = self.cached_column_names
        {
            return cached.clone();
        }
        crate::content_type::repository::build_column_names(self, requested, include_private)
    }

    /// 获取非私有字段列表
    #[must_use]
    pub fn public_fields(&self) -> Vec<&FieldSchema> {
        self.fields.iter().filter(|f| !f.private).collect()
    }

    /// 获取关系字段列表
    #[must_use]
    pub fn relation_fields(&self) -> Vec<&FieldSchema> {
        self.fields
            .iter()
            .filter(|f| f.field_type == FieldType::Relation)
            .collect()
    }

    /// 根据 field name 查找字段定义
    #[must_use]
    pub fn get_field(&self, name: &str) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// 是否为 Single Type
    #[must_use]
    pub fn is_single(&self) -> bool {
        self.kind == ContentKind::Single
    }

    /// 是否为 Collection Type
    #[must_use]
    pub fn is_collection(&self) -> bool {
        self.kind == ContentKind::Collection
    }

    /// 是否实现了指定 Protocol
    #[must_use]
    pub fn implements_protocol(&self, name: &str) -> bool {
        self.implements.iter().any(|p| p.name() == name)
    }

    /// 获取 UID 字段（用于 slug）
    #[must_use]
    pub fn uid_field(&self) -> Option<&FieldSchema> {
        self.fields.iter().find(|f| f.field_type == FieldType::Uid)
    }

    /// 序列化为 TOML 字符串
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

    /// 保存 TOML 文件到指定目录
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
        let mut rt = toml::Table::new();
        rt.insert(
            "relation_type".into(),
            toml::Value::String(format!("{:?}", rel.relation_type).to_lowercase()),
        );
        rt.insert("target".into(), toml::Value::String(rel.target.clone()));
        if let Some(ref through) = rel.through {
            rt.insert("through".into(), toml::Value::String(through.clone()));
        }
        if let Some(ref fk) = rel.foreign_key {
            rt.insert("foreign_key".into(), toml::Value::String(fk.clone()));
        }
        t.insert(
            "relation_type".into(),
            rt.get("relation_type").unwrap().clone(),
        );
        t.insert("target".into(), rt.get("target").unwrap().clone());
        if rt.contains_key("through") {
            t.insert("through".into(), rt.get("through").unwrap().clone());
        }
        if rt.contains_key("foreign_key") {
            t.insert("foreign_key".into(), rt.get("foreign_key").unwrap().clone());
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

    Ok(RelationConfig {
        relation_type,
        target: table
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        through: table
            .get("through")
            .and_then(|v| v.as_str())
            .map(String::from),
        foreign_key: table
            .get("foreign_key")
            .and_then(|v| v.as_str())
            .map(String::from),
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

/// 创建表单字段（用于 API）
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

/// 更新表单字段（用于 API）
///
/// 与 `CreateContentTypeRequest` 不同，所有字段都是可选的，
/// 只更新请求中提供的字段。`fields` 为整字段列表替换。
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
description = "博客文章"
slug_field = "title"

[fields.title]
type = "text"
required = true
max_length = 200
label = "标题"

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
        test_reg.register(crate::protocols::cacheable::CacheableProtocol);
        let reg = ContentTypeRegistry::load_from_dir(
            dir.path(),
            &crate::config::app::RuleEngineConfig::default(),
            &reserved,
            &[
                "ownable",
                "timestampable",
                "soft_deletable",
                "versionable",
                "cacheable",
            ],
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
}
