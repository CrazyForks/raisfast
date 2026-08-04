//! PostgreSQL BIGINT bind coercion.
//!
//! The `crud_insert!` derive macro wraps values bound to `BIGINT` columns with
//! [`PgBigInt::pg_bigint`]. This converts `SnowflakeId` (and `Option` wrappers)
//! into the plain `i64` / `Option<i64>` that sqlx's Postgres `query!` macro
//! requires for `BIGINT` columns. SQLite and MySQL leave binds untouched.

/// Coerce a value bound to a PostgreSQL `BIGINT` column.
pub trait PgBigInt {
    /// The PostgreSQL-compatible type the value is coerced to.
    type Output;

    /// Convert into the `BIGINT`-compatible type.
    fn pg_bigint(self) -> Self::Output;
}

impl PgBigInt for i64 {
    type Output = i64;
    fn pg_bigint(self) -> Self::Output {
        self
    }
}

impl PgBigInt for i32 {
    type Output = i64;
    fn pg_bigint(self) -> Self::Output {
        i64::from(self)
    }
}

impl PgBigInt for crate::types::snowflake_id::SnowflakeId {
    type Output = i64;
    fn pg_bigint(self) -> Self::Output {
        i64::from(self)
    }
}

impl PgBigInt for Option<i64> {
    type Output = Option<i64>;
    fn pg_bigint(self) -> Self::Output {
        self
    }
}

impl PgBigInt for Option<i32> {
    type Output = Option<i64>;
    fn pg_bigint(self) -> Self::Output {
        self.map(i64::from)
    }
}

impl PgBigInt for Option<crate::types::snowflake_id::SnowflakeId> {
    type Output = Option<i64>;
    fn pg_bigint(self) -> Self::Output {
        self.map(i64::from)
    }
}
