//! 数据库模块。
//!
//! 提供多数据库支持（SQLite、PostgreSQL、MySQL），
//! 通过 feature flag 在编译时选择后端。

pub mod connection;
pub mod dialect;
pub mod pool;

pub use pool::Pool;
