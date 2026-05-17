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

/// Execute a tenant-aware INSERT in one call.
#[macro_export]
macro_rules! tenant_insert {
    ($pool:expr, $table:literal, [$($col:literal => $val:expr),* $(,)?], $tenant_id:expr) => {{
        { const _: () = assert!($crate::db::schema_meta::table_exists($table), concat!("table \"", $table, "\" not found in schema")); }
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $col), concat!("column \"", $col, "\" not found in table \"", $table, "\"")); })*
        let sql = $crate::db::tenant::insert_sql($table, &[$($col),*], $tenant_id);
        let mut _q = sqlx::query(&sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.execute($pool).await
    }};
}

/// Execute a tenant-aware UPDATE in one call.
///
/// Supports both bound values (`col => val`) and raw SQL expressions (`col => "expr"`)
/// in the SET clause, plus optional extra WHERE conditions and tenant filter.
///
/// # Syntax
///
/// ```ignore
/// tenant_update!(pool, "table",
///     bind:  ["col1" => val1, "col2" => val2, ...],
///     raw:   ["col3" => "datetime('now')", "col4" => "version + 1"],
///     where: "pk_col" => pk_val,
///     and:   ["extra_col" => extra_val],   // optional
///     tenant: tenant_id                     // optional
/// )?;
/// ```
///
/// - `bind` — columns bound as `?` placeholders (required, can be empty `[]`)
/// - `raw`  — columns set to literal SQL expressions (optional, defaults to `[]`)
/// - `where` — primary key column and value (required)
/// - `and`  — extra `AND col = ?` conditions (optional, defaults to `[]`)
/// - `tenant` — optional `Option<&str>` tenant filter
///
/// # Examples
///
/// ```ignore
/// // Pure bind:
/// tenant_update!(pool, "tags",
///     bind: ["name" => name, "slug" => slug],
///     where: "id" => tag_id,
///     tenant: tenant_id
/// )?;
///
/// // Bind + raw expressions:
/// tenant_update!(pool, "orders",
///     bind: ["status" => status],
///     raw: ["updated_at" => "datetime('now')"],
///     where: "id" => id,
///     tenant: tenant_id
/// )?;
///
/// // With extra WHERE (optimistic locking):
/// tenant_update!(pool, "products",
///     bind: ["title" => &cmd.title, "price" => cmd.price],
///     raw: ["updated_at" => "datetime('now')", "version" => "version + 1"],
///     where: "id" => cmd.id,
///     and: ["version" => cmd.version],
///     tenant: tenant_id
/// )?;
/// ```
#[macro_export]
macro_rules! tenant_update {
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        raw: [$($rcol:literal => $rval:literal),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        and: [$($acol:literal => $aval:expr),* $(,)?],
        tenant: $tenant_id:expr $(,)?
    ) => {{
        { const _: () = assert!($crate::db::schema_meta::table_exists($table), concat!("table \"", $table, "\" not found in schema")); }
        { const _: () = assert!($crate::db::schema_meta::column_exists($table, $pk_col), concat!("column \"", $pk_col, "\" not found in table \"", $table, "\"")); }
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $bcol), concat!("column \"", $bcol, "\" not found in table \"", $table, "\"")); })*
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $rcol), concat!("column \"", $rcol, "\" not found in table \"", $table, "\"")); })*
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $acol), concat!("column \"", $acol, "\" not found in table \"", $table, "\"")); })*
        let _bind_count: usize = 0 $(+ { let _ = &$bcol; 1 })*;
        let _raw_cols: Vec<&str> = vec![$($rcol),*];
        let _raw_vals: Vec<&str> = vec![$($rval),*];
        let _extra_count: usize = 0 $(+ { let _ = &$acol; 1 })*;

        let _total_bind = _bind_count + 1 + _extra_count;
        let _tenant_idx = _total_bind + 1;
        let _t_ph = $crate::db::tenant::tenant_filter_ph($tenant_id, _tenant_idx);

        let mut _ph_idx = 1usize;
        let mut _set_parts: Vec<String> = vec![
            $(
                {
                    let _ph = $crate::db::dialect::ph(_ph_idx);
                    _ph_idx += 1;
                    format!("{} = {}", $bcol, _ph)
                },
            )*
        ];
        for (_c, _e) in _raw_cols.iter().zip(_raw_vals.iter()) {
            _set_parts.push(format!("{_c} = {_e}"));
        }

        let _pk_ph = $crate::db::dialect::ph(_ph_idx);
        _ph_idx += 1;

        let _extra_sql: Vec<String> = vec![
            $(
                {
                    let _ph = $crate::db::dialect::ph(_ph_idx);
                    _ph_idx += 1;
                    format!("AND {} = {}", $acol, _ph)
                },
            )*
        ];

        let _sql = format!(
            "UPDATE {} SET {} WHERE {} = {}{}{}",
            $table,
            _set_parts.join(", "),
            $pk_col,
            _pk_ph,
            _extra_sql.join(""),
            _t_ph,
        );

        let mut _q = sqlx::query(&_sql)$(.bind($bval))*;
        _q = _q.bind($pk_val);
        $(_q = _q.bind($aval);)*
        $crate::bind_tenant!(_q, $tenant_id);
        _q.execute($pool).await
    }};

    // Without raw:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        and: [$($acol:literal => $aval:expr),* $(,)?],
        tenant: $tenant_id:expr $(,)?
    ) => {
        $crate::tenant_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [],
            where: $pk_col => $pk_val,
            and: [$($acol => $aval),*],
            tenant: $tenant_id
        )
    };

    // Without and:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        raw: [$($rcol:literal => $rval:literal),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        tenant: $tenant_id:expr $(,)?
    ) => {
        $crate::tenant_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [$($rcol => $rval),*],
            where: $pk_col => $pk_val,
            and: [],
            tenant: $tenant_id
        )
    };

    // Without raw and without and:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        tenant: $tenant_id:expr $(,)?
    ) => {
        $crate::tenant_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [],
            where: $pk_col => $pk_val,
            and: [],
             tenant: $tenant_id
         )
     };
}

/// Execute a tenant-aware SELECT … WHERE col = ? [AND tenant_id = ?], returning `Option<T>`.
///
/// # Example
///
/// ```ignore
/// let post: Option<Post> = tenant_find!(pool, "posts" => Post, "slug" => slug, tenant_id)?;
/// ```
#[macro_export]
macro_rules! tenant_find {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr, $tenant_id:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}{}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
            $crate::db::tenant::tenant_filter_ph($tenant_id, 2),
        );
        let mut _q = sqlx::query_as::<_, $ty>(&_sql).bind($val);
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_optional($pool).await
    }};
}

/// Execute a tenant-aware SELECT … WHERE col = ? [AND tenant_id = ?], returning `T` (error if not found).
///
/// # Example
///
/// ```ignore
/// let tag: Tag = tenant_find_one!(pool, "tags" => Tag, "id" => id, tenant_id)?;
/// ```
#[macro_export]
macro_rules! tenant_find_one {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr, $tenant_id:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}{}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
            $crate::db::tenant::tenant_filter_ph($tenant_id, 2),
        );
        let mut _q = sqlx::query_as::<_, $ty>(&_sql).bind($val);
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_one($pool).await
    }};
}

/// Execute a tenant-aware SELECT … WHERE col = ? [AND tenant_id = ?], returning `Vec<T>`.
///
/// # Example
///
/// ```ignore
/// let items: Vec<OrderItem> = tenant_find_all!(pool, "order_items" => OrderItem, "order_id" => order_id, tenant_id)?;
/// ```
#[macro_export]
macro_rules! tenant_find_all {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr, $tenant_id:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}{}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
            $crate::db::tenant::tenant_filter_ph($tenant_id, 2),
        );
        let mut _q = sqlx::query_as::<_, $ty>(&_sql).bind($val);
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_all($pool).await
    }};
}

/// Execute a tenant-aware DELETE in one call.
///
/// Returns the raw `sqlx::Result<SqliteQueryResult>` so the caller can decide
/// how to handle affected rows (e.g. `AppError::expect_affected`).
///
/// # Example
///
/// ```ignore
/// let result = tenant_delete!(pool, "tags", "id" => id, tenant_id)?;
/// AppError::expect_affected(&result, "tag")?;
/// ```
#[macro_export]
macro_rules! tenant_delete {
    ($pool:expr, $table:literal, $col:literal => $val:expr, $tenant_id:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "DELETE FROM {} WHERE {} = {}{}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
            $crate::db::tenant::tenant_filter_ph($tenant_id, 2),
        );
        let mut _q = sqlx::query(&_sql).bind($val);
        $crate::bind_tenant!(_q, $tenant_id);
        _q.execute($pool).await
    }};
}

/// Execute a tenant-aware `query_as` with pre-built SQL.
///
/// For JOIN queries or custom SELECT columns where the SQL must be hand-crafted.
/// Handles bind + bind_tenant + fetch in one call.
///
/// # Example
///
/// ```ignore
/// let sql = format!(
///     "SELECT t.id, t.name FROM tags t WHERE t.id = {}{}",
///     ph(1), tenant_filter_aliased_ph("t", tenant_id, 2)
/// );
/// let rows = tenant_query!(pool, TagRow, &sql, [id], tenant_id, fetch_all)?;
/// ```
#[macro_export]
macro_rules! tenant_query {
    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], $tenant_id:expr, fetch_optional) => {{
        let mut _q = sqlx::query_as::<_, $ty>($sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_optional($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], $tenant_id:expr, fetch_one) => {{
        let mut _q = sqlx::query_as::<_, $ty>($sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_one($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], $tenant_id:expr, fetch_all) => {{
        let mut _q = sqlx::query_as::<_, $ty>($sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_all($pool).await
    }};
}

/// Execute a tenant-aware `query_scalar` with pre-built SQL.
///
/// Like `tenant_query!` but uses `query_scalar` for aggregate results (COUNT, SUM, etc.).
///
/// # Example
///
/// ```ignore
/// let count_sql = format!("SELECT COUNT(*) FROM tags WHERE 1=1{}", tenant_filter_ph(tenant_id, 1));
/// let total: i64 = tenant_scalar!(pool, i64, &count_sql, [], tenant_id, fetch_one)?;
/// ```
#[macro_export]
macro_rules! tenant_scalar {
    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], $tenant_id:expr, fetch_optional) => {{
        let mut _q = sqlx::query_scalar::<_, $ty>($sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_optional($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], $tenant_id:expr, fetch_one) => {{
        let mut _q = sqlx::query_scalar::<_, $ty>($sql)$(.bind($val))*;
        $crate::bind_tenant!(_q, $tenant_id);
        _q.fetch_one($pool).await
    }};
}

/// Compile-time schema validation for raw SQL functions.
///
/// Validates that a table and its columns exist in the schema at compile time.
/// Use this for complex queries that cannot be replaced by `crud_*!` or `tenant_*!` macros.
///
/// # Example
///
/// ```ignore
/// pub async fn find_by_token(pool: &Pool, token: &str) -> AppResult<Option<ResetToken>> {
///     check_schema!("password_reset_tokens", "token", "user_id", "expires_at", "used_at");
///     let sql = format!("SELECT * FROM password_reset_tokens WHERE token = {}", ph(1));
///     sqlx::query_as::<_, ResetToken>(&sql)
///         .bind(token)
///         .fetch_optional(pool)
///         .await?
/// }
/// ```
#[macro_export]
macro_rules! check_schema {
    ($table:literal, $($col:literal),* $(,)?) => {
        const _: () = {
            assert!($crate::db::schema_meta::table_exists($table), concat!("table \"", $table, "\" not found in schema"));
            $(assert!($crate::db::schema_meta::column_exists($table, $col), concat!("column \"", $col, "\" not found in table \"", $table, "\""));)*
        };
    };
}

// ---------------------------------------------------------------------------
// CRUD macro family — no tenant filtering, with compile-time validation
// ---------------------------------------------------------------------------

/// Execute a CRUD INSERT in one call (no tenant filtering).
#[macro_export]
macro_rules! crud_insert {
    ($pool:expr, $table:literal, [$($col:literal => $val:expr),* $(,)?]) => {{
        { const _: () = assert!($crate::db::schema_meta::table_exists($table), concat!("table \"", $table, "\" not found in schema")); }
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $col), concat!("column \"", $col, "\" not found in table \"", $table, "\"")); })*
        let sql = $crate::db::tenant::insert_sql($table, &[$($col),*], None::<&str>);
        let mut _q = sqlx::query(&sql)$(.bind($val))*;
        _q.execute($pool).await
    }};
}

/// Execute a CRUD SELECT … WHERE col = ?, returning `Option<T>` (no tenant filtering).
#[macro_export]
macro_rules! crud_find {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
        );
        sqlx::query_as::<_, $ty>(&_sql)
            .bind($val)
            .fetch_optional($pool)
            .await
    }};
}

/// Execute a CRUD SELECT … WHERE col = ?, returning `T` (error if not found, no tenant filtering).
#[macro_export]
macro_rules! crud_find_one {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
        );
        sqlx::query_as::<_, $ty>(&_sql)
            .bind($val)
            .fetch_one($pool)
            .await
    }};
}

/// Execute a CRUD SELECT … WHERE col = ?, returning `Vec<T>` (no tenant filtering).
#[macro_export]
macro_rules! crud_find_all {
    ($pool:expr, $table:literal => $ty:ty, $col:literal => $val:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "SELECT * FROM {} WHERE {} = {}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
        );
        sqlx::query_as::<_, $ty>(&_sql)
            .bind($val)
            .fetch_all($pool)
            .await
    }};
}

/// Execute a CRUD SELECT … (no WHERE), returning `Vec<T>` (no tenant filtering).
#[macro_export]
macro_rules! crud_list {
    ($pool:expr, $table:literal => $ty:ty) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        let _sql = format!("SELECT * FROM {}", $table);
        sqlx::query_as::<_, $ty>(&_sql).fetch_all($pool).await
    }};
}

/// Execute a CRUD DELETE in one call (no tenant filtering).
#[macro_export]
macro_rules! crud_delete {
    ($pool:expr, $table:literal, $col:literal => $val:expr) => {{
        {
            const _: () = assert!(
                $crate::db::schema_meta::table_exists($table),
                concat!("table \"", $table, "\" not found in schema")
            );
        }
        {
            const _: () = assert!(
                $crate::db::schema_meta::column_exists($table, $col),
                concat!("column \"", $col, "\" not found in table \"", $table, "\"")
            );
        }
        let _sql = format!(
            "DELETE FROM {} WHERE {} = {}",
            $table,
            $col,
            $crate::db::dialect::ph(1),
        );
        sqlx::query(&_sql).bind($val).execute($pool).await
    }};
}

/// Execute a CRUD UPDATE in one call (no tenant filtering).
///
/// Same syntax as `tenant_update!` but without `tenant:` parameter.
#[macro_export]
macro_rules! crud_update {
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        raw: [$($rcol:literal => $rval:literal),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        and: [$($acol:literal => $aval:expr),* $(,)?] $(,)?
    ) => {{
        { const _: () = assert!($crate::db::schema_meta::table_exists($table), concat!("table \"", $table, "\" not found in schema")); }
        { const _: () = assert!($crate::db::schema_meta::column_exists($table, $pk_col), concat!("column \"", $pk_col, "\" not found in table \"", $table, "\"")); }
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $bcol), concat!("column \"", $bcol, "\" not found in table \"", $table, "\"")); })*
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $rcol), concat!("column \"", $rcol, "\" not found in table \"", $table, "\"")); })*
        $({ const _: () = assert!($crate::db::schema_meta::column_exists($table, $acol), concat!("column \"", $acol, "\" not found in table \"", $table, "\"")); })*
        let _bind_count: usize = 0 $(+ { let _ = &$bcol; 1 })*;
        let _raw_cols: Vec<&str> = vec![$($rcol),*];
        let _raw_vals: Vec<&str> = vec![$($rval),*];
        let _extra_count: usize = 0 $(+ { let _ = &$acol; 1 })*;

        let mut _ph_idx = 1usize;
        let mut _set_parts: Vec<String> = vec![
            $(
                {
                    let _ph = $crate::db::dialect::ph(_ph_idx);
                    _ph_idx += 1;
                    format!("{} = {}", $bcol, _ph)
                },
            )*
        ];
        for (_c, _e) in _raw_cols.iter().zip(_raw_vals.iter()) {
            _set_parts.push(format!("{_c} = {_e}"));
        }

        let _pk_ph = $crate::db::dialect::ph(_ph_idx);
        _ph_idx += 1;

        let _extra_sql: Vec<String> = vec![
            $(
                {
                    let _ph = $crate::db::dialect::ph(_ph_idx);
                    _ph_idx += 1;
                    format!("AND {} = {}", $acol, _ph)
                },
            )*
        ];

        let _sql = format!(
            "UPDATE {} SET {} WHERE {} = {}{}",
            $table,
            _set_parts.join(", "),
            $pk_col,
            _pk_ph,
            _extra_sql.join(""),
        );

        let mut _q = sqlx::query(&_sql)$(.bind($bval))*;
        _q = _q.bind($pk_val);
        $(_q = _q.bind($aval);)*
        _q.execute($pool).await
    }};

    // Without raw:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        where: $pk_col:literal => $pk_val:expr,
        and: [$($acol:literal => $aval:expr),* $(,)?] $(,)?
    ) => {
        $crate::crud_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [],
            where: $pk_col => $pk_val,
            and: [$($acol => $aval),*]
        )
    };

    // Without and:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        raw: [$($rcol:literal => $rval:literal),* $(,)?],
        where: $pk_col:literal => $pk_val:expr $(,)?
    ) => {
        $crate::crud_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [$($rcol => $rval),*],
            where: $pk_col => $pk_val,
            and: []
        )
    };

    // Without raw and and:
    (
        $pool:expr, $table:literal,
        bind: [$($bcol:literal => $bval:expr),* $(,)?],
        where: $pk_col:literal => $pk_val:expr $(,)?
    ) => {
        $crate::crud_update!(
            $pool, $table,
            bind: [$($bcol => $bval),*],
            raw: [],
            where: $pk_col => $pk_val,
            and: []
        )
    };
}

/// Execute a CRUD `query_as` with pre-built SQL (no tenant filtering).
#[macro_export]
macro_rules! crud_query {
    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], fetch_optional) => {{
        sqlx::query_as::<_, $ty>($sql)$(.bind($val))*.fetch_optional($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], fetch_one) => {{
        sqlx::query_as::<_, $ty>($sql)$(.bind($val))*.fetch_one($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], fetch_all) => {{
        sqlx::query_as::<_, $ty>($sql)$(.bind($val))*.fetch_all($pool).await
    }};
}

/// Execute a CRUD `query_scalar` with pre-built SQL (no tenant filtering).
#[macro_export]
macro_rules! crud_scalar {
    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], fetch_optional) => {{
        sqlx::query_scalar::<_, $ty>($sql)$(.bind($val))*.fetch_optional($pool).await
    }};

    ($pool:expr, $ty:ty, $sql:expr, [$($val:expr),* $(,)?], fetch_one) => {{
        sqlx::query_scalar::<_, $ty>($sql)$(.bind($val))*.fetch_one($pool).await
    }};
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
        pool
    }};
}
