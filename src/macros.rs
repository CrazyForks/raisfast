#[macro_export]
macro_rules! impl_from_row_opt_tenant {
    ($t:ident { required { $($req:ident),* $(,)? } optional { $($opt:ident),* $(,)? } }) => {
        #[cfg(feature = "db-sqlite")]
        impl<'r> sqlx::FromRow<'r, sqlx::sqlite::SqliteRow> for $t {
            fn from_row(row: &'r sqlx::sqlite::SqliteRow) -> Result<Self, sqlx::Error> {
                use sqlx::Row;
                Ok(Self {
                    tenant_id: row.try_get($crate::constants::COL_TENANT_ID).ok(),
                    $($req: row.try_get(stringify!($req))?,)*
                    $($opt: row.try_get(stringify!($opt))?,)*
                })
            }
        }
        #[cfg(feature = "db-postgres")]
        impl<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> for $t {
            fn from_row(row: &'r sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
                use sqlx::Row;
                Ok(Self {
                    tenant_id: row.try_get($crate::constants::COL_TENANT_ID).ok(),
                    $($req: row.try_get(stringify!($req))?,)*
                    $($opt: row.try_get(stringify!($opt))?,)*
                })
            }
        }
        #[cfg(feature = "db-mysql")]
        impl<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> for $t {
            fn from_row(row: &'r sqlx::mysql::MySqlRow) -> Result<Self, sqlx::Error> {
                use sqlx::Row;
                Ok(Self {
                    tenant_id: row.try_get($crate::constants::COL_TENANT_ID).ok(),
                    $($req: row.try_get(stringify!($req))?,)*
                    $($opt: row.try_get(stringify!($opt))?,)*
                })
            }
        }
    };
}
