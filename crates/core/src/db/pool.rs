//! Database connection pool and transaction type aliases.
//!
//! Based on the compile-time selected feature (`db-sqlite`, `db-postgres`, `db-mysql`),
//! exports the corresponding connection pool type [`Pool`] and transaction type [`Transaction`].
//!
//! # Compile-time Validation
//!
//! Exactly one database feature must be enabled, otherwise compilation fails:
//! - No database feature enabled → compile error
//! - Multiple database features enabled → compile error
//!
//! # Adding a new database
//!
//! Add one `define_db_types!` invocation with the sqlx Database type.

macro_rules! define_db_types {
    ($db_type:path {
        Pool = $pool:ty,
        Transaction = $tx:ty,
        Row = $row:ty,
        Connection = $conn:ty,
        Arguments = $args:ty,
        QueryResult = $result:ty,
        PoolConnection = $pc:ty,
    }) => {
        pub type Db = $db_type;
        pub type Pool = $pool;
        pub type Transaction<'a> = $tx;
        pub type DbRow = $row;
        pub type DbConnection = $conn;
        pub type DbArguments<'q> = $args;
        pub type DbQueryResult = $result;
        pub type DbPoolConnection = $pc;
    };
}

#[cfg(all(
    feature = "db-sqlite",
    not(any(feature = "db-postgres", feature = "db-mysql"))
))]
define_db_types!(sqlx::Sqlite {
    Pool = sqlx::SqlitePool,
    Transaction = sqlx::Transaction<'a, sqlx::Sqlite>,
    Row = sqlx::sqlite::SqliteRow,
    Connection = sqlx::sqlite::SqliteConnection,
    Arguments = sqlx::sqlite::SqliteArguments<'q>,
    QueryResult = sqlx::sqlite::SqliteQueryResult,
    PoolConnection = sqlx::pool::PoolConnection<sqlx::Sqlite>,
});

#[cfg(all(
    feature = "db-postgres",
    not(any(feature = "db-sqlite", feature = "db-mysql"))
))]
define_db_types!(sqlx::Postgres {
    Pool = sqlx::PgPool,
    Transaction = sqlx::Transaction<'a, sqlx::Postgres>,
    Row = sqlx::postgres::PgRow,
    Connection = sqlx::postgres::PgConnection,
    Arguments = sqlx::postgres::PgArguments<'q>,
    QueryResult = sqlx::postgres::PgQueryResult,
    PoolConnection = sqlx::pool::PoolConnection<sqlx::Postgres>,
});

#[cfg(all(
    feature = "db-mysql",
    not(any(feature = "db-sqlite", feature = "db-postgres"))
))]
define_db_types!(sqlx::MySql {
    Pool = sqlx::MySqlPool,
    Transaction = sqlx::Transaction<'a, sqlx::MySql>,
    Row = sqlx::mysql::MySqlRow,
    Connection = sqlx::mysql::MySqlConnection,
        Arguments = sqlx::mysql::MySqlArguments,
    QueryResult = sqlx::mysql::MySqlQueryResult,
    PoolConnection = sqlx::pool::PoolConnection<sqlx::MySql>,
});

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
