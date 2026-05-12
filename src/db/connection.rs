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
    Err(last_err.unwrap())
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
///
/// Checks for the existence of the `_migrations` table to determine if this is the first run.
/// All SQL uses `IF NOT EXISTS` / `OR IGNORE`, making it naturally idempotent.
/// Subsequent structural changes are applied via incremental migration files through `db migrate`.
pub async fn ensure_schema(pool: &Pool) -> anyhow::Result<()> {
    let has_migrations = check_migrations_table(pool).await;

    if has_migrations {
        tracing::debug!("schema already initialized");
        return Ok(());
    }

    tracing::info!("first run — executing schema...");
    sqlx::query(crate::db::schema::SCHEMA_SQL)
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("schema execution failed: {e}"))?;

    let schema_label = if cfg!(feature = "db-sqlite") {
        "schema.sqlite.sql"
    } else if cfg!(feature = "db-postgres") {
        "schema.postgres.sql"
    } else {
        "schema.mysql.sql"
    };

    sqlx::query("CREATE TABLE IF NOT EXISTS _migrations (filename TEXT PRIMARY KEY)")
        .execute(pool)
        .await
        .map_err(|e| anyhow::anyhow!("create _migrations table failed: {e}"))?;

    sqlx::query(&format!(
        "INSERT INTO _migrations (filename) VALUES ({})",
        crate::db::dialect::ph(1)
    ))
    .bind(schema_label)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("record schema migration failed: {e}"))?;

    tracing::info!("schema initialized successfully");
    Ok(())
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
