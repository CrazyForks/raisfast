//! Audit log service

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::models::audit_log::{self, AuditEntry};
use crate::types::snowflake_id::SnowflakeId;

/// Audit log service
pub struct AuditService {
    pool: Pool,
}

impl AuditService {
    /// Create a new audit service instance
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Record an audit log entry
    #[allow(clippy::too_many_arguments)]
    pub async fn log(
        &self,
        tenant_id: &str,
        actor_id: Option<i64>,
        actor_role: Option<&str>,
        action: &str,
        subject: &str,
        subject_id: Option<&str>,
        detail: Option<&str>,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
    ) -> AppResult<()> {
        let (id, now) = (
            crate::utils::id::new_snowflake_id(),
            crate::utils::tz::now_utc(),
        );
        let entry = AuditEntry {
            id,
            tenant_id: Some(tenant_id.to_string()),
            actor_id: actor_id.map(crate::types::snowflake_id::SnowflakeId),
            actor_role: actor_role.map(|s| s.to_string()),
            action: action.to_string(),
            subject: subject.to_string(),
            subject_id: subject_id.map(|s| s.to_string()),
            detail: detail.map(|s| s.to_string()),
            ip_address: ip_address.map(|s| s.to_string()),
            user_agent: user_agent.map(|s| s.to_string()),
            created_at: now,
        };
        audit_log::insert(&self.pool, &entry).await
    }

    pub async fn list(
        &self,
        tenant_id: Option<&str>,
        action: Option<&str>,
        actor_id: Option<i64>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AuditEntry>, i64)> {
        audit_log::find_paginated(&self.pool, tenant_id, action, actor_id, page, page_size).await
    }

    pub async fn get(&self, id: SnowflakeId) -> AppResult<AuditEntry> {
        audit_log::find_by_id(&self.pool, id).await
    }
}
