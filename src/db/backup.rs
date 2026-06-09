//! Database backup utilities.
//!
//! - **SQLite**: Flushes WAL, then copies the database file.
//! - **PostgreSQL**: Calls `pg_dump` with the connection URL.
//! - **MySQL**: Calls `mysqldump` with credentials extracted from the URL.

use std::path::Path;

use crate::config::app::AppConfig;
#[cfg(feature = "db-sqlite")]
use crate::db::connection::init_pool;

/// Backup the database.
///
/// For SQLite: flushes WAL via `PRAGMA wal_checkpoint(TRUNCATE)`, then copies the file.
/// For PostgreSQL: calls `pg_dump` with the full connection URL.
/// For MySQL: calls `mysqldump` with credentials from the URL.
/// Retains only the latest `retention` backups in `output_dir`.
pub async fn backup_database(
    config: &AppConfig,
    output_dir: &str,
    retention: usize,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");

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

        let backup_name = format!("raisfast_{}.db", timestamp);
        let backup_path = Path::new(output_dir).join(&backup_name);

        std::fs::copy(db_path, &backup_path)?;
        let now = std::time::SystemTime::now();
        let _ = std::fs::File::open(&backup_path).and_then(|f| f.set_modified(now));
        let size = std::fs::metadata(&backup_path)?.len();

        tracing::info!("backed up to {} ({} bytes)", backup_path.display(), size);
    }

    #[cfg(feature = "db-postgres")]
    {
        let backup_name = format!("raisfast_{}.sql", timestamp);
        let backup_path = Path::new(output_dir).join(&backup_name);

        let status = std::process::Command::new("pg_dump")
            .args([
                "--no-password",
                "--dbname",
                &config.database_url,
                "--file",
                backup_path.to_str().unwrap_or(""),
            ])
            .status()
            .map_err(|e| anyhow::anyhow!("pg_dump not found: {e}"))?;

        if !status.success() {
            anyhow::bail!("pg_dump failed with exit code {:?}", status.code());
        }

        let size = std::fs::metadata(&backup_path)?.len();
        tracing::info!("backed up to {} ({} bytes)", backup_path.display(), size);
    }

    #[cfg(feature = "db-mysql")]
    {
        let backup_name = format!("raisfast_{}.sql", timestamp);
        let backup_path = Path::new(output_dir).join(&backup_name);

        let url = &config.database_url;
        let ConnParts {
            user,
            password,
            host,
            port,
            dbname,
        } = parse_mysql_url(url)?;

        let mut cmd = std::process::Command::new("mysqldump");
        cmd.args([
            "-h",
            &host,
            "-P",
            &port,
            "-u",
            &user,
            "--single-transaction",
            "--routines",
        ])
        .stdout(std::process::Stdio::piped());

        if !password.is_empty() {
            cmd.env("MYSQL_PWD", &password);
        }

        let output = cmd
            .arg(&dbname)
            .output()
            .map_err(|e| anyhow::anyhow!("mysqldump not found: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("mysqldump failed: {stderr}");
        }

        std::fs::write(&backup_path, &output.stdout)?;
        let size = std::fs::metadata(&backup_path)?.len();
        tracing::info!("backed up to {} ({} bytes)", backup_path.display(), size);
    }

    cleanup_old_backups(output_dir, retention);
    Ok(())
}

#[cfg(feature = "db-mysql")]
struct ConnParts {
    user: String,
    password: String,
    host: String,
    port: String,
    dbname: String,
}

/// Parse `mysql://user:pass@host:port/dbname?params` into components.
#[cfg(feature = "db-mysql")]
fn parse_mysql_url(url: &str) -> anyhow::Result<ConnParts> {
    let stripped = url
        .trim_start_matches("mysql://")
        .trim_start_matches("mariadb://");

    let (user_pass, rest) = stripped
        .split_once('@')
        .ok_or_else(|| anyhow::anyhow!("invalid MySQL URL: missing '@'"))?;
    let (user, password) = user_pass.split_once(':').unwrap_or((user_pass, ""));

    let (host_port_db, _) = rest.split_once('?').unwrap_or((rest, ""));
    let host_port_db = host_port_db.trim_end_matches('/');

    let (host_port, dbname) = host_port_db
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("invalid MySQL URL: missing database name"))?;

    let (host, port) = host_port.rsplit_once(':').unwrap_or((host_port, "3306"));

    Ok(ConnParts {
        user: user.to_string(),
        password: password.to_string(),
        host: host.to_string(),
        port: port.to_string(),
        dbname: dbname.to_string(),
    })
}

/// Clean up old backups, keeping only the latest `retention` count.
fn cleanup_old_backups(output_dir: &str, retention: usize) {
    let mut backups: Vec<_> = std::fs::read_dir(output_dir)
        .ok()
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|ext| ext == "db" || ext == "sql")
                })
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
