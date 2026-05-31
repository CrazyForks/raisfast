//! `db` subcommand: database migration, backup.

use raisfast::config::app::AppConfig;
use raisfast::db::connection::init_pool;

// ── migrate ──────────────────────────────────────────────────────

/// `db migrate` — execute incremental schema changes.
///
/// Uses the `_migrations` table to track executed filenames, idempotent and safe.
/// All applied migrations in one call share the same batch number.
pub async fn migrate(config: &AppConfig) -> anyhow::Result<()> {
    println!("running migrations...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::ensure_schema(&pool).await?;
    raisfast::db::connection::run_pending_migrations(&pool).await?;

    Ok(())
}

// ── rollback ─────────────────────────────────────────────────────

/// `db rollback` — rollback the last batch (or `--step=N` individual migrations).
///
/// For each migration, looks for a corresponding `.down.sql` file.
/// The schema baseline (batch 0) is never rolled back.
pub async fn rollback(config: &AppConfig, step: &Option<u32>) -> anyhow::Result<()> {
    let step_desc = match step {
        Some(n) => format!("step={n}"),
        None => "last batch".to_string(),
    };
    println!("rolling back ({step_desc})...");
    let pool = init_pool(&config.database_url, 1).await?;

    raisfast::db::connection::rollback_migrations(&pool, *step).await?;

    Ok(())
}

// ── backup ───────────────────────────────────────────────────────

/// `db backup` — backup the database.
///
/// Delegates to `raisfast::db::backup::backup_database` which flushes WAL and copies the file.
pub async fn backup(config: &AppConfig, output_dir: &str, retention: usize) -> anyhow::Result<()> {
    raisfast::db::backup::backup_database(config, output_dir, retention).await
}
