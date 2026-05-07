//! 数据库连接池初始化。
//!
//! 根据 feature flag 创建对应数据库类型的连接池：
//! - `sqlite`：创建 `SqlitePool` 并设置 WAL 模式、外键约束
//! - `postgres`：创建 `PgPool`
//! - `mysql`：创建 `MySqlPool`
//!
//! 连接池配置包含 `max_connections`、`min_connections`、`acquire_timeout`、`idle_timeout`
//! 和 `max_lifetime`，确保在高并发下不会无限等待连接，同时避免连接泄漏。
//!
//! 首次启动时自动执行 `SCHEMA_SQL` 建表 + 预置数据（幂等）。

use std::time::Duration;

use crate::db::pool::Pool;

const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_LIFETIME: Duration = Duration::from_secs(1800);

/// 初始化数据库连接池。
///
/// `SQLite` 额外执行 PRAGMA 配置（WAL 模式 + 外键约束），
/// `PostgreSQL` / `MySQL` 无需额外设置。
pub async fn init_pool(database_url: &str, pool_size: u32) -> Result<Pool, sqlx::Error> {
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

/// 首次启动时自动执行 schema 建表 + 预置数据。
///
/// 检测 `_migrations` 表是否存在来判断是否首次启动。
/// 所有 SQL 使用 `IF NOT EXISTS` / `OR IGNORE`，天然幂等。
/// 后续结构变更通过 `db migrate` 执行增量迁移文件。
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

    sqlx::query(&crate::db::dialect::translate(
        "CREATE TABLE IF NOT EXISTS _migrations (filename TEXT PRIMARY KEY)",
    ))
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("create _migrations table failed: {e}"))?;

    sqlx::query(&crate::db::dialect::translate(
        "INSERT INTO _migrations (filename) VALUES (?)",
    ))
    .bind(schema_label)
    .execute(pool)
    .await
    .map_err(|e| anyhow::anyhow!("record schema migration failed: {e}"))?;

    tracing::info!("schema initialized successfully");
    Ok(())
}

/// 检测 `_migrations` 表是否存在（按数据库类型分分支）。
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
