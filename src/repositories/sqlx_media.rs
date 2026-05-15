//! sqlx-based `MediaRepository` implementation

use crate::commands::CreateMediaCmd;
use crate::errors::app_error::AppResult;
use crate::models::media::{Media, MediaStats};
use raisfast_derive::repository;

/// Media file Repository interface
#[repository(model = "media", struct_name = SqlxMediaRepository)]
pub trait MediaRepository: Send + Sync {
    /// Create a media file record
    async fn create(&self, cmd: &CreateMediaCmd, tenant_id: Option<&str>) -> AppResult<Media>;

    /// Find media files for a given user with pagination
    async fn find_all(
        &self,
        user_id: i64,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)>;

    /// Find all users' media files with pagination (admin)
    async fn find_all_admin(
        &self,
        page: i64,
        page_size: i64,
        tenant_id: Option<&str>,
    ) -> AppResult<(Vec<Media>, i64)>;

    /// Find a media file by ID
    async fn find_by_id(&self, id: i64, tenant_id: Option<&str>) -> AppResult<Option<Media>>;

    /// Delete a media file record
    async fn delete(&self, id: i64, tenant_id: Option<&str>) -> AppResult<()>;

    /// Get storage statistics
    async fn stats(&self, user_id: i64, tenant_id: Option<&str>) -> AppResult<MediaStats>;
}
