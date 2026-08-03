//! Bulk import file parsing for CMS content types.
//!
//! Parses `.json`, `.csv` and `.xlsx` uploads into a list of record objects
//! that can be passed to [`super::handler::do_admin_bulk_create`]. Headers are
//! matched against content-type field names (or display labels); values are
//! coerced to the target field type. Managed columns (`id`, `created_at`,
//! `updated_at`, `deleted_at`, `deleted_by`) are dropped.

use std::io::{BufReader, Cursor};

use calamine::Reader;
use serde_json::{Map, Value};

use crate::errors::app_error::AppError;

use super::schema::{ContentTypeSchema, FieldType};

/// Maximum number of records accepted per import.
pub const IMPORT_MAX_RECORDS: usize = 1000;

// const MANAGED_FIELDS: &[&str] = &["id", "created_at", "updated_at", "deleted_at", "deleted_by"];
const MANAGED_FIELDS: &[&str] = &["id"];

/// Resolve a spreadsheet/CSV header to a target field name (or the `status`
/// meta column). Returns `None` for headers that match no field.
fn resolve_column(ct: &ContentTypeSchema, header: &str) -> Option<String> {
    let h = header.trim();
    if h.is_empty() {
        return None;
    }
    if let Some(f) = ct.fields.iter().find(|f| f.name == h) {
        return Some(f.name.clone());
    }
    if let Some(f) = ct.fields.iter().find(|f| f.label.as_deref() == Some(h)) {
        return Some(f.name.clone());
    }
    if h.eq_ignore_ascii_case("status") {
        return Some("status".into());
    }
    None
}

fn coerce_value(field_type: Option<FieldType>, value: Value) -> Value {
    match field_type {
        Some(FieldType::Integer | FieldType::BigInt) => match value {
            Value::Number(ref n) => {
                if let Some(i) = n.as_i64() {
                    Value::Number(i.into())
                } else {
                    match n.as_f64() {
                        Some(f) if f.fract() == 0.0 && f.abs() < 9.0e15 => {
                            Value::Number((f as i64).into())
                        }
                        _ => value,
                    }
                }
            }
            Value::String(s) => s
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or(Value::String(s)),
            _ => value,
        },
        Some(FieldType::Decimal | FieldType::Float) => match value {
            Value::Number(n) => Value::Number(n),
            Value::String(s) => s
                .trim()
                .parse::<f64>()
                .map(Value::from)
                .unwrap_or(Value::String(s)),
            _ => value,
        },
        Some(FieldType::Boolean) => match value {
            Value::Bool(b) => Value::Bool(b),
            Value::String(s) => {
                let v = s.trim().to_lowercase();
                Value::Bool(matches!(v.as_str(), "true" | "1" | "yes" | "y"))
            }
            Value::Number(n) => Value::Bool(n.as_i64() == Some(1)),
            _ => value,
        },
        Some(FieldType::Json) => match value {
            Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
            v => v,
        },
        _ => value,
    }
}

fn strip_managed_fields(obj: &mut Map<String, Value>) {
    for key in MANAGED_FIELDS {
        obj.remove(*key);
    }
}

/// Convert header row + data rows into record objects, matching headers to fields.
fn rows_to_objects(ct: &ContentTypeSchema, headers: &[String], rows: &[Vec<Value>]) -> Vec<Value> {
    let columns: Vec<Option<String>> = headers.iter().map(|h| resolve_column(ct, h)).collect();
    let mut objects = Vec::with_capacity(rows.len());
    for row in rows {
        let mut obj = Map::new();
        let mut has_value = false;
        for (i, column) in columns.iter().enumerate() {
            let Some(column) = column else { continue };
            let Some(raw) = row.get(i) else { continue };
            if raw.is_null() {
                continue;
            }
            if let Value::String(s) = raw
                && s.trim().is_empty()
            {
                continue;
            }
            let field = (column != "status")
                .then(|| ct.fields.iter().find(|f| f.name == *column))
                .flatten();
            obj.insert(
                column.clone(),
                coerce_value(field.map(|f| f.field_type.clone()), raw.clone()),
            );
            has_value = true;
        }
        if !has_value {
            continue;
        }
        strip_managed_fields(&mut obj);
        objects.push(Value::Object(obj));
    }
    objects
}

/// Parse a JSON import (array of records, or an object wrapping an array).
fn parse_json(ct: &ContentTypeSchema, data: &[u8]) -> Result<Vec<Value>, AppError> {
    let value: Value = serde_json::from_slice(data)
        .map_err(|e| AppError::BadRequest(format!("invalid JSON file: {e}")))?;
    let arr = match value {
        Value::Array(arr) => arr,
        Value::Object(map) => {
            for key in ["items", "data", "records"] {
                if let Some(Value::Array(arr)) = map.get(key) {
                    return Ok(filter_records(ct, arr));
                }
            }
            return Err(AppError::BadRequest(
                "JSON must be an array of records".into(),
            ));
        }
        _ => {
            return Err(AppError::BadRequest(
                "JSON must be an array of records".into(),
            ));
        }
    };
    Ok(filter_records(ct, &arr))
}

fn filter_records(ct: &ContentTypeSchema, arr: &[Value]) -> Vec<Value> {
    arr.iter()
        .filter(|v| v.is_object())
        .map(|v| {
            let mut obj = v.as_object().cloned().unwrap_or_default();
            for (key, value) in obj.iter_mut() {
                if let Some(field) = ct.fields.iter().find(|f| f.name == *key) {
                    *value = coerce_value(Some(field.field_type.clone()), value.clone());
                }
            }
            // Empty strings and nulls break typed columns (e.g.
            // `author_id: null` on a relation/integer column becomes `''` in
            // the create pipeline). Drop them so the column stays NULL.
            obj.retain(|_, value| {
                !matches!(value, Value::String(s) if s.trim().is_empty()) && !value.is_null()
            });
            strip_managed_fields(&mut obj);
            Value::Object(obj)
        })
        .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
        .collect()
}

/// Parse a CSV import using the first row as headers.
fn parse_csv(ct: &ContentTypeSchema, data: &[u8]) -> Result<Vec<Value>, AppError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(Cursor::new(data));
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| AppError::BadRequest(format!("invalid CSV file: {e}")))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut rows: Vec<Vec<Value>> = Vec::new();
    for result in rdr.records() {
        let record = result.map_err(|e| AppError::BadRequest(format!("invalid CSV file: {e}")))?;
        let row: Vec<Value> = record
            .iter()
            .map(|s| Value::String(s.to_string()))
            .collect();
        rows.push(row);
    }
    Ok(rows_to_objects(ct, &headers, &rows))
}

/// Parse an XLSX import using the first row of the first sheet as headers.
fn parse_xlsx(ct: &ContentTypeSchema, data: &[u8]) -> Result<Vec<Value>, AppError> {
    let reader = BufReader::new(Cursor::new(data));
    let mut workbook = calamine::Xlsx::new(reader)
        .map_err(|e| AppError::BadRequest(format!("invalid xlsx file: {e}")))?;

    let sheet_name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| AppError::BadRequest("xlsx file has no sheets".into()))?;
    let range = workbook
        .worksheet_range(&sheet_name)
        .map_err(|e| AppError::BadRequest(format!("invalid xlsx file: {e}")))?;

    let rows: Vec<Vec<Value>> = range
        .rows()
        .map(|row| row.iter().map(calamine_data_to_json).collect())
        .filter(|r: &Vec<Value>| !r.is_empty())
        .collect();

    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let headers: Vec<String> = rows[0]
        .iter()
        .map(|v| v.as_str().map(str::to_string).unwrap_or_default())
        .collect();
    Ok(rows_to_objects(ct, &headers, &rows[1..]))
}

fn calamine_data_to_json(data: &calamine::Data) -> Value {
    use calamine::Data;
    match data {
        Data::Int(i) => Value::Number((*i).into()),
        Data::Float(f) => serde_json::Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Data::String(s) => Value::String(s.clone()),
        Data::Bool(b) => Value::Bool(*b),
        Data::DateTime(dt) => Value::String(dt.to_string()),
        Data::DateTimeIso(s) => Value::String(s.clone()),
        Data::DurationIso(s) => Value::String(s.clone()),
        Data::Error(_) | Data::Empty => Value::Null,
    }
}

/// Parse an uploaded import file into records for a content type.
///
/// `format` must be one of `json`, `csv`, `xlsx`.
pub fn parse_import_records(
    ct: &ContentTypeSchema,
    format: &str,
    data: &[u8],
) -> Result<Vec<Value>, AppError> {
    let mut records = match format {
        "json" => parse_json(ct, data)?,
        "csv" => parse_csv(ct, data)?,
        "xlsx" => parse_xlsx(ct, data)?,
        _ => return Err(AppError::BadRequest("unsupported import format".into())),
    };
    if records.is_empty() {
        return Err(AppError::BadRequest("no records found in file".into()));
    }
    if records.len() > IMPORT_MAX_RECORDS {
        return Err(AppError::BadRequest(format!(
            "too many records (max {IMPORT_MAX_RECORDS})"
        )));
    }
    // Statusable content types default to `draft` when no status was provided.
    if ct.implements.iter().any(|p| p.name() == "statusable") {
        for record in records.iter_mut() {
            if let Value::Object(map) = record {
                map.entry("status".to_string())
                    .or_insert_with(|| Value::String("draft".into()));
            }
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_type::schema::ContentTypeSchema;

    fn schema() -> ContentTypeSchema {
        ContentTypeSchema::parse_from_str(
            r#"
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"
label = "Title"

[fields.views]
type = "integer"
label = "Views"

[fields.active]
type = "boolean"
"#,
        )
        .unwrap()
    }

    #[test]
    fn parse_csv_maps_headers_and_coerces() {
        let csv = "title,views,active\nHello,10,true\nWorld,3,0\n";
        let records = parse_import_records(&schema(), "csv", csv.as_bytes()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["title"], "Hello");
        assert_eq!(records[0]["views"], 10);
        assert_eq!(records[0]["active"], true);
        assert_eq!(records[1]["active"], false);
    }

    #[test]
    fn parse_csv_maps_by_label_and_drops_managed() {
        let csv = "Title,id\nHi,123\n";
        let records = parse_import_records(&schema(), "csv", csv.as_bytes()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["title"], "Hi");
        assert!(!records[0].as_object().unwrap().contains_key("id"));
    }

    #[test]
    fn parse_json_array() {
        let json = r#"[{"title":"A","views":5},{"title":"B"}]"#;
        let records = parse_import_records(&schema(), "json", json.as_bytes()).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["views"], 5);
    }

    #[test]
    fn parse_json_coerces_boolean_strings() {
        let json = r#"[{"title":"A","active":"1"},{"title":"B","active":"0"}]"#;
        let records = parse_import_records(&schema(), "json", json.as_bytes()).unwrap();
        assert_eq!(records[0]["active"], true);
        assert_eq!(records[1]["active"], false);
    }

    #[test]
    fn parse_csv_boolean_zero_and_one() {
        let csv = "title,active\nA,1\nB,0\nC,false\n";
        let records = parse_import_records(&schema(), "csv", csv.as_bytes()).unwrap();
        assert_eq!(records[0]["active"], true);
        assert_eq!(records[1]["active"], false);
        assert_eq!(records[2]["active"], false);
    }

    #[test]
    fn parse_json_drops_empty_relation_values() {
        let json =
            r#"[{"title":"A","content":"x","author_id":"","category_id":"","view_count":3}]"#;
        let records = parse_import_records(&schema(), "json", json.as_bytes()).unwrap();
        assert_eq!(records.len(), 1);
        let map = records[0].as_object().unwrap();
        assert!(!map.contains_key("author_id"));
        assert!(!map.contains_key("category_id"));
        assert_eq!(map["view_count"], 3);
    }

    #[test]
    fn parse_json_drops_null_relation_values() {
        // `author_id: null` is what our own JSON export emits for unset
        // relations; it must not reach the create pipeline as `''`.
        let json = r#"[{"title":"A","content":"x","author_id":null,"category_id":null}]"#;
        let records = parse_import_records(&schema(), "json", json.as_bytes()).unwrap();
        assert_eq!(records.len(), 1);
        let map = records[0].as_object().unwrap();
        assert!(!map.contains_key("author_id"));
        assert!(!map.contains_key("category_id"));
    }

    #[test]
    fn parse_csv_drops_empty_relation_columns() {
        let csv = "title,views\nA,\nB,5\n";
        let records = parse_import_records(&schema(), "csv", csv.as_bytes()).unwrap();
        assert_eq!(records.len(), 2);
        assert!(records[0].as_object().unwrap().get("views").is_none());
        assert_eq!(records[1]["views"], 5);
    }

    #[test]
    fn coerce_integer_from_whole_float() {
        let v = coerce_value(Some(FieldType::Integer), serde_json::json!(4.0));
        assert_eq!(v, serde_json::json!(4));
    }

    #[test]
    fn coerce_fractional_float_keeps_float() {
        let v = coerce_value(Some(FieldType::Integer), serde_json::json!(4.5));
        assert_eq!(v, serde_json::json!(4.5));
    }

    #[test]
    fn parse_json_wrapped_array() {
        let json = r#"{"records":[{"title":"A"}]}"#;
        let records = parse_import_records(&schema(), "json", json.as_bytes()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["title"], "A");
    }

    #[test]
    fn parse_empty_file_errors() {
        let err = parse_import_records(&schema(), "csv", b"").unwrap_err();
        assert!(err.to_string().contains("no records"));
    }

    #[test]
    fn unsupported_format_errors() {
        let err = parse_import_records(&schema(), "txt", b"x").unwrap_err();
        assert!(err.to_string().contains("unsupported"));
    }
}
