//! 内置表 Aspect dispatch 辅助函数
//!
//! 提供统一的 before/after dispatch 调用，减少 Service 层重复代码。

use crate::aspects::engine::AspectEngine;
use crate::aspects::{
    BaseContext, DataAfterCreateContext, DataAfterDeleteContext, DataAfterUpdateContext,
    DataBeforeCreateContext, DataBeforeDeleteContext, DataBeforeUpdateContext, Record,
};
use crate::db::pool::Pool;
use crate::errors::app_error::AppResult;

pub struct AspectDispatch<'a> {
    pub engine: &'a AspectEngine,
    pub pool: &'a Pool,
    pub table: &'a str,
    pub user_id: Option<&'a str>,
    pub tenant_id: Option<&'a str>,
}

impl AspectDispatch<'_> {
    pub async fn before_create(&self, record: Record) -> AppResult<()> {
        let mut ctx = DataBeforeCreateContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            record,
            schema: None,
        };
        self.engine
            .dispatch_data_before_create(self.table, &mut ctx)
            .await
            .map_err(crate::errors::app_error::AppError::Internal)?;
        Ok(())
    }

    pub async fn after_create(&self, record: Record) {
        let mut ctx = DataAfterCreateContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            record,
            schema: None,
        };
        if let Err(e) = self
            .engine
            .dispatch_data_after_create(self.table, &mut ctx)
            .await
        {
            tracing::warn!("aspect after_create dispatch error for {}: {e}", self.table);
        }
    }

    pub async fn before_update(&self, old_record: Record, new_record: Record) -> AppResult<()> {
        let mut ctx = DataBeforeUpdateContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            old_record,
            new_record,
            schema: None,
        };
        self.engine
            .dispatch_data_before_update(self.table, &mut ctx)
            .await
            .map_err(crate::errors::app_error::AppError::Internal)?;
        Ok(())
    }

    pub async fn after_update(&self, new_record: Record) {
        let mut ctx = DataAfterUpdateContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            old_record: Record::new(),
            new_record,
            schema: None,
        };
        if let Err(e) = self
            .engine
            .dispatch_data_after_update(self.table, &mut ctx)
            .await
        {
            tracing::warn!("aspect after_update dispatch error for {}: {e}", self.table);
        }
    }

    pub async fn before_delete(&self, record: Record) -> AppResult<()> {
        let mut ctx = DataBeforeDeleteContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            record,
            soft_delete: false,
            schema: None,
        };
        self.engine
            .dispatch_data_before_delete(self.table, &mut ctx)
            .await
            .map_err(crate::errors::app_error::AppError::Internal)?;
        Ok(())
    }

    pub async fn after_delete(&self) {
        let mut ctx = DataAfterDeleteContext {
            base: self.make_base_ctx(),
            table: self.table.to_string(),
            record: Record::new(),
            schema: None,
        };
        if let Err(e) = self
            .engine
            .dispatch_data_after_delete(self.table, &mut ctx)
            .await
        {
            tracing::warn!("aspect after_delete dispatch error for {}: {e}", self.table);
        }
    }

    fn make_base_ctx(&self) -> BaseContext {
        BaseContext::new(
            self.user_id.map(|s| s.to_string()),
            self.tenant_id.unwrap_or("default").to_string(),
            crate::utils::tz::now_str(),
        )
        .with_pool(self.pool.clone())
    }
}

/// 创建一个 minimal Record 用于 dispatch（只含 id 和关键标识字段）
pub fn id_record(id: &str) -> Record {
    let mut r = Record::new();
    r.insert("id".into(), serde_json::json!(id));
    r
}
