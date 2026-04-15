//! 数据库连接池初始化。
//!
//! 根据 feature flag 创建对应数据库类型的连接池：
//! - `sqlite`：创建 `SqlitePool` 并设置 WAL 模式、外键约束
//! - `postgres`：创建 `PgPool`
//! - `mysql`：创建 `MySqlPool`

use crate::db::pool::Pool;

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
            .connect(database_url)
            .await?;

        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await?;

        tracing::info!("sqlite connection pool initialized");
        Ok(pool)
    }

    #[cfg(feature = "db-postgres")]
    {
        use sqlx::pool::PoolOptions;
        let pool = PoolOptions::<sqlx::Postgres>::new()
            .max_connections(pool_size)
            .connect(database_url)
            .await?;

        tracing::info!("postgres connection pool initialized");
        Ok(pool)
    }

    #[cfg(feature = "db-mysql")]
    {
        use sqlx::pool::PoolOptions;
        let pool = PoolOptions::<sqlx::MySql>::new()
            .max_connections(pool_size)
            .connect(database_url)
            .await?;

        tracing::info!("mysql connection pool initialized");
        Ok(pool)
    }
}
