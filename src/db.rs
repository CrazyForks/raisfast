//! Database module.
//!
//! Provides multi-database support (SQLite, PostgreSQL, MySQL),
//! with the backend selected at compile time via feature flags.

pub mod backup;
pub mod connection;
pub mod dialect;
pub mod pool;
pub mod schema;
pub mod tenant;

pub use pool::{
    DbArguments, DbConnection, DbPoolConnection, DbQueryResult, DbRow, Pool, Transaction,
};
