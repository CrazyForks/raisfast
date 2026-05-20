//! Cross-database SQL column type enum.
//!
//! Each variant maps to the appropriate native type for SQLite / MySQL / PostgreSQL.
//! Used by `field_type_to_sql()`, Aspect protocols, and migration code.

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
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            SqlType::Varchar => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "VARCHAR(255)"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "VARCHAR(255)"
                }
            }
            SqlType::Text => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "TEXT"
                }
            }
            SqlType::Integer => {
                #[cfg(feature = "db-sqlite")]
                {
                    "INTEGER"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "INTEGER"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "INT"
                }
            }
            SqlType::BigInt => {
                #[cfg(feature = "db-sqlite")]
                {
                    "INTEGER"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "BIGINT"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "BIGINT"
                }
            }
            SqlType::Real => {
                #[cfg(feature = "db-sqlite")]
                {
                    "REAL"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "DOUBLE PRECISION"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "DOUBLE"
                }
            }
            SqlType::Boolean => {
                #[cfg(feature = "db-sqlite")]
                {
                    "BOOLEAN"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "BOOLEAN"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "TINYINT(1)"
                }
            }
            SqlType::Blob => {
                #[cfg(feature = "db-sqlite")]
                {
                    "BLOB"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "BYTEA"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "BLOB"
                }
            }
            SqlType::Timestamp => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "TIMESTAMPTZ(0)"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "DATETIME"
                }
            }
            SqlType::Date => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "DATE"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "DATE"
                }
            }
            SqlType::Time => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "TIMETZ"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "TIME"
                }
            }
            SqlType::Decimal => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "NUMERIC(16,4)"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "DECIMAL(16,4)"
                }
            }
            SqlType::Json => {
                #[cfg(feature = "db-sqlite")]
                {
                    "TEXT"
                }
                #[cfg(feature = "db-postgres")]
                {
                    "JSONB"
                }
                #[cfg(feature = "db-mysql")]
                {
                    "JSON"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varchar_maps_correctly() {
        let s = SqlType::Varchar.as_str();
        assert!(!s.is_empty());
    }

    #[test]
    fn all_variants_have_mapping() {
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
