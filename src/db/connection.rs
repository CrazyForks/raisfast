//! SQLite 连接池初始化。
//!
//! 创建异步连接池并设置关键 PRAGMA：
//! - `journal_mode = WAL` — Write-Ahead Logging，提升并发读写性能
//! - `foreign_keys = ON` — 启用外键约束（SQLite 默认关闭）

use sqlx::SqlitePool;
use sqlx::pool::PoolOptions;

/// 初始化 SQLite 连接池。
///
/// # 参数
///
/// - `database_url` — SQLite 连接字符串，如 `sqlite:./data/blog.db?mode=rwc`
/// - `pool_size` — 最大连接数
///
/// # 返回
///
/// 配置好 PRAGMA 的 `SqlitePool` 实例。
pub async fn init_pool(database_url: &str, pool_size: u32) -> Result<SqlitePool, sqlx::Error> {
    let pool = PoolOptions::<sqlx::Sqlite>::new()
        .max_connections(pool_size)
        .connect(database_url)
        .await?;

    // WAL 模式：读写不互相阻塞，适合 Web 场景
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(&pool)
        .await?;
    // 启用外键约束：确保 posts_tags、comments 等表的级联删除正常工作
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await?;

    tracing::info!("sqlite connection pool initialized");
    Ok(pool)
}
