//! Database schema constants.
//!
//! Embedded at compile time from `migrations/{db}/schema.{db}.sql`, used for testing and initialization.
//! The appropriate schema file is selected based on the compile-time feature flag.

#[cfg(feature = "db-sqlite")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/sqlite/schema.sqlite.sql");

#[cfg(feature = "db-postgres")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/postgres/schema.postgres.sql");

#[cfg(feature = "db-mysql")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/mysql/schema.mysql.sql");
