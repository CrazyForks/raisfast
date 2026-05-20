//! tenantable Protocol — multi-tenant isolation
//!
//! Provides a `tenant_id` column; automatically injects the current tenant ID and filters by tenant on list queries.

use std::sync::Arc;

use async_trait::async_trait;

use crate::aspects::{Aspect, ColumnDef, Pointcut, SqlType};
use crate::constants::COL_TENANT_ID;
use crate::protocols::Protocol;

pub struct TenantableAspect;

#[async_trait]
impl Aspect for TenantableAspect {
    fn name(&self) -> &str {
        "tenantable"
    }

    fn priority(&self) -> i32 {
        -600
    }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![]
    }

    fn columns(&self) -> Vec<ColumnDef> {
        vec![ColumnDef {
            name: COL_TENANT_ID.into(),
            sql_type: SqlType::Varchar,
            default: Some(format!("'{}'", crate::constants::DEFAULT_TENANT)),
        }]
    }
}

pub struct TenantableProtocol;

impl Protocol for TenantableProtocol {
    fn name(&self) -> &str {
        "tenantable"
    }

    fn description(&self) -> &str {
        "Multi-tenant isolation; automatically filters data by tenant ID"
    }

    fn aspects(&self) -> Vec<Arc<dyn Aspect>> {
        vec![Arc::new(TenantableAspect)]
    }

    fn behaviors(&self) -> Vec<&'static str> {
        vec!["tenantable"]
    }

    fn built_in(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provides_tenant_id_column() {
        let cols = TenantableAspect.columns();
        assert_eq!(cols.len(), 1);
        assert_eq!(cols[0].name, COL_TENANT_ID);
        assert_eq!(cols[0].sql_type, SqlType::Varchar);
        assert!(cols[0].default.is_some());
    }

    #[test]
    fn protocol_declares_tenantable_behavior() {
        let p = TenantableProtocol;
        assert_eq!(p.name(), "tenantable");
        assert!(p.behaviors().contains(&"tenantable"));
        assert!(p.built_in());
    }
}

crate::register_protocol!(
    crate::protocols::tenantable::TenantableProtocol,
    crate::protocols::tenantable::TenantableProtocol
);
