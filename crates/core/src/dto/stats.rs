use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TrendsQuery {
    pub table: Option<String>,
    pub days: Option<i64>,
}
