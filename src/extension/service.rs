//! Extension 服务层

use crate::db::Pool;
use crate::errors::app_error::AppResult;
use crate::extension::model;

/// Extension 服务
pub struct ExtensionService {
    pool: Pool,
}

impl ExtensionService {
    /// 创建 Extension 服务实例
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// 查询所有已安装 Extension 记录
    pub async fn list_installed(&self) -> AppResult<Vec<model::ExtensionRecord>> {
        model::find_all(&self.pool).await
    }

    /// 按 ID 查询
    pub async fn get(&self, id: &str) -> AppResult<Option<model::ExtensionRecord>> {
        model::find_by_id(&self.pool, id).await
    }
}
