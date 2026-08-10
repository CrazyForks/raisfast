/// Register a single route with automatic POST compatibility for PUT/DELETE.
///
/// # Syntax
///
/// ```ignore
/// // Without permission (backward compatible):
/// reg_route!(router, registry, restful, "/path", method, handler, "source", "name");
///
/// // With permission declaration:
/// reg_route!(router, registry, restful, "/path", method, handler, "source", "name", "admin");
///
/// // With custom middleware (MethodRouter with .layer()):
/// reg_route!(router, registry, restful, "/path", post, post(handler).layer(mw), "source", "name", "public", layered);
/// ```
///
/// - `restful`: bool (typically `config.api_restful`)
/// - `method`: `get`, `post`, `create`, `put`, or `delete`
/// - `handler`: handler function, or `MethodRouter` expression when `layered` is used
/// - `permission` (optional): `"public"`, `"admin"`, `"authed"`, or `"resource:action"`
/// - `layered` (optional, must come after permission): treat `$handler` as a pre-built `MethodRouter`
/// - When `restful=false`:
///   - `create` → `POST /path/create`
///   - `put`    → `POST /path/update`
///   - `delete` → `POST /path/delete`
/// - When `restful=true`: no extra routes generated
#[macro_export]
macro_rules! reg_route {
    // ═══════════════════════════════════════════════════════════════
    // Layered arms: accept a pre-built MethodRouter (with .layer() applied).
    // Requires permission (9 params + `layered`).
    // ═══════════════════════════════════════════════════════════════
    ($router:expr, $registry:expr, $restful:expr, $path:literal, get, $router_expr:expr, $source:expr, $name:expr, $perm:expr, layered $(,)?) => {{
        let r = $router.route($path, $router_expr);
        $registry.record_perm("GET", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    ($router:expr, $registry:expr, $restful:expr, $path:literal, post, $router_expr:expr, $source:expr, $name:expr, $perm:expr, layered $(,)?) => {{
        let r = $router.route($path, $router_expr);
        $registry.record_perm("POST", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    ($router:expr, $registry:expr, $restful:expr, $path:literal, put, $router_expr:expr, $source:expr, $name:expr, $perm:expr, layered $(,)?) => {{
        let r = $router.route($path, $router_expr);
        $registry.record_perm("PUT", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    ($router:expr, $registry:expr, $restful:expr, $path:literal, delete, $router_expr:expr, $source:expr, $name:expr, $perm:expr, layered $(,)?) => {{
        let r = $router.route($path, $router_expr);
        $registry.record_perm("DELETE", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    ($router:expr, $registry:expr, $restful:expr, $path:literal, create, $router_expr:expr, $source:expr, $name:expr, $perm:expr, layered $(,)?) => {{
        let r = $router.route($path, $router_expr);
        $registry.record_perm("POST", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    // ═══════════════════════════════════════════════════════════════
    // Arms WITH permission declaration (9 parameters)
    // ═══════════════════════════════════════════════════════════════

    // ── GET + perm ──
    ($router:expr, $registry:expr, $restful:expr, $path:literal, get, $handler:expr, $source:expr, $name:expr, $perm:expr $(,)?) => {{
        let r = $router.route($path, axum::routing::get($handler));
        $registry.record_perm("GET", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    // ── POST + perm ──
    ($router:expr, $registry:expr, $restful:expr, $path:literal, post, $handler:expr, $source:expr, $name:expr, $perm:expr $(,)?) => {{
        let r = $router.route($path, axum::routing::post($handler));
        $registry.record_perm("POST", concat!("/api/v1", $path), $source, $name, $perm);
        r
    }};

    // ── CREATE + perm ──
    ($router:expr, $registry:expr, $restful:expr, $path:literal, create, $handler:expr, $source:expr, $name:expr, $perm:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::post($handler));
            $registry.record_perm("POST", concat!("/api/v1", $path), $source, $name, $perm);
            r
        } else {
            let __compat_path = concat!($path, "/create");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record_perm(
                "POST",
                concat!("/api/v1", $path, "/create"),
                $source,
                $name,
                $perm,
            );
            r
        };
        r
    }};

    // ── PUT + perm ──
    ($router:expr, $registry:expr, $restful:expr, $path:literal, put, $handler:expr, $source:expr, $name:expr, $perm:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::put($handler));
            $registry.record_perm("PUT", concat!("/api/v1", $path), $source, $name, $perm);
            r
        } else {
            let __compat_path = concat!($path, "/update");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record_perm(
                "POST",
                concat!("/api/v1", $path, "/update"),
                $source,
                $name,
                $perm,
            );
            r
        };
        r
    }};

    // ── DELETE + perm ──
    ($router:expr, $registry:expr, $restful:expr, $path:literal, delete, $handler:expr, $source:expr, $name:expr, $perm:expr $(,)?) => {{
        let r = if $restful {
            let r = $router.route($path, axum::routing::delete($handler));
            $registry.record_perm("DELETE", concat!("/api/v1", $path), $source, $name, $perm);
            r
        } else {
            let __compat_path = concat!($path, "/delete");
            let r = $router.route(__compat_path, axum::routing::post($handler));
            $registry.record_perm(
                "POST",
                concat!("/api/v1", $path, "/delete"),
                $source,
                $name,
                $perm,
            );
            r
        };
        r
    }};

    // ═══════════════════════════════════════════════════════════════
    // Arms WITHOUT permission (backward compatible, 8 parameters)
    // ═══════════════════════════════════════════════════════════════
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
macro_rules! in_transaction {
    ($pool:expr, $tx:ident, $body:block) => {{
        let __write_guard = $crate::db::connection::acquire_write().await;
        #[allow(unused_mut)]
        let mut $tx: $crate::db::pool::Transaction<'_> = match $pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                return Err($crate::errors::app_error::AppError::Internal(
                    anyhow::anyhow!("begin tx: {e}"),
                ));
            }
        };
        let __tx_result: Result<_, $crate::errors::app_error::AppError> = async { $body }.await;
        if __tx_result.is_ok() {
            if let Err(e) = $tx.commit().await {
                return Err($crate::errors::app_error::AppError::Internal(
                    anyhow::anyhow!("commit tx: {e}"),
                ));
            }
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

        $crate::__define_enum_sqlx!($name);
    };
}

#[macro_export]
macro_rules! __define_enum_sqlx {
    ($name:ident) => {
        #[cfg(feature = "db-sqlite")]
        $crate::__define_enum_sqlx_impl! {
            $name,
            db = sqlx::Sqlite,
            type_info = sqlx::sqlite::SqliteTypeInfo,
            value_ref = sqlx::sqlite::SqliteValueRef<'_>,
        }

        #[cfg(feature = "db-postgres")]
        $crate::__define_enum_sqlx_impl! {
            $name,
            db = sqlx::Postgres,
            type_info = sqlx::postgres::PgTypeInfo,
            value_ref = sqlx::postgres::PgValueRef<'_>,
        }

        #[cfg(feature = "db-mysql")]
        $crate::__define_enum_sqlx_impl! {
            $name,
            db = sqlx::MySql,
            type_info = sqlx::mysql::MySqlTypeInfo,
            value_ref = sqlx::mysql::MySqlValueRef<'_>,
        }
    };
}

#[macro_export]
macro_rules! __define_enum_sqlx_impl {
    (
        $name:ident,
        db = $db:ty,
        type_info = $type_info:ty,
        value_ref = $value_ref:ty,
    ) => {
        impl sqlx::Type<$db> for $name {
            fn type_info() -> $type_info {
                <String as sqlx::Type<$db>>::type_info()
            }

            fn compatible(ty: &$type_info) -> bool {
                <String as sqlx::Type<$db>>::compatible(ty)
            }
        }

        impl sqlx::Decode<'_, $db> for $name {
            fn decode(value: $value_ref) -> Result<Self, sqlx::error::BoxDynError> {
                let s = <String as sqlx::Decode<'_, $db>>::decode(value)?;
                s.parse().map_err(Into::into)
            }
        }

        impl<'q> sqlx::Encode<'q, $db> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut <$db as sqlx::Database>::ArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                <String as sqlx::Encode<'q, $db>>::encode_by_ref(&self.to_string(), buf)
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
/// let mut q = sqlx::query(crate::db::safe_sql(&sql));
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
/// let mut q = sqlx::query_as::<_, Tag>(crate::db::safe_sql(&sql)).bind(id);
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

/// Create a test pool with schema applied.
///
/// Uses SQLite `:memory:` when the `db-sqlite` feature is active (fresh DB per
/// test), or a shared PostgreSQL / MySQL test database otherwise. The schema
/// is fully idempotent (`CREATE TABLE IF NOT EXISTS` + `ON CONFLICT DO NOTHING`
/// on all seed data), so concurrent tests can safely apply it in parallel.
/// Tests use unique Snowflake IDs for their data, avoiding cross-test
/// collisions. Set the env var `RAISFAST_TEST_DB_URL` to override the default
/// test connection string.
#[macro_export]
macro_rules! test_pool {
    () => {{
        #[cfg(feature = "db-sqlite")]
        {
            let pool = $crate::db::Pool::connect("sqlite::memory:").await.unwrap();
            sqlx::query($crate::db::schema::SCHEMA_SQL)
                .execute(&pool)
                .await
                .unwrap();
            pool
        }
        #[cfg(not(feature = "db-sqlite"))]
        {
            let url = std::env::var("RAISFAST_TEST_DB_URL").unwrap_or_else(|_| {
                panic!("RAISFAST_TEST_DB_URL must be set for non-SQLite tests")
            });
            let pool = $crate::db::Pool::connect(&url).await.unwrap();
            $crate::db::connection::execute_schema(&pool).await.unwrap();
            pool
        }
    }};
}

#[cfg(feature = "export-types")]
#[macro_export]
macro_rules! export_types {
    ($($ty:ty),* $(,)?) => {
        $(
            inventory::submit! {
                $crate::export_type::ExportType::new::<$ty>()
            }
        )*
    };
}

#[cfg(not(feature = "export-types"))]
#[macro_export]
macro_rules! export_types {
    ($($ty:ty),* $(,)?) => {};
}
