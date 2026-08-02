//! Database backup Handler
//!
//! Performs a consistent SQLite backup by flushing the WAL and copying the database file.

use std::sync::Arc;

use crate::config::app::AppConfig;
use crate::errors::app_error::AppResult;
use crate::worker::{Job, JobHandler};

/// Database backup handler
pub struct DbBackupHandler {
    config: Arc<AppConfig>,
}

impl DbBackupHandler {
    /// Creates a new database backup handler
    #[must_use]
    pub fn new(config: Arc<AppConfig>) -> Self {
        Self { config }
    }
}

#[async_trait::async_trait]
impl JobHandler for DbBackupHandler {
    async fn handle(&self, job: &Job) -> AppResult<()> {
        match job {
            Job::Custom { job_type, .. } if job_type == "db_backup" => {}
            _ => return Ok(()),
        }

        let output_dir = format!("{}/backups", self.config.storage_root_dir);

        crate::db::backup::backup_database(&self.config, &output_dir, self.config.backup_retention)
            .await
            .map_err(crate::errors::app_error::AppError::Internal)?;

        tracing::info!("[db_backup] backup completed to {}", output_dir);
        Ok(())
    }
}
