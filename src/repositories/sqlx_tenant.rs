//! sqlx-based `TenantRepository` implementation

use crate::errors::app_error::AppResult;
use crate::models::tenant::{Tenant, TenantStatus};
use raisfast_derive::repository;

#[repository(model = "tenant", struct_name = SqlxTenantRepository)]
pub trait TenantRepository: Send + Sync {
    async fn find_all(&self) -> AppResult<Vec<Tenant>>;

    async fn find_by_id(&self, id: &str) -> AppResult<Option<Tenant>>;

    async fn find_by_domain(&self, domain: &str) -> AppResult<Option<Tenant>>;

    async fn create(
        &self,
        id: &str,
        name: &str,
        domain: Option<&str>,
        config: &str,
    ) -> AppResult<Tenant>;

    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        domain: Option<&str>,
        config: Option<&str>,
        status: Option<TenantStatus>,
    ) -> AppResult<Tenant>;

    async fn delete(&self, id: &str) -> AppResult<()>;
}
