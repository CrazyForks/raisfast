//! 审计日志服务

use crate::audit::model::{self, AuditEntry};
use crate::db::Pool;
use crate::errors::app_error::AppResult;

/// 审计日志服务
pub struct AuditService {
    pool: Pool,
}

impl AuditService {
    /// 创建审计日志服务实例
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 记录一条审计日志
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
        let (document_id, now) = crate::utils::id::new_document_id_and_timestamp();
        let entry = AuditEntry {
            id: 0,
            document_id,
            tenant_id: Some(tenant_id.to_string()),
            actor_id,
            actor_role: actor_role.map(|s| s.to_string()),
            action: action.to_string(),
            subject: subject.to_string(),
            subject_id: subject_id.map(|s| s.to_string()),
            detail: detail.map(|s| s.to_string()),
            ip_address: ip_address.map(|s| s.to_string()),
            user_agent: user_agent.map(|s| s.to_string()),
            created_at: now,
        };
        model::insert(&self.pool, &entry).await
    }

    pub async fn list(
        &self,
        tenant_id: Option<&str>,
        action: Option<&str>,
        actor_id: Option<i64>,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<AuditEntry>, i64)> {
        model::find_paginated(&self.pool, tenant_id, action, actor_id, page, page_size).await
    }

    pub async fn get(&self, id: i64) -> AppResult<AuditEntry> {
        model::find_by_id(&self.pool, id).await
    }
}
