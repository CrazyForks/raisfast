//! tenantable Protocol — 多租户隔离
//!
//! 提供 `tenant_id` 列，自动注入当前租户 ID，列表查询按租户过滤。

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
            sql_type: SqlType::Text,
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
        "多租户隔离，自动按租户 ID 过滤数据"
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
        assert_eq!(cols[0].sql_type, SqlType::Text);
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
