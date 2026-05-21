# 数据库兼容设计

## 架构概览

项目通过 **编译时 feature flag** 选择唯一的数据库后端，所有差异封装在 `src/db/` 模块内。
业务代码（models / services / handlers / content_type）完全不感知底层数据库类型。

```
src/db/                          ← 唯一知道数据库差异的地方
  driver.rs    ← sealed trait DbDriver + 3 个 impl（核心）
  pool.rs      ← 7 个类型别名（Pool, DbRow, DbArguments...）
  sql_type.rs  ← 12 种 SQL 列类型映射表
  connection.rs← 连接池初始化 + schema 执行
  backup.rs    ← 备份逻辑
  tenant.rs    ← 租户隔离辅助
  schema.rs    ← 编译时 schema SQL（build.rs 生成）

src/macros.rs                   ← define_enum! 宏（sqlx Type/Decode/Encode）
raisfast-derive/src/schema.rs   ← proc-macro 编译时 Dialect 枚举
migrations/                     ← 每个数据库独立的 SQL 文件
  sqlite/schema.sqlite.sql
  postgres/schema.postgres.sql
  mysql/schema.mysql.sql
```

## 核心设计模式

### 1. Sealed Trait — `DbDriver`

`src/db/driver.rs` 定义了一个 sealed trait，编译时只有唯一的 impl 被激活：

```rust
pub struct Sqlite;
pub struct Postgres;
pub struct MySql;

mod sealed { pub trait Sealed {} }
impl sealed::Sealed for Sqlite {}
impl sealed::Sealed for Postgres {}
impl sealed::Sealed for MySql {}

#[cfg(feature = "db-sqlite")]
pub type Driver = Sqlite;          // 编译时选定

pub trait DbDriver: sealed::Sealed {
    fn pk_type() -> &'static str;           // "INTEGER" / "BIGINT"
    fn ph(idx: usize) -> String;            // "?" / "$1"
    fn now_fn() -> &'static str;            // "strftime(...)" / "NOW()"
    fn ago_expr(days: i64) -> String;
    fn date_trunc_day(col: &str) -> String;
    fn upsert_clause(...) -> String;
    fn excluded_col(col: &str) -> String;
    fn returning_clause() -> &'static str;
    fn returning_col(col: &str) -> String;
    fn insert_ignore_sql(...) -> String;
    fn columns_sql(table: &str) -> (String, usize);
    fn column_names_sql(table: &str) -> (String, usize);
    fn rebuild_wrapper_sql(...) -> String;
    fn has_column(pool, table, column) -> impl Future<Output = bool> + Send;
    fn table_exists(pool, table) -> impl Future<Output = bool> + Send;
    fn list_user_tables(pool, excluded) -> impl Future<Output = Vec<String>> + Send;
    fn fetch_columns_with_types(pool, table) -> impl Future<Output = Result<...>> + Send;
}
```

**业务代码调用方式：**

```rust
use crate::db::{DbDriver, Driver};

let sql = format!("SELECT * FROM users WHERE id = {}", Driver::ph(1));
let pk = Driver::pk_type();                     // "INTEGER PRIMARY KEY"
let exists = Driver::table_exists(pool, "users").await;
```

`Driver` 是编译时类型别名，`DbDriver` 是 sealed trait — 编译器直接单态化，零运行时开销。

### 2. 数据表驱动 — `SqlType`

`src/db/sql_type.rs` 用 `const` 数组替代 match 分支：

```rust
pub enum SqlType { Varchar, Text, Integer, BigInt, Real, Boolean, Blob,
                   Timestamp, Date, Time, Decimal, Json }

impl SqlType {
    pub fn as_str(self) -> &'static str {
        TYPE_MAP[self as usize]       // 数组下标，编译时确定
    }
}

#[cfg(feature = "db-sqlite")]
const TYPE_MAP: [&str; 12] = row("TEXT", "TEXT", "INTEGER", "INTEGER", ...);

#[cfg(feature = "db-postgres")]
const TYPE_MAP: [&str; 12] = row("VARCHAR(255)", "TEXT", "INTEGER", "BIGINT", ...);

#[cfg(feature = "db-mysql")]
const TYPE_MAP: [&str; 12] = row("VARCHAR(255)", "TEXT", "INT", "BIGINT", ...);
```

加新数据库只需加一行 `const TYPE_MAP`。

### 3. 内部宏 — `define_db_types!`

`src/db/pool.rs` 用宏消除 7 × 3 = 21 个 cfg 块的重复：

```rust
macro_rules! define_db_types {
    ($db_type:path {
        Pool = $pool:ty, Transaction = $tx:ty, Row = $row:ty,
        Connection = $conn:ty, Arguments = $args:ty,
        QueryResult = $result:ty, PoolConnection = $pc:ty,
    }) => { /* 定义 7 个 type alias */ };
}

#[cfg(all(feature = "db-sqlite", not(...)))]
define_db_types!(sqlx::Sqlite { Pool = sqlx::SqlitePool, ... });
```

### 4. CRUD 宏系统

`raisfast-derive` 中的 `crud_find!` / `crud_update!` 等 166 处调用自动根据编译时
`DATABASE_URL` 检测 dialect，生成正确的 SQL。业务代码完全不需要改动。

### 5. `define_enum!` 子宏

`src/macros.rs` 中的 `define_enum!` 自动为枚举生成 sqlx 的 Type / Decode / Encode impl，
通过 `__define_enum_sqlx_impl!` 子宏参数化数据库类型。

---

## 添加新数据库支持（完整步骤）

以添加 **CockroachDB** 为例（与 PostgreSQL 高度兼容）。

### Step 1: Cargo.toml — 加 feature flag

```toml
[features]
db-cockroach = ["sqlx/postgres"]    # CockroachDB 使用 postgres 协议

# 互斥检查在 pool.rs 的 compile_error! 中处理
# 需要在 pool.rs 和各 cfg 条件中加上 db-cockroach
```

### Step 2: pool.rs — 加类型别名（7 行）

在现有 PostgreSQL 块后面加：

```rust
#[cfg(all(
    feature = "db-cockroach",
    not(any(feature = "db-sqlite", feature = "db-postgres", feature = "db-mysql"))
))]
define_db_types!(sqlx::Postgres {           // CockroachDB 用 postgres 驱动
    Pool = sqlx::PgPool,
    Transaction = sqlx::Transaction<'a, sqlx::Postgres>,
    Row = sqlx::postgres::PgRow,
    Connection = sqlx::postgres::PgConnection,
    Arguments = sqlx::postgres::PgArguments<'q>,
    QueryResult = sqlx::postgres::PgQueryResult,
    PoolConnection = sqlx::pool::PoolConnection<sqlx::Postgres>,
});
```

同时更新所有 `not(any(...))` 条件，加上 `feature = "db-cockroach"`。

### Step 3: driver.rs — 加 impl DbDriver（~100 行）

```rust
pub struct Cockroach;
impl sealed::Sealed for Cockroach {}

#[cfg(feature = "db-cockroach")]
pub type Driver = Cockroach;

#[cfg(feature = "db-cockroach")]
impl DbDriver for Cockroach {
    fn pk_type() -> &'static str { "BIGINT PRIMARY KEY" }
    fn fk_type() -> &'static str { "BIGINT" }
    fn ph(idx: usize) -> String { format!("${idx}") }
    fn now_fn() -> &'static str { "NOW()" }
    fn ago_expr(days: i64) -> String { format!("NOW() - INTERVAL '{days} days'") }
    fn date_trunc_day(col: &str) -> String { format!("DATE_TRUNC('day', {col}::timestamp)") }
    fn upsert_clause(conflict_cols: &str, assignments: &str) -> String {
        format!("ON CONFLICT({conflict_cols}) DO UPDATE SET {assignments}")
    }
    fn excluded_col(col: &str) -> String { format!("excluded.{col}") }
    fn returning_clause() -> &'static str { "RETURNING *" }
    fn returning_col(col: &str) -> String { format!("RETURNING {col}") }
    fn insert_ignore_sql(table: &str, columns: &str, phs: &str) -> String {
        assert!(is_safe_identifier(table), "unsafe table name: {table}");
        format!("INSERT INTO {table} ({columns}) VALUES ({phs}) ON CONFLICT DO NOTHING")
    }
    fn columns_sql(table: &str) -> (String, usize) { /* 同 PostgreSQL */ }
    fn column_names_sql(table: &str) -> (String, usize) { /* 同 PostgreSQL */ }
    fn rebuild_wrapper_sql(...) -> String { /* 同 PostgreSQL */ }
    async fn has_column(...) -> bool { /* 同 PostgreSQL */ }
    async fn table_exists(...) -> bool { /* 同 PostgreSQL */ }
    async fn list_user_tables(...) -> Vec<String> { /* 同 PostgreSQL */ }
    async fn fetch_columns_with_types(...) -> Result<...> { /* 同 PostgreSQL */ }
}
```

> **提示**：如果是与现有数据库兼容的（如 CockroachDB ↔ PostgreSQL），
> 直接复制对应的 impl 修改差异即可。

### Step 4: sql_type.rs — 加一行数据（1 行）

```rust
#[cfg(feature = "db-cockroach")]
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
```

### Step 5: macros.rs — 加一个 cfg 分支（5 行）

在 `__define_enum_sqlx!` 宏中加：

```rust
#[cfg(feature = "db-cockroach")]
$crate::__define_enum_sqlx_impl! {
    $name,
    db = sqlx::Postgres,
    type_info = sqlx::postgres::PgTypeInfo,
    value_ref = sqlx::postgres::PgValueRef<'_>,
    arg_buf = sqlx::postgres::PgArgumentBuffer,
}
```

### Step 6: schema.rs (proc-macro) — 加 Dialect 变体（5 行）

```rust
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
    Cockroach,           // 新增
}

// 在 const TABLE 中加：
DialectCfg {
    migration_dir: "cockroach",
    schema_ext: ".cockroach.sql",
    ph_prefix: "$",
},

// 在 from_env() 中加：
} else if url.starts_with("cockroach:") || url.starts_with("postgresql://...cockroach") {
    Dialect::Cockroach
```

### Step 7: connection.rs — 加连接初始化（~10 行）

```rust
#[cfg(feature = "db-cockroach")]
{
    use sqlx::pool::PoolOptions;
    let pool = PoolOptions::<sqlx::Postgres>::new()
        .max_connections(pool_size)
        .min_connections(1)
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .idle_timeout(Some(IDLE_TIMEOUT))
        .max_lifetime(Some(MAX_LIFETIME))
        .connect(database_url)
        .await?;
    tracing::info!(%pool_size, "cockroach connection pool initialized");
    Ok(pool)
}
```

### Step 8: migrations/ — 加 SQL 文件

```
migrations/cockroach/
  schema.cockroach.sql
  tenantable.cockroach.sql
```

### Step 9: config/app.rs — 加默认 URL（可选）

```rust
#[cfg(feature = "db-cockroach")]
fn default_db_url() -> String {
    "cockroach://root@localhost:26257/raisfast?sslmode=disable".to_string()
}
```

### Step 10: 编译验证

```bash
SQLX_OFFLINE=false DATABASE_URL="cockroach://..." \
  cargo clippy --tests --no-default-features --features "db-cockroach,plugin-js,plugin-lua,plugin-rhai" \
  -- -D warnings

SQLX_OFFLINE=false DATABASE_URL="cockroach://..." \
  cargo test --no-default-features --features "db-cockroach,plugin-js,plugin-lua,plugin-rhai"
```

---

## 改动清单汇总

| # | 文件 | 改动 | 行数 |
|---|------|------|------|
| 1 | `Cargo.toml` | feature flag + 依赖 | ~2 |
| 2 | `src/db/pool.rs` | `define_db_types!` 块 + 更新互斥条件 | ~10 |
| 3 | `src/db/driver.rs` | marker struct + `impl DbDriver` | ~100 |
| 4 | `src/db/sql_type.rs` | `const TYPE_MAP` 数据行 | ~15 |
| 5 | `src/macros.rs` | `__define_enum_sqlx!` cfg 分支 | ~5 |
| 6 | `raisfast-derive/src/schema.rs` | Dialect 变体 + DialectCfg + from_env | ~5 |
| 7 | `src/db/connection.rs` | 连接池初始化 | ~15 |
| 8 | `migrations/xxx/` | schema SQL 文件 | 1 文件 |
| 9 | `src/config/app.rs` | 默认 URL（可选） | ~3 |
| | **业务代码（48 个文件）** | **零改动** | **0** |

---

## 设计原则

1. **编译时保证** — `compile_error!` 确保恰好选一个后端，错误配置直接编译失败
2. **Sealed trait** — 外部 crate 无法实现 `DbDriver`，所有差异集中在 `src/db/`
3. **数据驱动** — `SqlType` 和 `Dialect` 用 const 数据表，加新条目无需理解逻辑
4. **零运行时开销** — 所有分支在编译时单态化，没有 `if cfg` 运行时判断
5. **CRUD 宏优先** — 166 处数据库调用通过宏自动适配，加新 DB 不改业务代码
6. **业务层零 cfg** — `#[cfg(feature = "db-xxx")]` 只出现在 `src/db/` 和 `src/macros.rs`
