//! Database module.
//!
//! Provides multi-database support (SQLite, PostgreSQL, MySQL),
//! with the backend selected at compile time via feature flags.

pub mod backup;
pub mod bigint;
pub mod connection;
pub mod driver;
pub mod json;
pub mod pool;
pub mod schema;
pub mod sql_type;
pub mod tenant;

pub mod schema_meta {
    include!(concat!(env!("OUT_DIR"), "/schema_meta.rs"));
}

pub mod prelude {
    pub use super::driver::DbDriver;
    pub use super::driver::Driver;
    pub use super::driver::is_safe_identifier;
    pub use super::driver::sanitize_identifier;
    pub use super::pool::{
        Db, DbArguments, DbConnection, DbPoolConnection, DbQueryResult, DbRow, Pool, Transaction,
    };
}

pub use driver::DbDriver;
pub use driver::Driver;
pub use pool::{
    Db, DbArguments, DbConnection, DbPoolConnection, DbQueryResult, DbRow, Pool, Transaction,
};

/// Wrap a dynamic SQL string with [`sqlx::AssertSqlSafe`].
///
/// Asserts that the SQL string has been manually audited for injection
/// vulnerabilities. Required by sqlx 0.9+ which gates `query()` on the
/// `SqlSafeStr` trait.
#[inline(always)]
pub fn safe_sql(sql: &str) -> sqlx::AssertSqlSafe<&str> {
    sqlx::AssertSqlSafe(sql)
}
