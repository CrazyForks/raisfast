/// Register a single route with automatic POST compatibility for PUT/DELETE.
///
/// # Syntax
///
/// ```ignore
/// reg_route!(router, registry, restful, "/path", method, handler, "source", "name");
/// ```
///
/// - `restful`: bool (typically `config.api_restful`)
/// - `method`: `get`, `post`, `create`, `put`, or `delete`
/// - `handler`: handler function or `axum::routing::MethodRouter` expression
/// - When `restful=false`:
///   - `create` → `POST /path/create`
///   - `put`    → `POST /path/update`
///   - `delete` → `POST /path/delete`
/// - When `restful=true`: no extra routes generated
///
/// # Examples
///
/// ```ignore
/// reg_route!(r, reg, restful, "/pages", get, list, "public", "pages");
/// reg_route!(r, reg, restful, "/pages", create, create_page, "public", "pages");
/// reg_route!(r, reg, restful, "/pages/{id}", put, update, "admin", "pages");
/// reg_route!(r, reg, restful, "/pages/{id}", delete, remove, "admin", "pages");
/// // Non-CRUD POST (login, batch, callback — always POST /path):
/// reg_route!(r, reg, restful, "/auth/login", post, login, "public", "auth");
/// // With middleware:
/// reg_route!(r, reg, restful, "/auth/login", post, post(login).layer(mw), "public", "auth");
/// ```
#[macro_export]
macro_rules! reg_route {
    // ── GET ──────────────────────────────────────────────────────
    ($router:expr, $registry:expr, $restful:expr, $path:literal, get, $handler:expr, $source:expr, $name:expr $(,)?) => {{
        let r = $router.route($path, axum::routing::get($handler));
        $registry.record("GET", concat!("/api/v1", $path), $source, $name);
        r
    }};

    // ── POST (non-CRUD, always POST /path) ──────────────────────
    ($router:expr, $registry:expr, $restful:expr, $path:literal, post, $handler:expr, $source:expr, $name:expr $(,)?) => {{
        let r = $router.route($path, axum::routing::post($handler));
        $registry.record("POST", concat!("/api/v1", $path), $source, $name);
        r
    }};

    // ── CREATE ──────────────────────────────────────────────────
    //   restful=true  → POST /path
    //   restful=false → POST /path/create
    ($router:expr, $registry:expr, $restful:expr, $path:literal, create, $handler:expr, $source:expr, $name:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::post($handler));
            $registry.record("POST", concat!("/api/v1", $path), $source, $name);
            r
        } else {
            let __compat_path = concat!($path, "/create");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record("POST", concat!("/api/v1", $path, "/create"), $source, $name);
            r
        };
        r
    }};

    // ── PUT ──────────────────────────────────────────────────────
    //   restful=true  → PUT /path
    //   restful=false → POST /path/update
    ($router:expr, $registry:expr, $restful:expr, $path:literal, put, $handler:expr, $source:expr, $name:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::put($handler));
            $registry.record("PUT", concat!("/api/v1", $path), $source, $name);
            r
        } else {
            let __compat_path = concat!($path, "/update");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record("POST", concat!("/api/v1", $path, "/update"), $source, $name);
            r
        };
        r
    }};

    // ── DELETE ───────────────────────────────────────────────────
    //   restful=true  → DELETE /path
    //   restful=false → POST /path/delete
    ($router:expr, $registry:expr, $restful:expr, $path:literal, delete, $handler:expr, $source:expr, $name:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::delete($handler));
            $registry.record("DELETE", concat!("/api/v1", $path), $source, $name);
            r
        } else {
            let __compat_path = concat!($path, "/delete");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record("POST", concat!("/api/v1", $path, "/delete"), $source, $name);
            r
        };
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
        #[allow(unused_mut)]
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

/// Bind a value to a query if Some, no-op if None.
///
/// General-purpose optional bind — works for any `Option<T>` parameter.
///
/// # Examples
///
/// ```ignore
/// let mut q = sqlx::query(&sql);
/// bind_optional!(q, name);       // if name is Some, bind it
/// bind_optional!(q, description); // same
/// ```
#[macro_export]
macro_rules! bind_optional {
    ($q:ident, $val:expr) => {
        if let Some(_v) = $val {
            $q = $q.bind(_v);
        }
    };
}

/// Bind tenant_id to a query if Some, no-op if None.
///
/// Replaces the repeated 3-line pattern:
/// ```ignore
/// if let Some(tid) = tenant_id {
///     q = q.bind(tid);
/// }
/// ```
///
/// # Examples
///
/// ```ignore
/// let mut q = sqlx::query_as::<_, Tag>(&sql).bind(id);
/// bind_tenant!(q, tenant_id);
/// q.fetch_one(pool).await?
/// ```
#[macro_export]
macro_rules! bind_tenant {
    ($q:ident, $tenant_id:expr) => {
        if let Some(_tid) = $tenant_id {
            $q = $q.bind(_tid);
        }
    };
}

/// Create an in-memory SQLite test pool with schema applied.
#[macro_export]
macro_rules! test_pool {
    () => {{
        let pool = $crate::db::Pool::connect("sqlite::memory:").await.unwrap();
        sqlx::query($crate::db::schema::SCHEMA_SQL)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query($crate::db::schema::TENANTABLE_SQL)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }};
}
