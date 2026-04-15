//! 内容类型 Schema 数据结构与 TOML 解析

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::errors::app_error::AppError;

/// 内容类型定义
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
    /// 描述
    #[serde(default)]
    pub description: String,
    /// 字段列表
    pub fields: Vec<FieldSchema>,
    /// 是否支持 draft/published/archived 状态
    #[serde(default)]
    pub draft_publish: bool,
    /// 自动从哪个字段生成 slug
    pub slug_field: Option<String>,
    /// 是否自动维护 `created_at` / `updated_at`
    #[serde(default = "default_true")]
    pub timestamps: bool,
    /// 是否软删除
    #[serde(default)]
    pub soft_delete: bool,
    /// 索引定义
    #[serde(default)]
    pub indexes: Vec<IndexDef>,
    /// 列表视图配置
    #[serde(default)]
    pub list_view: Option<ListViewConfig>,
}

/// 字段定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSchema {
    /// 字段名
    pub name: String,
    /// 字段类型
    #[serde(rename = "type")]
    pub field_type: FieldType,
    /// 是否必填
    #[serde(default)]
    pub required: bool,
    /// 是否唯一
    #[serde(default)]
    pub unique: bool,
    /// 默认值
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// 私有字段，不出现在 API 响应中
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RelationType {
    OneToOne,
    OneToMany,
    ManyToOne,
    ManyToMany,
    OneWay,
    ManyWay,
}

/// 关系配置
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// 索引包含的字段
    pub fields: Vec<String>,
    /// 是否唯一索引
    #[serde(default)]
    pub unique: bool,
}

/// 列表视图配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListViewConfig {
    /// 默认排序（如 "`created_at:desc`"）
    #[serde(default = "default_sort")]
    pub default_sort: String,
    /// 列表显示的列
    #[serde(default)]
    pub columns: Vec<String>,
}

fn default_sort() -> String {
    "created_at:desc".into()
}

fn default_true() -> bool {
    true
}

/// TOML 解析用的顶层结构
#[derive(Debug, Deserialize)]
struct ContentTypeToml {
    content_type: ContentTypeHeader,
    fields: toml::Table,
    list_view: Option<ListViewConfig>,
    indexes: Option<Vec<IndexDef>>,
}

#[derive(Debug, Deserialize)]
struct ContentTypeHeader {
    name: String,
    singular: String,
    plural: String,
    table: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    draft_publish: bool,
    slug_field: Option<String>,
    #[serde(default = "default_true")]
    timestamps: bool,
    #[serde(default)]
    soft_delete: bool,
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
                min: field_toml.get("min").and_then(toml::Value::as_float),
                max: field_toml.get("max").and_then(toml::Value::as_float),
                pattern: field_toml
                    .get("pattern")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                relation,
                media_config,
                enum_values: field_toml
                    .get("values")
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
            table: toml.content_type.table,
            description: toml.content_type.description,
            fields,
            draft_publish: toml.content_type.draft_publish,
            slug_field: toml.content_type.slug_field,
            timestamps: toml.content_type.timestamps,
            soft_delete: toml.content_type.soft_delete,
            indexes: toml.indexes.unwrap_or_default(),
            list_view: toml.list_view,
        })
    }

    /// 获取非私有字段列表（API 响应用）
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
        if self.draft_publish {
            header.insert("draft_publish".into(), toml::Value::Boolean(true));
        }
        if let Some(ref sf) = self.slug_field {
            header.insert("slug_field".into(), toml::Value::String(sf.clone()));
        }
        if !self.timestamps {
            header.insert("timestamps".into(), toml::Value::Boolean(false));
        }
        if self.soft_delete {
            header.insert("soft_delete".into(), toml::Value::Boolean(true));
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

        if let Some(ref lv) = self.list_view {
            let mut lv_table = toml::Table::new();
            lv_table.insert(
                "default_sort".into(),
                toml::Value::String(lv.default_sort.clone()),
            );
            lv_table.insert(
                "columns".into(),
                toml::Value::Array(
                    lv.columns
                        .iter()
                        .map(|c| toml::Value::String(c.clone()))
                        .collect(),
                ),
            );
            root.insert("list_view".into(), toml::Value::Table(lv_table));
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
            "values".into(),
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
    pub draft_publish: bool,
    pub slug_field: Option<String>,
    #[serde(default = "default_true")]
    pub timestamps: bool,
    #[serde(default)]
    pub soft_delete: bool,
    #[serde(default)]
    pub fields: Vec<FieldSchema>,
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
        assert!(!ct.draft_publish);
        assert!(ct.timestamps);
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
draft_publish = true
slug_field = "title"
timestamps = true
soft_delete = false

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
values = ["draft", "published", "archived"]
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

[list_view]
default_sort = "is_pinned:desc,created_at:desc"
columns = ["title", "status", "created_at"]
"#;
        let ct = ContentTypeSchema::parse_from_str(toml).unwrap();
        assert_eq!(ct.name, "Post");
        assert!(ct.draft_publish);
        assert_eq!(ct.slug_field, Some("title".into()));
        assert_eq!(ct.fields.len(), 8);
        assert_eq!(ct.indexes.len(), 1);
        assert_eq!(ct.list_view.as_ref().unwrap().columns.len(), 3);

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
values = ["a", "b"]

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
        let path1 = dir.path().join("post.toml");
        let path2 = dir.path().join("page.toml");
        std::fs::write(
            &path1,
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        std::fs::write(
            &path2,
            r#"
[content_type]
name = "Page"
singular = "page"
plural = "pages"
table = "pages"

[fields.title]
type = "text"
"#,
        )
        .unwrap();

        let reg = ContentTypeRegistry::load_from_dir(dir.path()).unwrap();
        assert_eq!(reg.len(), 2);
        assert!(reg.get("post").is_some());
        assert!(reg.get("page").is_some());
        assert!(reg.get("nonexistent").is_none());
        assert!(reg.get_by_table("posts").is_some());
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
}
