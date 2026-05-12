#[macro_export]
macro_rules! reg_route {
    ($router:expr, $registry:expr, $path:literal, $handler:expr, $source:expr, $name:expr, [$($method:literal),+ $(,)?]) => {{
        let r = $router.route($path, $handler);
        $($registry.record($method, concat!("/api/v1", $path), $source, $name);)+
        r
    }};
}

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

#[macro_export]
macro_rules! in_transaction {
    ($pool:expr, $tx:ident, $body:block) => {{
        let mut $tx = $pool.begin().await.map_err(|e| {
            $crate::errors::app_error::AppError::Internal(anyhow::anyhow!("begin tx: {e}"))
        })?;
        let __tx_result: Result<_, $crate::errors::app_error::AppError> = async { $body }.await;
        if __tx_result.is_ok() {
            $tx.commit().await.map_err(|e| {
                $crate::errors::app_error::AppError::Internal(anyhow::anyhow!("commit tx: {e}"))
            })?;
        }
        __tx_result
    }};
}

#[macro_export]
macro_rules! define_enum {
    (
        $(#[$meta:meta])*
        $name:ident { $($variant:ident = $value:literal),+ $(,)? }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[derive(serde::Serialize, serde::Deserialize)]
        #[derive(utoipa::ToSchema)]
        #[cfg_attr(feature = "export-types", derive(ts_rs::TS))]
        pub enum $name {
            $(
                #[serde(rename = $value)]
                $variant,
            )+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $value),+
                }
            }

            pub fn all_values() -> &'static [&'static str] {
                &[$($value),+]
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($value => Ok($name::$variant)),+,
                    _ => Err(format!(
                        "invalid {}: '{}', expected one of [{}]",
                        stringify!($name),
                        s,
                        Self::all_values().join(", ")
                    )),
                }
            }
        }

        impl From<$name> for String {
            fn from(val: $name) -> String {
                val.as_str().to_string()
            }
        }

        // ── sqlx support: SQLite ──────────────────────────────────────
        #[cfg(feature = "db-sqlite")]
        impl sqlx::Type<sqlx::Sqlite> for $name {
            fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
                <String as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
        }

        #[cfg(feature = "db-sqlite")]
        impl sqlx::Decode<'_, sqlx::Sqlite> for $name {
            fn decode(
                value: sqlx::sqlite::SqliteValueRef<'_>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <String as sqlx::Decode<'_, sqlx::Sqlite>>::decode(value)?;
                s.parse().map_err(Into::into)
            }
        }

        #[cfg(feature = "db-sqlite")]
        impl<'q> sqlx::Encode<'q, sqlx::Sqlite> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut Vec<sqlx::sqlite::SqliteArgumentValue<'q>>,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<'q, sqlx::Sqlite>>::encode(self.as_str(), buf)
            }
        }

        // ── sqlx support: PostgreSQL ──────────────────────────────────
        #[cfg(feature = "db-postgres")]
        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <String as sqlx::Type<sqlx::Postgres>>::type_info()
            }
        }

        #[cfg(feature = "db-postgres")]
        impl sqlx::Decode<'_, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'_>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <String as sqlx::Decode<'_, sqlx::Postgres>>::decode(value)?;
                s.parse().map_err(Into::into)
            }
        }

        #[cfg(feature = "db-postgres")]
        impl<'q> sqlx::Encode<'q, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<'q, sqlx::Postgres>>::encode(self.as_str(), buf)
            }
        }

        // ── sqlx support: MySQL ──────────────────────────────────────
        #[cfg(feature = "db-mysql")]
        impl sqlx::Type<sqlx::MySql> for $name {
            fn type_info() -> sqlx::mysql::MySqlTypeInfo {
                <String as sqlx::Type<sqlx::MySql>>::type_info()
            }
        }

        #[cfg(feature = "db-mysql")]
        impl sqlx::Decode<'_, sqlx::MySql> for $name {
            fn decode(
                value: sqlx::mysql::MySqlValueRef<'_>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <String as sqlx::Decode<'_, sqlx::MySql>>::decode(value)?;
                s.parse().map_err(Into::into)
            }
        }

        #[cfg(feature = "db-mysql")]
        impl<'q> sqlx::Encode<'q, sqlx::MySql> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::mysql::MySqlArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <&str as sqlx::Encode<'q, sqlx::MySql>>::encode(self.as_str(), buf)
            }
        }
    };
}
