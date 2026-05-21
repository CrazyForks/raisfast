//! Database connection pool initialization.
//!
//! Creates the appropriate connection pool based on the feature flag:
//! - `sqlite`: Creates `SqlitePool` with WAL mode and foreign key constraints
//! - `postgres`: Creates `PgPool`
//! - `mysql`: Creates `MySqlPool`
//!
//! Pool configuration includes `max_connections`, `min_connections`, `acquire_timeout`, `idle_timeout`,
//! and `max_lifetime`, ensuring no infinite waits under high concurrency while avoiding connection leaks.
//!
//! On first startup, automatically executes `SCHEMA_SQL` to create tables + seed data (idempotent).
//! On every startup, automatically runs pending incremental migrations from `migrations/{db}/`.

use crate::db::DbDriver;
use std::time::Duration;

use crate::db::pool::Pool;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_LIFETIME: Duration = Duration::from_secs(1800);
const MAX_CONNECT_RETRIES: u32 = 5;
const CONNECT_RETRY_BASE_DELAY: Duration = Duration::from_secs(1);

/// Initialize the database connection pool (with exponential backoff retry).
///
/// In container environments, the database may not be ready yet.
/// Retries up to `MAX_CONNECT_RETRIES` times, waiting `2^attempt * base_delay` each time.
pub async fn init_pool(database_url: &str, pool_size: u32) -> Result<Pool, sqlx::Error> {
    let mut last_err = None;
    for attempt in 0..=MAX_CONNECT_RETRIES {
        match try_connect(database_url, pool_size).await {
            Ok(pool) => return Ok(pool),
            Err(e) => {
                if attempt < MAX_CONNECT_RETRIES {
                    let delay = CONNECT_RETRY_BASE_DELAY * 2u32.pow(attempt);
                    let delay_secs = delay.as_secs();
                    tracing::warn!(attempt, delay_secs, error = %e, "database connection failed, retrying...");
                    tokio::time::sleep(delay).await;
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        sqlx::Error::Configuration("database connection failed with no error recorded".into())
    }))
}

async fn try_connect(database_url: &str, pool_size: u32) -> Result<Pool, sqlx::Error> {
    #[cfg(feature = "db-sqlite")]
    {
        use sqlx::pool::PoolOptions;
        let pool = PoolOptions::<sqlx::Sqlite>::new()
            .max_connections(pool_size)
            .min_connections(1)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .idle_timeout(Some(IDLE_TIMEOUT))
            .max_lifetime(Some(MAX_LIFETIME))
            .after_connect(|conn, _meta| {
                Box::pin(async {
                    sqlx::query("PRAGMA journal_mode = WAL")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA foreign_keys = ON")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout = 5000")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA synchronous = NORMAL")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA cache_size = -64000")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA temp_store = MEMORY")
                        .execute(&mut *conn)
                        .await?;
                    sqlx::query("PRAGMA mmap_size = 268435456")
                        .execute(&mut *conn)
                        .await?;
                    Ok(())
                })
            })
            .connect(database_url)
            .await?;

        tracing::info!(%pool_size, "sqlite connection pool initialized");
        Ok(pool)
    }

    #[cfg(feature = "db-postgres")]
    {
        use sqlx::pool::PoolOptions;
        let pool = PoolOptions::<sqlx::Postgres>::new()
            .max_connections(pool_size)
            .min_connections(1)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .idle_timeout(Some(IDLE_TIMEOUT))
            .max_lifetime(Some(MAX_LIFETIME))
            .connect(database_url)
            .await?;

        tracing::info!(%pool_size, "postgres connection pool initialized");
        Ok(pool)
    }

    #[cfg(feature = "db-mysql")]
    {
        use sqlx::pool::PoolOptions;
        let pool = PoolOptions::<sqlx::MySql>::new()
            .max_connections(pool_size)
            .min_connections(1)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .idle_timeout(Some(IDLE_TIMEOUT))
            .max_lifetime(Some(MAX_LIFETIME))
            .connect(database_url)
            .await?;

        tracing::info!(%pool_size, "mysql connection pool initialized");
        Ok(pool)
    }
}

/// On first startup, automatically execute schema to create tables + seed data.
/// On every startup, run any pending incremental migrations.
///
/// Checks for the existence of the `_migrations` table to determine if this is the first run.
/// All SQL uses `IF NOT EXISTS` / `OR IGNORE`, making it naturally idempotent.
pub async fn ensure_schema(pool: &Pool) -> anyhow::Result<()> {
    let has_migrations = check_migrations_table(pool).await;

    if !has_migrations {
        tracing::info!("first run — executing schema...");
        sqlx::query(crate::db::schema::SCHEMA_SQL)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("schema execution failed: {e}"))?;

        let schema_label = db_label();
        let checksum = sha256_hex(crate::db::schema::SCHEMA_SQL);

        {
            #[cfg(feature = "db-sqlite")]
            let sql = "CREATE TABLE IF NOT EXISTS _migrations (\
                 filename TEXT PRIMARY KEY, \
                 checksum TEXT NOT NULL, \
                 applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))\
                 )";
            #[cfg(feature = "db-postgres")]
            let sql = "CREATE TABLE IF NOT EXISTS _migrations (\
                 filename TEXT PRIMARY KEY, \
                 checksum TEXT NOT NULL, \
                 applied_at TEXT NOT NULL DEFAULT NOW()\
                 )";
            #[cfg(feature = "db-mysql")]
            let sql = "CREATE TABLE IF NOT EXISTS _migrations (\
                 filename TEXT PRIMARY KEY, \
                 checksum TEXT NOT NULL, \
                 applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP\
                 )";
            sqlx::query(sql)
                .execute(pool)
                .await
                .map_err(|e| anyhow::anyhow!("create _migrations table failed: {e}"))?;
        }

        let ph = crate::db::Driver::ph;
        sqlx::query(&format!(
            "INSERT INTO _migrations (filename, checksum) VALUES ({}, {})",
            ph(1),
            ph(2)
        ))
        .bind(format!("schema.{schema_label}.sql"))
        .bind(&checksum)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("record schema migration failed: {e}"))?;

        tracing::info!("schema initialized successfully");
    } else {
        ensure_migrations_table_schema(pool).await;
    }

    run_pending_migrations(pool).await?;

    Ok(())
}

/// Auto-run any pending incremental migration files from `migrations/{db}/`.
///
/// Called automatically during startup after `ensure_schema`.
/// Migrations are sorted alphabetically and tracked in `_migrations`.
/// Files already recorded are skipped. Checksums are verified on re-run detection.
async fn run_pending_migrations(pool: &Pool) -> anyhow::Result<()> {
    let db_name = match db_label() {
        "sqlite" => "sqlite",
        "postgres" => "postgres",
        "mysql" => "mysql",
        other => {
            tracing::debug!("unknown db label '{other}', skipping auto-migration");
            return Ok(());
        }
    };

    let migrations_dir = std::path::Path::new("./migrations").join(db_name);
    if !migrations_dir.exists() {
        tracing::debug!("no migrations directory found, skipping auto-migration");
        return Ok(());
    }

    let schema_label = format!("schema.{db_name}.sql");

    let mut entries: Vec<_> = std::fs::read_dir(&migrations_dir)
        .ok()
        .map(|dir| {
            dir.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().extension().is_some_and(|ext| ext == "sql")
                        && e.file_name().to_string_lossy() != schema_label
                })
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());

    if entries.is_empty() {
        return Ok(());
    }

    let ph = crate::db::Driver::ph;
    let check_sql = format!(
        "SELECT checksum FROM _migrations WHERE filename = {}",
        ph(1)
    );
    let insert_sql = format!(
        "INSERT INTO _migrations (filename, checksum) VALUES ({}, {})",
        ph(1),
        ph(2)
    );

    let mut applied = 0u32;

    for entry in &entries {
        let filename = entry.file_name().to_string_lossy().to_string();

        let existing: Option<(String,)> = sqlx::query_as(&check_sql)
            .bind(&filename)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();

        if let Some((recorded_checksum,)) = existing {
            let sql = std::fs::read_to_string(entry.path()).unwrap_or_default();
            let current_checksum = sha256_hex(&sql);
            if recorded_checksum != current_checksum {
                tracing::warn!(
                    filename = %filename,
                    "migration file checksum mismatch — file was modified after being applied, skipping"
                );
            }
            continue;
        }

        let sql = std::fs::read_to_string(entry.path())?;
        let checksum = sha256_hex(&sql);

        tracing::info!(filename = %filename, "applying migration...");
        sqlx::query(&sql)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("migration {filename} failed: {e}"))?;

        sqlx::query(&insert_sql)
            .bind(&filename)
            .bind(&checksum)
            .execute(pool)
            .await
            .map_err(|e| anyhow::anyhow!("record migration {filename} failed: {e}"))?;

        tracing::info!(filename = %filename, "migration applied");
        applied += 1;
    }

    if applied > 0 {
        tracing::info!("applied {applied} migration(s)");
    }

    Ok(())
}

/// Ensure the `_migrations` table has the `checksum` and `applied_at` columns.
/// Handles upgrades from the old schema (filename-only).
async fn ensure_migrations_table_schema(pool: &Pool) {
    let _ = sqlx::query("ALTER TABLE _migrations ADD COLUMN checksum TEXT NOT NULL DEFAULT ''")
        .execute(pool)
        .await;
    {
        #[cfg(feature = "db-sqlite")]
        let sql = "ALTER TABLE _migrations ADD COLUMN applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))";
        #[cfg(feature = "db-postgres")]
        let sql = "ALTER TABLE _migrations ADD COLUMN applied_at TEXT NOT NULL DEFAULT NOW()";
        #[cfg(feature = "db-mysql")]
        let sql =
            "ALTER TABLE _migrations ADD COLUMN applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP";
        let _ = sqlx::query(sql).execute(pool).await;
    }
}

/// Check if the `_migrations` table exists (branched by database type).
async fn check_migrations_table(pool: &Pool) -> bool {
    #[cfg(feature = "db-sqlite")]
    {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_migrations'",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(0)
            > 0
    }

    #[cfg(feature = "db-postgres")]
    {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = '_migrations'",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(0)
            > 0
    }

    #[cfg(feature = "db-mysql")]
    {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = '_migrations'",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(0)
            > 0
    }
}

/// Return the database label string based on active feature flag.
fn db_label() -> &'static str {
    if cfg!(feature = "db-sqlite") {
        "sqlite"
    } else if cfg!(feature = "db-postgres") {
        "postgres"
    } else if cfg!(feature = "db-mysql") {
        "mysql"
    } else {
        "unknown"
    }
}

/// Compute SHA-256 hex digest of the input string.
fn sha256_hex(input: &str) -> String {
    use std::fmt::Write;
    let hash = <sha2::Sha256 as sha2::Digest>::digest(input.as_bytes());
    let mut hex = String::with_capacity(hash.len() * 2);
    for byte in hash {
        write!(hex, "{byte:02x}").unwrap();
    }
    hex
}
