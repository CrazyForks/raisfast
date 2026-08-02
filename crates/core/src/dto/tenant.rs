use serde::Serialize;
#[cfg(feature = "export-types")]
use ts_rs::TS;
use utoipa::ToSchema;

use crate::models::tenant::Tenant;

#[cfg_attr(feature = "export-types", derive(TS))]
#[derive(Debug, Serialize, ToSchema)]
pub struct TenantResponse {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub status: String,
    pub config: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TenantResponse {
    pub fn from_tenant(t: Tenant) -> Self {
        Self {
            id: t.id.to_string(),
            name: t.name,
            domain: t.domain,
            status: t.status.to_string(),
            config: Some(t.config),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}
