//! 数据库连接池与事务类型别名。
//!
//! 根据编译时选定的 feature（`db-sqlite`、`db-postgres`、`db-mysql`），
//! 导出对应的连接池类型 [`Pool`] 和事务类型 [`Transaction`]。
//!
//! # 编译时校验
//!
//! 必须恰好启用一个数据库 feature，否则编译失败：
//! - 未启用任何数据库 feature → 编译错误
//! - 同时启用多个数据库 feature → 编译错误

#[cfg(all(
    feature = "db-sqlite",
    not(any(feature = "db-postgres", feature = "db-mysql"))
))]
pub type Pool = sqlx::SqlitePool;

#[cfg(all(
    feature = "db-postgres",
    not(any(feature = "db-sqlite", feature = "db-mysql"))
))]
pub type Pool = sqlx::PgPool;

#[cfg(all(
    feature = "db-mysql",
    not(any(feature = "db-sqlite", feature = "db-postgres"))
))]
pub type Pool = sqlx::MySqlPool;

#[cfg(all(
    feature = "db-sqlite",
    not(any(feature = "db-postgres", feature = "db-mysql"))
))]
pub type Transaction<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

#[cfg(all(
    feature = "db-postgres",
    not(any(feature = "db-sqlite", feature = "db-mysql"))
))]
pub type Transaction<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

#[cfg(all(
    feature = "db-mysql",
    not(any(feature = "db-sqlite", feature = "db-postgres"))
))]
pub type Transaction<'a> = sqlx::Transaction<'a, sqlx::MySql>;

#[cfg(not(any(feature = "db-sqlite", feature = "db-postgres", feature = "db-mysql")))]
compile_error!(
    "no database backend selected. \
     Enable exactly one: --features db-sqlite, --features db-postgres, or --features db-mysql"
);

#[cfg(any(
    all(feature = "db-sqlite", feature = "db-postgres"),
    all(feature = "db-sqlite", feature = "db-mysql"),
    all(feature = "db-postgres", feature = "db-mysql"),
))]
compile_error!(
    "multiple database backends selected. \
     Enable exactly one: --features db-sqlite, --features db-postgres, or --features db-mysql"
);
