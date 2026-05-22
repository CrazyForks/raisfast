use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AuditFilter {
    pub action: Option<String>,
    pub actor_id: Option<i64>,
}
