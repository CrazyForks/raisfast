//! 数据库连接池初始化。
//!
//! 根据 feature flag 创建对应数据库类型的连接池：
//! - `sqlite`：创建 `SqlitePool` 并设置 WAL 模式、外键约束
//! - `postgres`：创建 `PgPool`
//! - `mysql`：创建 `MySqlPool`
//!
//! 连接池配置包含 `max_connections`、`min_connections`、`acquire_timeout`、`idle_timeout`
//! 和 `max_lifetime`，确保在高并发下不会无限等待连接，同时避免连接泄漏。

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
