//! 基于 sqlx 的 `MediaRepository` 实现

use crate::commands::CreateMediaCmd;
use crate::errors::app_error::AppResult;
use crate::models::media::{self, Media, MediaStats};
use crate::repositories::define_sqlx_repo;

define_sqlx_repo!(SqlxMediaRepository);

/// 媒体文件 Repository 接口
#[async_trait::async_trait]
pub trait MediaRepository: Send + Sync {
    /// 创建媒体文件记录
    async fn create(&self, cmd: CreateMediaCmd, tenant_id: Option<&str>) -> AppResult<Media>;

    /// 分页查询指定用户的媒体文件
    async fn find_all(
        &self,
        user_id: i64,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)>;

    /// 根据媒体文件 ID 查找
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Media>>;

    /// 删除媒体文件记录
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;

    /// 获取存储统计
    async fn stats(&self, user_id: i64, tenant_id: Option<&str>) -> AppResult<MediaStats>;
}

#[async_trait::async_trait]
impl MediaRepository for SqlxMediaRepository {
    async fn create(&self, cmd: CreateMediaCmd, tenant_id: Option<&str>) -> AppResult<Media> {
        media::create(&self.pool, &cmd, tenant_id).await
    }

    async fn find_all(
        &self,
        user_id: i64,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)> {
        media::find_all(&self.pool, user_id, page, page_size, tenant_id).await
    }

    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Media>> {
        media::find_by_id(&self.pool, id, tenant_id).await
    }

    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()> {
        media::delete(&self.pool, id, tenant_id).await
    }

    async fn stats(&self, user_id: i64, tenant_id: Option<&str>) -> AppResult<media::MediaStats> {
        media::stats(&self.pool, user_id, tenant_id).await
    }
}
