//! Cross-database SQL column type enum.
//!
//! Each variant maps to the appropriate native type for SQLite / MySQL / PostgreSQL.
//! Used by `field_type_to_sql()`, Aspect protocols, and migration code.
//!
//! Adding a new database: just add one column to the `TYPE_MAP` table below.

/// SQL column type with per-dialect mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlType {
    Varchar,
    Text,
    Integer,
    BigInt,
    Real,
    Boolean,
    Blob,
    Timestamp,
    Date,
    Time,
    Decimal,
    Json,
}

impl SqlType {
    /// Returns the native SQL type string for the current database dialect.
    ///
    /// Determined at compile time via feature flags — zero runtime branching.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        TYPE_MAP[self as usize]
    }
}

type Row = [&'static str; 12];

#[allow(clippy::too_many_arguments)]
const fn row(
    varchar: &'static str,
    text: &'static str,
    integer: &'static str,
    bigint: &'static str,
    real: &'static str,
    boolean: &'static str,
    blob: &'static str,
    timestamp: &'static str,
    date: &'static str,
    time: &'static str,
    decimal: &'static str,
    json: &'static str,
) -> Row {
    [
        varchar, text, integer, bigint, real, boolean, blob, timestamp, date, time, decimal, json,
    ]
}

#[cfg(feature = "db-sqlite")]
const TYPE_MAP: Row = row(
    "TEXT",    // Varchar
    "TEXT",    // Text
    "INTEGER", // Integer
    "INTEGER", // BigInt
    "REAL",    // Real
    "BOOLEAN", // Boolean
    "BLOB",    // Blob
    "TEXT",    // Timestamp
    "TEXT",    // Date
    "TEXT",    // Time
    "TEXT",    // Decimal
    "TEXT",    // Json
);

#[cfg(feature = "db-postgres")]
const TYPE_MAP: Row = row(
    "VARCHAR(255)",     // Varchar
    "TEXT",             // Text
    "INTEGER",          // Integer
    "BIGINT",           // BigInt
    "DOUBLE PRECISION", // Real
    "BOOLEAN",          // Boolean
    "BYTEA",            // Blob
    "TIMESTAMPTZ(0)",   // Timestamp
    "DATE",             // Date
    "TIMETZ",           // Time
    "NUMERIC(16,4)",    // Decimal
    "JSONB",            // Json
);

#[cfg(feature = "db-mysql")]
const TYPE_MAP: Row = row(
    "VARCHAR(255)",  // Varchar
    "TEXT",          // Text
    "INT",           // Integer
    "BIGINT",        // BigInt
    "DOUBLE",        // Real
    "TINYINT(1)",    // Boolean
    "BLOB",          // Blob
    "DATETIME",      // Timestamp
    "DATE",          // Date
    "TIME",          // Time
    "DECIMAL(16,4)", // Decimal
    "JSON",          // Json
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_have_non_empty_mapping() {
        let variants = [
            SqlType::Varchar,
            SqlType::Text,
            SqlType::Integer,
            SqlType::BigInt,
            SqlType::Real,
            SqlType::Boolean,
            SqlType::Blob,
            SqlType::Timestamp,
            SqlType::Date,
            SqlType::Time,
            SqlType::Decimal,
            SqlType::Json,
        ];
        for v in &variants {
            assert!(!v.as_str().is_empty(), "{v:?} returned empty string");
        }
    }
}
