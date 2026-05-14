//! Database backup utilities.
//!
//! Provides consistent SQLite backup by flushing the WAL before copying the database file.

use std::path::Path;

use crate::config::app::AppConfig;
use crate::db::connection::init_pool;

/// Backup the SQLite database.
///
/// Flushes the WAL via `PRAGMA wal_checkpoint(TRUNCATE)` for consistency,
/// then copies the database file to `output_dir` with a timestamp suffix.
/// Retains only the latest `retention` backups.
pub async fn backup_database(
    config: &AppConfig,
    output_dir: &str,
    retention: usize,
) -> anyhow::Result<()> {
    #[cfg(feature = "db-sqlite")]
    {
        let db_path = config
            .database_url
            .trim_start_matches("sqlite:")
            .split('?')
            .next()
            .ok_or_else(|| anyhow::anyhow!("invalid DATABASE_URL: {}", config.database_url))?;

        if !Path::new(db_path).exists() {
            anyhow::bail!("database file not found: {}", db_path);
        }

        let pool = init_pool(&config.database_url, 1).await?;
        if let Err(e) = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
        {
            tracing::warn!("WAL checkpoint failed, proceeding with file copy: {e}");
        }

        std::fs::create_dir_all(output_dir)?;

        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let backup_name = format!("raisfast_{}.db", timestamp);
        let backup_path = Path::new(output_dir).join(&backup_name);

        std::fs::copy(db_path, &backup_path)?;
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&backup_path).and_then(|f| f.set_modified(now));
        let size = std::fs::metadata(&backup_path)?.len();

        tracing::info!("backed up to {} ({} bytes)", backup_path.display(), size);

        cleanup_old_backups(output_dir, retention);
        Ok(())
    }

    #[cfg(not(feature = "db-sqlite"))]
    {
        let _ = (config, output_dir, retention);
        anyhow::bail!(
            "file-based backup is only supported for SQLite. \
             Use pg_dump (PostgreSQL) or mysqldump (MySQL) instead."
        );
    }
}

/// Clean up old backups, keeping only the latest `retention` count.
fn cleanup_old_backups(output_dir: &str, retention: usize) {
    let mut backups: Vec<_> = std::fs::read_dir(output_dir)
        .ok()
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "db"))
                .collect()
        })
        .unwrap_or_default();
    backups.sort_by_key(|e| e.metadata().ok().map(|m| m.modified().ok()));
    while backups.len() > retention {
        if let Some(old) = backups.first() {
            let _ = std::fs::remove_file(old.path());
            tracing::info!("removed old backup: {}", old.path().display());
        }
        backups.remove(0);
    }
}
