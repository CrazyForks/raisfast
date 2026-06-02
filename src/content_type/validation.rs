//! Content Type field validation
//!
//! Validates input data against schema definitions before create/update operations:
//! - `required` — must exist and be non-empty on create
//! - `immutable` — cannot be modified on update
//! - `unique` — database uniqueness check
//! - `enum_values` — enum value validity
//! - `min` / `max` — numeric range
//! - `max_length` — text length
//! - `pattern` — regex matching
//! - Type compatibility — field value type must match `FieldType`

use crate::types::snowflake_id::SnowflakeId;
use serde_json::Value;
use sqlx::Row;

use super::schema::{ContentTypeSchema, FieldType, RelationType};
use crate::constants::COL_ID;
use crate::db::DbDriver;
use crate::db::Pool;
use crate::errors::app_error::AppError;

/// Validate create data
pub async fn validate_create(
    pool: &Pool,
    ct: &ContentTypeSchema,
    data: &Value,
) -> Result<(), AppError> {
    do_validate_create(pool, ct, data).await
}

/// Validate create data (within transaction)
pub async fn validate_create_tx(
    pool: &Pool,
    ct: &ContentTypeSchema,
    data: &Value,
) -> Result<(), AppError> {
    do_validate_create(pool, ct, data).await
}

async fn do_validate_create(
    pool: &Pool,
    ct: &ContentTypeSchema,
    data: &Value,
) -> Result<(), AppError> {
    let obj = data
        .as_object()
        .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

    let mut errors = Vec::new();

    for field in &ct.fields {
        let val = obj.get(&field.name);

        if field.required && val.is_none_or(is_empty_value) {
            errors.push(format!("field '{}' is required", field.name));
            continue;
        }

        let Some(val) = val else { continue };
        if is_empty_value(val) {
            continue;
        }

        if let Err(e) = check_type(&field.field_type, val) {
            errors.push(format!("field '{}': {e}", field.name));
            continue;
        }

        if field.field_type == FieldType::Relation {
            if let Some(ref rel) = field.relation {
                match rel.relation_type {
                    RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                        if !val.is_string() && !val.is_null() {
                            errors.push(format!(
                                "field '{}': expected string (id) or null",
                                field.name
                            ));
                        }
                    }
                    RelationType::ManyToMany | RelationType::ManyWay => {
                        if !val.is_array() && !val.is_null() {
                            errors.push(format!(
                                "field '{}': expected array of ids or null",
                                field.name
                            ));
                        }
                    }
                    RelationType::OneToMany => {}
                }
            }
            continue;
        }

        if field.field_type == FieldType::Enum
            && let Some(ref allowed) = field.enum_values
        {
            let s = val.as_str().unwrap_or("");
            if !allowed.contains(&s.to_string()) {
                errors.push(format!(
                    "field '{}': value '{}' is not one of [{}]",
                    field.name,
                    s,
                    allowed.join(", ")
                ));
            }
        }

        if let Some(max_len) = field.max_length
            && let Some(s) = val.as_str()
            && s.len() > max_len
        {
            errors.push(format!(
                "field '{}': length {} exceeds max_length {}",
                field.name,
                s.len(),
                max_len
            ));
        }

        if matches!(
            field.field_type,
            FieldType::Integer | FieldType::BigInt | FieldType::Decimal | FieldType::Float
        ) && let Some(n) = val.as_f64()
        {
            if let Some(min) = field.min
                && n < min
            {
                errors.push(format!(
                    "field '{}': value {} is less than min {}",
                    field.name, n, min
                ));
            }
            if let Some(max) = field.max
                && n > max
            {
                errors.push(format!(
                    "field '{}': value {} exceeds max {}",
                    field.name, n, max
                ));
            }
        }

        if matches!(
            field.field_type,
            FieldType::Integer | FieldType::BigInt | FieldType::Decimal | FieldType::Float
        ) {
            let as_f64 = val
                .as_f64()
                .or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()));
            if let Some(n) = as_f64 {
                if let Some(min) = field.min
                    && n < min
                {
                    errors.push(format!(
                        "field '{}': value {} is less than min {}",
                        field.name, n, min
                    ));
                }
                if let Some(max) = field.max
                    && n > max
                {
                    errors.push(format!(
                        "field '{}': value {} exceeds max {}",
                        field.name, n, max
                    ));
                }
            }
        }

        if let Some(ref pattern) = field.pattern
            && let Some(s) = val.as_str()
            && let Ok(re) = regex::Regex::new(pattern)
            && !re.is_match(s)
        {
            errors.push(format!(
                "field '{}': value '{}' does not match pattern '{}'",
                field.name, s, pattern
            ));
        }
    }

    check_unique_fields(pool, ct, obj, None, &mut errors).await?;

    finish_validation(errors)
}

/// Validate update data
pub async fn validate_update(
    pool: &Pool,
    ct: &ContentTypeSchema,
    id: SnowflakeId,
    data: &Value,
) -> Result<(), AppError> {
    do_validate_update(pool, ct, id, data).await
}

/// Validate update data (within transaction)
pub async fn validate_update_tx(
    pool: &Pool,
    ct: &ContentTypeSchema,
    id: SnowflakeId,
    data: &Value,
) -> Result<(), AppError> {
    do_validate_update(pool, ct, id, data).await
}

async fn do_validate_update(
    pool: &Pool,
    ct: &ContentTypeSchema,
    id: SnowflakeId,
    data: &Value,
) -> Result<(), AppError> {
    let obj = data
        .as_object()
        .ok_or_else(|| AppError::BadRequest("request body must be a JSON object".into()))?;

    let mut errors = Vec::new();

    for field in &ct.fields {
        if field.immutable && obj.contains_key(&field.name) {
            errors.push(format!("field '{}' is immutable", field.name));
        }
    }

    for field in &ct.fields {
        let Some(val) = obj.get(&field.name) else {
            continue;
        };
        if is_empty_value(val) {
            continue;
        }

        if let Err(e) = check_type(&field.field_type, val) {
            errors.push(format!("field '{}': {e}", field.name));
            continue;
        }

        if field.field_type == FieldType::Relation {
            if let Some(ref rel) = field.relation {
                match rel.relation_type {
                    RelationType::ManyToOne | RelationType::OneToOne | RelationType::OneWay => {
                        if !val.is_string() && !val.is_null() {
                            errors.push(format!(
                                "field '{}': expected string (id) or null",
                                field.name
                            ));
                        }
                    }
                    RelationType::ManyToMany | RelationType::ManyWay => {
                        if !val.is_array() && !val.is_null() {
                            errors.push(format!(
                                "field '{}': expected array of ids or null",
                                field.name
                            ));
                        }
                    }
                    RelationType::OneToMany => {}
                }
            }
            continue;
        }

        if field.field_type == FieldType::Enum
            && let Some(ref allowed) = field.enum_values
        {
            let s = val.as_str().unwrap_or("");
            if !allowed.contains(&s.to_string()) {
                errors.push(format!(
                    "field '{}': value '{}' is not one of [{}]",
                    field.name,
                    s,
                    allowed.join(", ")
                ));
            }
        }

        if let Some(max_len) = field.max_length
            && let Some(s) = val.as_str()
            && s.len() > max_len
        {
            errors.push(format!(
                "field '{}': length {} exceeds max_length {}",
                field.name,
                s.len(),
                max_len
            ));
        }

        if matches!(
            field.field_type,
            FieldType::Integer | FieldType::BigInt | FieldType::Decimal | FieldType::Float
        ) && let Some(n) = val.as_f64()
        {
            if let Some(min) = field.min
                && n < min
            {
                errors.push(format!(
                    "field '{}': value {} is less than min {}",
                    field.name, n, min
                ));
            }
            if let Some(max) = field.max
                && n > max
            {
                errors.push(format!(
                    "field '{}': value {} exceeds max {}",
                    field.name, n, max
                ));
            }
        }

        if matches!(
            field.field_type,
            FieldType::Integer | FieldType::BigInt | FieldType::Decimal | FieldType::Float
        ) {
            let as_f64 = val
                .as_f64()
                .or_else(|| val.as_str().and_then(|s| s.parse::<f64>().ok()));
            if let Some(n) = as_f64 {
                if let Some(min) = field.min
                    && n < min
                {
                    errors.push(format!(
                        "field '{}': value {} is less than min {}",
                        field.name, n, min
                    ));
                }
                if let Some(max) = field.max
                    && n > max
                {
                    errors.push(format!(
                        "field '{}': value {} exceeds max {}",
                        field.name, n, max
                    ));
                }
            }
        }

        if let Some(ref pattern) = field.pattern
            && let Some(s) = val.as_str()
            && let Ok(re) = regex::Regex::new(pattern)
            && !re.is_match(s)
        {
            errors.push(format!(
                "field '{}': value '{}' does not match pattern '{}'",
                field.name, s, pattern
            ));
        }
    }

    check_unique_fields(pool, ct, obj, Some(*id), &mut errors).await?;

    finish_validation(errors)
}

async fn check_unique_fields(
    pool: &Pool,
    ct: &ContentTypeSchema,
    obj: &serde_json::Map<String, Value>,
    exclude_id: Option<i64>,
    errors: &mut Vec<String>,
) -> Result<(), AppError> {
    let mut sql_builder = String::new();
    for field in &ct.fields {
        if !field.unique {
            continue;
        }
        let Some(val) = obj.get(&field.name) else {
            continue;
        };
        if is_empty_value(val) {
            continue;
        }

        let val_str = value_to_db_string(val);
        sql_builder.clear();
        if exclude_id.is_some() {
            sql_builder = format!(
                "SELECT COUNT(*) as cnt FROM {} WHERE {} = {} AND {COL_ID} != {}",
                ct.table,
                field.name,
                crate::db::Driver::ph(1),
                crate::db::Driver::ph(2)
            );
        } else {
            sql_builder = format!(
                "SELECT COUNT(*) as cnt FROM {} WHERE {} = {}",
                ct.table,
                field.name,
                crate::db::Driver::ph(1)
            );
        }
        let sql = &sql_builder;

        let row = if let Some(id) = exclude_id {
            sqlx::query(sql)
                .bind(&val_str)
                .bind(id)
                .fetch_one(pool)
                .await
        } else {
            sqlx::query(sql).bind(&val_str).fetch_one(pool).await
        }
        .map_err(|e| AppError::Internal(anyhow::anyhow!("unique check failed: {e}")))?;

        let count: i64 = row.try_get("cnt").unwrap_or(0);
        if count > 0 {
            errors.push(format!("field '{}': value already exists", field.name));
        }
    }
    Ok(())
}

fn finish_validation(errors: Vec<String>) -> Result<(), AppError> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::BadRequest(errors.join("; ")))
    }
}

fn is_empty_value(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        _ => false,
    }
}

fn check_type(ft: &FieldType, val: &Value) -> Result<(), String> {
    match ft {
        FieldType::Text | FieldType::RichText | FieldType::Password => {
            if !val.is_string() {
                return Err("expected string".into());
            }
        }
        FieldType::Email => {
            if !val.is_string() {
                return Err("expected string".into());
            }
            let s = val.as_str().unwrap_or("");
            if !s.contains('@') || !s.contains('.') || s.starts_with('@') || s.ends_with('@') {
                return Err("invalid email format".into());
            }
        }
        FieldType::Integer | FieldType::BigInt => {
            if !val.is_i64() {
                return Err("expected integer".into());
            }
        }
        FieldType::Decimal => {
            if !val.is_f64() && !val.is_i64() && !val.is_string() {
                return Err("expected number or numeric string".into());
            }
        }
        FieldType::Float => {
            if !val.is_f64() && !val.is_i64() {
                return Err("expected number".into());
            }
        }
        FieldType::Boolean => {
            if !val.is_boolean() {
                return Err("expected boolean".into());
            }
        }
        FieldType::Date | FieldType::DateTime | FieldType::Time => {
            if !val.is_string() {
                return Err("expected string (date/time)".into());
            }
        }
        FieldType::Enum => {
            if !val.is_string() {
                return Err("expected string (enum)".into());
            }
        }
        FieldType::Uid => {
            if !val.is_string() {
                return Err("expected string (uid)".into());
            }
        }
        FieldType::Json => {}
        FieldType::Media => {
            if !val.is_string() {
                return Err("expected string (media url)".into());
            }
        }
        FieldType::Relation => {}
    }
    Ok(())
}

fn value_to_db_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => {
            if *b {
                "1".into()
            } else {
                "0".into()
            }
        }
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_type::schema::ContentTypeSchema;
    use serde_json::json;

    fn make_test_ct() -> ContentTypeSchema {
        ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "ct_validation_test"
implements = ["ownable", "timestampable"]

[fields.name]
type = "text"
required = true
max_length = 50

[fields.code]
type = "text"
unique = true

[fields.price]
type = "float"
min = 0

[fields.quantity]
type = "integer"
min = 0
max = 99999

[fields.status]
type = "enum"
enum_values = ["draft", "published", "archived"]
default = "draft"

[fields.secret]
type = "text"
immutable = true
"#,
        )
        .unwrap()
    }

    #[test]
    fn check_type_text_expects_string() {
        assert!(check_type(&FieldType::Text, &json!("hello")).is_ok());
        assert!(check_type(&FieldType::Text, &json!(42)).is_err());
    }

    #[test]
    fn check_type_integer_expects_number() {
        assert!(check_type(&FieldType::Integer, &json!(42)).is_ok());
        assert!(check_type(&FieldType::Integer, &json!("42")).is_err());
    }

    fn test_protocol_registry() -> crate::protocols::ProtocolRegistry {
        let mut reg = crate::protocols::ProtocolRegistry::new();
        reg.register(crate::protocols::ownable::OwnableProtocol);
        reg.register(crate::protocols::timestampable::TimestampableProtocol);
        reg.register(crate::protocols::soft_deletable::SoftDeletableProtocol);
        reg.register(crate::protocols::versionable::VersionableProtocol);
        reg.register(crate::protocols::lockable::LockableProtocol);
        reg.register(crate::protocols::sortable::SortableProtocol);
        reg.register(crate::protocols::expirable::ExpirableProtocol);
        reg.register(crate::protocols::nestable::NestableProtocol);
        reg.register(crate::protocols::tenantable::TenantableProtocol);
        reg
    }

    #[test]
    fn check_type_boolean_expects_bool() {
        assert!(check_type(&FieldType::Boolean, &json!(true)).is_ok());
        assert!(check_type(&FieldType::Boolean, &json!(1)).is_err());
    }

    #[tokio::test]
    async fn validate_create_missing_required() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let data = json!({"price": 9.99});
        let result = validate_create(&pool, &ct, &data).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("required"), "expected 'required' in: {msg}");
    }

    #[tokio::test]
    async fn validate_create_bad_enum() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let data = json!({"name": "Test", "status": "unknown"});
        let result = validate_create(&pool, &ct, &data).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not one of"), "expected enum error in: {msg}");
    }

    #[tokio::test]
    async fn validate_create_exceeds_max_length() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let data = json!({"name": "x".repeat(51)});
        let result = validate_create(&pool, &ct, &data).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("max_length"),
            "expected max_length error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_create_number_out_of_range() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let data = json!({"name": "Test", "price": -5.0});
        let result = validate_create(&pool, &ct, &data).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("min"), "expected min error in: {msg}");
    }

    #[tokio::test]
    async fn validate_create_unique_duplicate() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let now = crate::utils::tz::now_str();
        repo.create(
            &ct,
            json!({"name": "A", "code": "ABC", "created_at": now, "updated_at": now}),
            None,
            &Default::default(),
        )
        .await
        .unwrap();

        let data = json!({"name": "B", "code": "ABC"});
        let result = validate_create(&pool, &ct, &data).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already exists"),
            "expected unique error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_update_immutable_field() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let now = crate::utils::tz::now_str();
        let created = repo
            .create(
                &ct,
                json!({"name": "Test", "secret": "sauce", "created_at": now, "updated_at": now}),
                None,
                &Default::default(),
            )
            .await
            .unwrap();
        let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

        let result = validate_update(&pool, &ct, SnowflakeId(id), &json!({"secret": "new"})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("immutable"),
            "expected immutable error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_update_unique_excludes_self() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let now = crate::utils::tz::now_str();
        let created = repo
            .create(
                &ct,
                json!({"name": "Test", "code": "XYZ", "created_at": now, "updated_at": now}),
                None,
                &Default::default(),
            )
            .await
            .unwrap();
        let id: i64 = created["id"].as_str().unwrap().parse().unwrap();

        let result = validate_update(
            &pool,
            &ct,
            SnowflakeId(id),
            &json!({"name": "Updated", "code": "XYZ"}),
        )
        .await;
        assert!(result.is_ok(), "updating same unique value should be ok");
    }

    #[test]
    fn is_empty_value_various() {
        assert!(is_empty_value(&Value::Null));
        assert!(is_empty_value(&Value::String(String::new())));
        assert!(!is_empty_value(&Value::String("x".into())));
        assert!(!is_empty_value(&Value::Number(0.into())));
        assert!(!is_empty_value(&Value::Bool(false)));
    }

    #[test]
    fn value_to_db_string_various() {
        assert_eq!(value_to_db_string(&json!("hello")), "hello");
        assert_eq!(value_to_db_string(&json!(42)), "42");
        assert_eq!(value_to_db_string(&json!(true)), "1");
        assert_eq!(value_to_db_string(&json!(false)), "0");
        assert_eq!(value_to_db_string(&json!(null)), "null");
    }

    #[test]
    fn check_type_all_text_variants() {
        for ft in &[FieldType::Text, FieldType::RichText, FieldType::Password] {
            assert!(check_type(ft, &json!("hello")).is_ok());
            assert!(check_type(ft, &json!(42)).is_err());
        }
        // Email requires valid format
        assert!(check_type(&FieldType::Email, &json!("hello@example.com")).is_ok());
        assert!(check_type(&FieldType::Email, &json!("hello")).is_err());
        assert!(check_type(&FieldType::Email, &json!(42)).is_err());
    }

    #[test]
    fn check_type_bigint() {
        assert!(check_type(&FieldType::BigInt, &json!(42)).is_ok());
        assert!(check_type(&FieldType::BigInt, &json!("42")).is_err());
    }

    #[test]
    fn check_type_float_accepts_int_and_float() {
        assert!(check_type(&FieldType::Float, &json!(3.15)).is_ok());
        assert!(check_type(&FieldType::Float, &json!(42)).is_ok());
        assert!(check_type(&FieldType::Float, &json!("42")).is_err());
    }

    #[test]
    fn check_type_decimal_accepts_int_and_float() {
        assert!(check_type(&FieldType::Decimal, &json!(9.99)).is_ok());
        assert!(check_type(&FieldType::Decimal, &json!(42)).is_ok());
    }

    #[test]
    fn check_type_decimal_accepts_numeric_string() {
        assert!(check_type(&FieldType::Decimal, &json!("9.99")).is_ok());
        assert!(check_type(&FieldType::Decimal, &json!("42")).is_ok());
    }

    #[test]
    fn check_type_date_variants() {
        for ft in &[FieldType::Date, FieldType::DateTime, FieldType::Time] {
            assert!(check_type(ft, &json!("2024-01-01")).is_ok());
            assert!(check_type(ft, &json!(42)).is_err());
        }
    }

    #[test]
    fn check_type_uid_expects_string() {
        assert!(check_type(&FieldType::Uid, &json!("abc123")).is_ok());
        assert!(check_type(&FieldType::Uid, &json!(42)).is_err());
    }

    #[test]
    fn check_type_json_accepts_anything() {
        assert!(check_type(&FieldType::Json, &json!({"key": "val"})).is_ok());
        assert!(check_type(&FieldType::Json, &json!(42)).is_ok());
        assert!(check_type(&FieldType::Json, &json!(null)).is_ok());
    }

    #[test]
    fn check_type_media_expects_string() {
        assert!(check_type(&FieldType::Media, &json!("/img/a.png")).is_ok());
        assert!(check_type(&FieldType::Media, &json!(42)).is_err());
    }

    #[test]
    fn check_type_relation_accepts_anything() {
        assert!(check_type(&FieldType::Relation, &json!("anything")).is_ok());
        assert!(check_type(&FieldType::Relation, &json!(42)).is_ok());
    }

    #[tokio::test]
    async fn validate_create_non_object_body() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!("not an object")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_update_non_object_body() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = make_test_ct();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_update(&pool, &ct, SnowflakeId(0), &json!(42)).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_create_with_pattern_mismatch() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_pattern_test"

[fields.code]
type = "text"
pattern = "^[A-Z]{3}$"
"#,
        )
        .unwrap();
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"code": "abc"})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("pattern"), "expected pattern error in: {msg}");
    }

    #[tokio::test]
    async fn validate_create_with_pattern_match() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_pattern_ok_test"

[fields.code]
type = "text"
pattern = "^[A-Z]{3}$"
"#,
        )
        .unwrap();
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"code": "ABC"})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn validate_create_max_exceeded() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_max_test"

[fields.qty]
type = "integer"
max = 100
"#,
        )
        .unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"qty": 200})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("max"), "expected max error in: {msg}");
    }

    #[tokio::test]
    async fn validate_create_wrong_type_for_field() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_wrong_type_test"

[fields.title]
type = "text"
"#,
        )
        .unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"title": 123})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("expected string"),
            "expected type error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_create_with_m2o_relation_wrong_type() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_rel_type_test"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
"#,
        )
        .unwrap();
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"author": 123})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("expected string"),
            "expected relation error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_create_with_m2m_relation_wrong_type() {
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_rel_m2m_test"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
"#,
        )
        .unwrap();
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"tags": "not-array"})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("expected array"),
            "expected array error in: {msg}"
        );
    }

    #[tokio::test]
    async fn validate_create_required_but_empty_string() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_empty_req_test"

[fields.title]
type = "text"
required = true
"#,
        )
        .unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"title": ""})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn validate_create_required_but_null() {
        let pool = crate::db::Pool::connect(":memory:").await.unwrap();
        let ct = ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "T"
singular = "t"
plural = "ts"
table = "ct_null_req_test"

[fields.title]
type = "text"
required = true
"#,
        )
        .unwrap();
        let repo = crate::content_type::repository::ContentRepository::new(pool.clone());
        repo.migrate(&ct, &test_protocol_registry()).await.unwrap();

        let result = validate_create(&pool, &ct, &json!({"title": null})).await;
        assert!(result.is_err());
    }
}
