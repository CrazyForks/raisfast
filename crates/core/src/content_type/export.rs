//! Full-table export for CMS content types (streaming).
//!
//! Records are streamed row-by-row from the database (never materialized in
//! memory as a whole) and serialized incrementally. The handler pushes chunks
//! to the HTTP response as they fill, so exports of hundreds of thousands to
//! millions of records only hold one `CHUNK_SIZE` buffer in memory.
//!
//! - JSON / CSV / SQL are fully streamed.
//! - XLSX is row-streamed into the workbook but the final workbook is
//!   serialized at the end (the `rust_xlsxwriter` 0.x in use has no
//!   constant-memory mode), so XLSX is capped at [`EXPORT_XLSX_MAX_RECORDS`].

use serde_json::{Map, Value};

use crate::errors::app_error::AppError;

use super::schema::ContentTypeSchema;

/// Maximum number of records that may be exported at once.
pub const EXPORT_MAX_RECORDS: usize = 1_000_000;

/// XLSX builds the whole workbook in memory; cap it well below the streaming
/// formats. Spreadsheets are an interactive format, not a bulk-export one.
pub const EXPORT_XLSX_MAX_RECORDS: usize = 100_000;

/// Chunk size pushed to the HTTP response per write.
const CHUNK_SIZE: usize = 64 * 1024;

/// Supported export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Json,
    Csv,
    Sql,
    Xlsx,
}

impl std::str::FromStr for ExportFormat {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "json" => Ok(Self::Json),
            "csv" => Ok(Self::Csv),
            "sql" => Ok(Self::Sql),
            "xlsx" => Ok(Self::Xlsx),
            _ => Err(()),
        }
    }
}

impl ExportFormat {
    /// HTTP `Content-Type` for the exported bytes.
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Sql => "application/sql",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        }
    }

    /// File extension for the download filename.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Sql => "sql",
            Self::Xlsx => "xlsx",
        }
    }
}

/// Render a cell value to its string form (JSON/objects become compact JSON).
fn cell_string(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => {
            if *b {
                "true".into()
            } else {
                "false".into()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => {
            serde_json::to_string(&v).unwrap_or_default()
        }
    }
}

/// CSV-escape a single field (RFC 4180).
fn csv_field(s: &str) -> String {
    if s.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn sql_literal(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "NULL".into(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => {
            if *b {
                "1".into()
            } else {
                "0".into()
            }
        }
        Some(Value::String(s)) => format!("'{}'", s.replace('\'', "''")),
        Some(Value::Array(_)) | Some(Value::Object(_)) => {
            let s = serde_json::to_string(&v).unwrap_or_default();
            format!("'{}'", s.replace('\'', "''"))
        }
    }
}

/// Suggested download filename for an export.
#[must_use]
pub fn suggested_filename(ct: &ContentTypeSchema, format: ExportFormat) -> String {
    format!("{}-{}.{}", ct.plural, today(), format.extension())
}

fn today() -> String {
    use chrono::{Datelike, Utc};
    let now = Utc::now();
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

/// Streaming export sink.
///
/// Call [`ExportSink::write_row`] once per record; it encodes the row into an
/// internal buffer and returns a ready-to-send chunk whenever the buffer fills.
/// Call [`ExportSink::finish`] at the end for trailing bytes.
pub struct ExportSink {
    format: ExportFormat,
    columns: Vec<String>,
    table: String,
    buf: Vec<u8>,
    json_started: bool,
    xlsx: Option<rust_xlsxwriter::Workbook>,
    count: usize,
}

impl ExportSink {
    /// Create a sink for the given content type. The CSV header (or XLSX
    /// header row) is written up front.
    pub fn new(ct: &ContentTypeSchema, format: ExportFormat) -> Self {
        let columns = ct.column_names(None, true);
        let table = ct.table.clone();
        let mut buf = Vec::with_capacity(CHUNK_SIZE);
        let mut xlsx = None;
        match format {
            ExportFormat::Csv => {
                let header: Vec<String> = columns.iter().map(|c| csv_field(c)).collect();
                buf.extend_from_slice(header.join(",").as_bytes());
                buf.push(b'\n');
            }
            ExportFormat::Xlsx => {
                let mut workbook = rust_xlsxwriter::Workbook::new();
                let ws = workbook.add_worksheet();
                for (i, name) in columns.iter().enumerate() {
                    let _ = ws.write_string(0, i as u16, name);
                }
                xlsx = Some(workbook);
            }
            _ => {}
        }
        Self {
            format,
            columns,
            table,
            buf,
            json_started: false,
            xlsx,
            count: 0,
        }
    }

    /// Number of records written so far.
    #[must_use]
    pub fn count(&self) -> usize {
        self.count
    }

    /// Encode one record. Returns a flushable chunk (possibly empty) to send
    /// to the client; callers must forward non-empty chunks.
    pub fn write_row(&mut self, row: &Value) -> Result<Vec<u8>, AppError> {
        self.count += 1;
        if self.count > EXPORT_MAX_RECORDS {
            return Err(AppError::BadRequest(format!(
                "too many records (max {EXPORT_MAX_RECORDS})"
            )));
        }
        match self.format {
            ExportFormat::Json => {
                if self.json_started {
                    self.buf.push(b',');
                } else {
                    self.buf.push(b'[');
                    self.json_started = true;
                }
                serde_json::to_writer(&mut self.buf, row)
                    .map_err(|e| AppError::BadRequest(format!("failed to serialize JSON: {e}")))?;
            }
            ExportFormat::Csv => {
                let map = row.as_object().map_or_else(Map::new, Map::clone);
                let record: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| csv_field(&cell_string(map.get(c))))
                    .collect();
                self.buf.extend_from_slice(record.join(",").as_bytes());
                self.buf.push(b'\n');
            }
            ExportFormat::Sql => {
                let map = row.as_object().map_or_else(Map::new, Map::clone);
                let cols: Vec<String> = self.columns.iter().map(|c| format!("`{c}`")).collect();
                let vals: Vec<String> = self
                    .columns
                    .iter()
                    .map(|c| sql_literal(map.get(c)))
                    .collect();
                self.buf.extend_from_slice(
                    format!(
                        "INSERT INTO `{}` ({}) VALUES ({});\n",
                        self.table,
                        cols.join(", "),
                        vals.join(", ")
                    )
                    .as_bytes(),
                );
            }
            ExportFormat::Xlsx => {
                if self.count > EXPORT_XLSX_MAX_RECORDS {
                    return Err(AppError::BadRequest(format!(
                        "too many records for xlsx export (max {EXPORT_XLSX_MAX_RECORDS}); use json/csv/sql"
                    )));
                }
                let workbook = self.xlsx.as_mut().ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!("xlsx workbook not initialized"))
                })?;
                let ws = workbook
                    .worksheet_from_index(0)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("xlsx worksheet: {e}")))?;
                let map = row.as_object().map_or_else(Map::new, Map::clone);
                let row_idx = self.count as u32;
                for (i, name) in self.columns.iter().enumerate() {
                    let col = i as u16;
                    match map.get(name) {
                        None | Some(Value::Null) => {}
                        Some(Value::Number(n)) => {
                            if let Some(v) = n.as_i64() {
                                ws.write_number(row_idx, col, v as f64).map_err(|e| {
                                    AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                                })?;
                            } else if let Some(f) = n.as_f64() {
                                ws.write_number(row_idx, col, f).map_err(|e| {
                                    AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                                })?;
                            } else {
                                ws.write_string(row_idx, col, n.to_string()).map_err(|e| {
                                    AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                                })?;
                            }
                        }
                        Some(Value::Bool(b)) => {
                            ws.write_boolean(row_idx, col, *b).map_err(|e| {
                                AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                            })?;
                        }
                        Some(Value::String(s)) => {
                            ws.write_string(row_idx, col, s).map_err(|e| {
                                AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                            })?;
                        }
                        Some(_) => {
                            ws.write_string(row_idx, col, cell_string(map.get(name)))
                                .map_err(|e| {
                                    AppError::Internal(anyhow::anyhow!("xlsx write: {e}"))
                                })?;
                        }
                    }
                }
                return Ok(Vec::new());
            }
        }

        if self.buf.len() >= CHUNK_SIZE {
            return Ok(std::mem::take(&mut self.buf));
        }
        Ok(Vec::new())
    }

    /// Flush trailing bytes. For XLSX this serializes the whole workbook.
    pub fn finish(&mut self) -> Result<Vec<u8>, AppError> {
        match self.format {
            ExportFormat::Json => {
                if !self.json_started {
                    self.buf.push(b'[');
                }
                self.buf.push(b']');
                self.buf.push(b'\n');
            }
            ExportFormat::Xlsx => {
                let workbook = self.xlsx.as_mut().ok_or_else(|| {
                    AppError::Internal(anyhow::anyhow!("xlsx workbook not initialized"))
                })?;
                let bytes = workbook
                    .save_to_buffer()
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("xlsx serialize: {e}")))?;
                return Ok(bytes);
            }
            ExportFormat::Csv | ExportFormat::Sql => {}
        }
        Ok(std::mem::take(&mut self.buf))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

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

[fields.views]
type = "integer"

[fields.active]
type = "boolean"
"#,
        )
        .unwrap()
    }

    fn sink_all(format: ExportFormat, rows: &[Value]) -> Vec<u8> {
        let ct = schema();
        let mut sink = ExportSink::new(&ct, format);
        for row in rows {
            sink.write_row(row).unwrap();
        }
        sink.finish().unwrap()
    }

    fn rows() -> Vec<Value> {
        serde_json::from_str(
            r#"[
                {"id":"1","title":"Hello","views":10,"active":true},
                {"id":"2","title":"World","views":0,"active":false}
            ]"#,
        )
        .unwrap()
    }

    #[test]
    fn format_parse_roundtrip() {
        assert_eq!(ExportFormat::from_str("json"), Ok(ExportFormat::Json));
        assert_eq!(ExportFormat::from_str("csv"), Ok(ExportFormat::Csv));
        assert_eq!(ExportFormat::from_str("sql"), Ok(ExportFormat::Sql));
        assert_eq!(ExportFormat::from_str("xlsx"), Ok(ExportFormat::Xlsx));
        assert!(ExportFormat::from_str("txt").is_err());
    }

    #[test]
    fn json_streams_all_rows() {
        let bytes = sink_all(ExportFormat::Json, &rows());
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 2);
        assert_eq!(parsed[0]["title"], "Hello");
    }

    #[test]
    fn json_empty_is_valid_array() {
        let bytes = sink_all(ExportFormat::Json, &[]);
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 0);
    }

    #[test]
    fn csv_has_headers_and_rows() {
        let text = String::from_utf8(sink_all(ExportFormat::Csv, &rows())).unwrap();
        assert!(text.starts_with("id,title,views,active"));
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn csv_escapes_commas_and_quotes() {
        let rows = vec![serde_json::json!({"title": "a,b\"c", "views": 1})];
        let text = String::from_utf8(sink_all(ExportFormat::Csv, &rows)).unwrap();
        assert!(text.contains("\"a,b\"\"c\""));
    }

    #[test]
    fn sql_emits_inserts_and_escapes_quotes() {
        let rows = vec![serde_json::json!({"title": "O'Brien", "views": 3, "active": true})];
        let text = String::from_utf8(sink_all(ExportFormat::Sql, &rows)).unwrap();
        assert!(text.contains("INSERT INTO `posts`"));
        assert!(text.contains("'O''Brien'"));
        assert!(text.contains("3"));
        assert!(text.contains("1"));
    }

    #[test]
    fn xlsx_builds_workbook() {
        let bytes = sink_all(ExportFormat::Xlsx, &rows());
        assert!(bytes.starts_with(b"PK"));
    }

    #[test]
    fn xlsx_rejects_oversized() {
        let ct = schema();
        let mut sink = ExportSink::new(&ct, ExportFormat::Xlsx);
        let row = serde_json::json!({"title": "x", "views": 1});
        for _ in 0..(EXPORT_XLSX_MAX_RECORDS as u64) {
            sink.write_row(&row).unwrap();
        }
        let err = sink.write_row(&row).unwrap_err();
        assert!(err.to_string().contains("too many records"));
    }
}
