//! BIGINT bind coercion.
//!
//! The `crud_insert!` derive macro wraps values bound to `BIGINT` columns with
//! [`DbBigint::to_bigint`]. This converts `SnowflakeId`, `i32` (and `Option`
//! wrappers) into the plain `i64` / `Option<i64>` that sqlx's `query!` macro
//! requires for `BIGINT` columns.
//!
//! On PostgreSQL (Strong param checking) this is required — `SnowflakeId` and
//! `i32` are not directly compatible with `INT8`. On SQLite and MySQL (Weak
//! checking) the coercion is a harmless no-op for `i64` and widens `i32` to
//! `i64` which is always safe.

/// Coerce a value bound to a `BIGINT` column.
pub trait DbBigint {
    /// The BIGINT-compatible type the value is coerced to.
    type Output;

    /// Convert into the BIGINT-compatible type.
    fn to_bigint(self) -> Self::Output;
}

impl DbBigint for i64 {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        self
    }
}

impl DbBigint for &i64 {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        *self
    }
}

impl DbBigint for i32 {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        i64::from(self)
    }
}

impl DbBigint for &i32 {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        i64::from(*self)
    }
}

impl DbBigint for crate::types::snowflake_id::SnowflakeId {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        i64::from(self)
    }
}

impl DbBigint for &crate::types::snowflake_id::SnowflakeId {
    type Output = i64;
    fn to_bigint(self) -> Self::Output {
        i64::from(*self)
    }
}

impl DbBigint for Option<i64> {
    type Output = Option<i64>;
    fn to_bigint(self) -> Self::Output {
        self
    }
}

impl DbBigint for Option<&i64> {
    type Output = Option<i64>;
    fn to_bigint(self) -> Self::Output {
        self.copied()
    }
}

impl DbBigint for Option<i32> {
    type Output = Option<i64>;
    fn to_bigint(self) -> Self::Output {
        self.map(i64::from)
    }
}

impl DbBigint for Option<crate::types::snowflake_id::SnowflakeId> {
    type Output = Option<i64>;
    fn to_bigint(self) -> Self::Output {
        self.map(i64::from)
    }
}
