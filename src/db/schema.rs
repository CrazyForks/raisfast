//! 数据库 Schema 常量。
//!
//! 由 `migrations/{db}/schema.{db}.sql` 编译时嵌入，供测试和初始化使用。
//! 按编译 feature flag 选择对应数据库的 schema 文件。

#[cfg(feature = "db-sqlite")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/sqlite/schema.sqlite.sql");

#[cfg(feature = "db-postgres")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/postgres/schema.postgres.sql");

#[cfg(feature = "db-mysql")]
pub const SCHEMA_SQL: &str = include_str!("../../migrations/mysql/schema.mysql.sql");

#[cfg(feature = "db-sqlite")]
pub const TENANTABLE_SQL: &str = include_str!("../../migrations/sqlite/tenantable.sqlite.sql");

#[cfg(feature = "db-postgres")]
pub const TENANTABLE_SQL: &str = include_str!("../../migrations/postgres/tenantable.postgres.sql");

#[cfg(feature = "db-mysql")]
pub const TENANTABLE_SQL: &str = include_str!("../../migrations/mysql/tenantable.mysql.sql");
