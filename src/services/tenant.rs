//! 租户服务层 — 租户 CRUD + 配置覆盖

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::errors::app_error::AppError;
use crate::models::tenant::Tenant;
use crate::repositories::TenantRepository;

#[derive(Debug, Deserialize)]
pub struct CreateTenantRequest {
    pub name: String,
    pub domain: Option<String>,
    pub config: Option<HashMap<String, Value>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub domain: Option<String>,
    pub config: Option<HashMap<String, Value>>,
    pub status: Option<String>,
}

/// 租户服务
pub struct TenantService {
    repo: Arc<dyn TenantRepository>,
}

impl TenantService {
    /// 创建 `TenantService` 实例
    pub fn new(repo: Arc<dyn TenantRepository>) -> Self {
        Self { repo }
    }

    /// 列出所有租户
    pub async fn list(&self) -> Result<Vec<Tenant>, AppError> {
        self.repo.find_all().await
    }

    /// 根据 ID 获取租户
    pub async fn get(&self, id: &str) -> Result<Option<Tenant>, AppError> {
        self.repo.find_by_id(id).await
    }

    /// 根据域名获取租户
    pub async fn get_by_domain(&self, domain: &str) -> Result<Option<Tenant>, AppError> {
        self.repo.find_by_domain(domain).await
    }

    /// 创建租户
    pub async fn create(&self, req: &CreateTenantRequest) -> Result<Tenant, AppError> {
        let (id, now) = crate::utils::id::new_id_and_timestamp();
        let config = req.config.as_ref().map_or_else(
            || "{}".into(),
            |c| serde_json::to_string(c).unwrap_or_else(|_| "{}".into()),
        );
        self.repo
            .create(&id, &req.name, req.domain.as_deref(), &config, &now)
            .await
    }

    /// 更新租户
    pub async fn update(&self, id: &str, req: &UpdateTenantRequest) -> Result<Tenant, AppError> {
        let now = crate::utils::tz::now_str();
        let config = req
            .config
            .as_ref()
            .map(|c| serde_json::to_string(c).unwrap_or_else(|_| "{}".into()));
        self.repo
            .update(
                id,
                req.name.as_deref(),
                req.domain.as_deref(),
                config.as_deref(),
                req.status.as_deref(),
                &now,
            )
            .await
    }

    /// 删除租户（默认租户不可删除）
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        if id == crate::constants::DEFAULT_TENANT {
            return Err(AppError::BadRequest("cannot delete default tenant".into()));
        }
        self.repo.delete(id).await
    }

    /// 解析租户 ID（从 header 或默认值）
    pub async fn resolve_tenant_id(&self, tenant_id: Option<&str>) -> Result<String, AppError> {
        let id = crate::db::tenant::resolve_tenant(tenant_id);
        let tenant = self.repo.find_by_id(id).await?;
        match tenant {
            Some(t) if t.status == "active" => Ok(t.id),
            Some(_) => Err(AppError::BadRequest("tenant is not active".into())),
            None => Err(AppError::not_found(&format!("tenant/{id}"))),
        }
    }
}
