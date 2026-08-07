# AGENTS.md

## Project

raisfast — Rust-powered high-performance BaaS and headless CMS. Single binary, zero dependencies, zero GC. Built-in blog, ecommerce, wallet, payment & multi-tenant SaaS. JS / Rhai / Lua / WASM plugin engines for infinite extensibility.

- **Crate name:** `raisfast`
- **Rust edition:** 2024
- **Architecture:** Handler → Service → Model three-layer
- **Plugin engines:** JS (QuickJS) / Rhai / Lua (mlua) / WASM (wasmtime)
- **Databases:** SQLite / PostgreSQL / MySQL (feature-gated)

## Commands

The default dev backend is PostgreSQL (`justfile` sets `db = "postgres"`).

```bash
# Compile/run (PostgreSQL + JS + Rhai + search + payment + mcp)
# The raisfast database must already contain the schema (see below).
SQLX_OFFLINE=false DATABASE_URL="postgres://postgres:postgres@localhost:5432/raisfast" \
  cargo clippy --tests --no-default-features \
  --features "db-postgres,plugin-js,plugin-rhai,search-tantivy,payment-all,tunnel,mcp" -- -D warnings

# Test (PostgreSQL)
SQLX_OFFLINE=false DATABASE_URL="postgres://postgres:postgres@localhost:5432/raisfast" \
  cargo test --no-default-features \
  --features "db-postgres,plugin-js,plugin-rhai,search-tantivy,payment-all,tunnel,mcp"

# Format check
cargo fmt --check
```

### Postgres first-run

sqlx proc-macros need live tables at compile time, and the binary embeds the
schema (`SCHEMA_SQL`) for first-run. After changing `migrations/postgres/schema.postgres.sql`,
load it into the DB, then build:

```bash
psql "postgres://postgres:postgres@localhost:5432/raisfast" \
  -f migrations/postgres/schema.postgres.sql
```

### SQLite commands (legacy path)

```bash
# NOTE: DATABASE_URL must be absolute — sqlx proc-macros run with CWD = crate
# root (crates/core/), so relative paths resolve wrongly. `$PWD` anchors to the
# workspace root when commands are run from there.
SQLX_OFFLINE=false DATABASE_URL="sqlite:$PWD/storage/db/raisfast.db?mode=rwc" \
  cargo clippy --tests --no-default-features \
  --features "db-sqlite,plugin-js,plugin-rhai" -- -D warnings

SQLX_OFFLINE=false DATABASE_URL="sqlite:$PWD/storage/db/raisfast.db?mode=rwc" \
  cargo test --no-default-features \
  --features "db-sqlite,plugin-js,plugin-rhai"
```

### Postgres type discipline

sqlx's `query!` macro type-checks binds strictly against Postgres column types
(SQLite/MySQL accept loose binds). `crud_insert!` auto-coerces BIGINT binds via
`crate::db::bigint::PgBigInt` (`SnowflakeId`/`i64`/`i32` and their `Option`s).
All other binds must match exactly:

- TEXT/VARCHAR → `&str`/`String`/`Option<&str>` (use `.as_deref()` on `Option<String>`)
- enum values (`define_enum!`) → `.as_str()`
- TIMESTAMPTZ → `DateTime<Utc>`/`Timestamp`; parse `&str` with `crate::utils::tz::parse_rfc3339*`
- BOOLEAN → `bool` (never `0`/`1` integers)

## Architecture

```
Handler → Service → Model (SQL)
                ↘ External: Storage / Cache / Search / EventBus
```

- **src/handlers/** — axum route handlers (thin: extract params, call service, return response)
  - Handler layer is the **only** auth entry point (`ensure_*` calls)
- **src/services/** — business logic layer
  - Service layer does **Policy only** (resource ownership checks), never calls `ensure_*`
- **src/models/** — data structures and DB queries (sqlx + CRUD macros)
  - Model provides `tx_*` variants for transaction participation
- **src/middleware/** — JWT auth, rate limiting
- **src/errors/** — unified `AppError` (thiserror) implementing `IntoResponse`
- **src/config/** — env/config loading
- **src/db/** — connection pool, SQL dialect, schema, write lock
- **src/plugins/** — 4-engine plugin system (JS/Rhai/Lua/WASM)
- **src/content_type/** — dynamic content type system
- **src/worker/** — job queue + cron scheduler (infrastructure, not model layer)

## Key Constraints

- **`unsafe` is banned.** `#![deny(unsafe_code)]` at crate root.
- **No `unwrap()` / `expect()`** in non-test code. Use `?` or explicit error handling.
- **Error handling:** `thiserror` for `AppError` at handler boundaries; `anyhow` for internal service propagation.
- **Database:** SQLite / PostgreSQL / MySQL via sqlx. Timestamps as `TIMESTAMPTZ` (PG/MySQL) or TEXT (SQLite). Always bind `DateTime<Utc>`, never strings.
- **Cross-DB portability is mandatory.** All code must run on SQLite, PostgreSQL, and MySQL. Every SQL statement — including hand-written `sqlx::query()`, macro-generated CRUD, and test fixtures — must consider cross-database compatibility. Default to `Driver::` helpers for dialect-specific syntax (`?` vs `$N`, `MAX(a,b)` vs `GREATEST`, `AUTOINCREMENT`, `STRFTIME`, etc.). When a `Driver::` helper doesn't exist or a fundamental difference can't be abstracted, use `#[cfg(feature = "db-sqlite")]` / `#[cfg(feature = "db-postgres")]` / `#[cfg(feature = "db-mysql")]` to handle the edge case explicitly.
- **Primary keys:** Snowflake ID (ferroid) with multiplicative inverse cipher + base62 encoding.
- **Auth:** JWT (HS256) with short-lived access tokens + DB-stored refresh tokens.
- **Write lock:** All transactions go through `acquire_write()` (tokio Mutex) to serialize SQLite writes and eliminate `SQLITE_BUSY` tail latency.

## CRUD Macro System

All DB operations use the Where DSL macro system (`raisfast-derive`):

- `crud_insert!`, `crud_update!`, `crud_delete!` — write operations
- `crud_find!`, `crud_find_one!`, `crud_find_all!` — read operations
- `crud_find_page!`, `crud_join_paged!` — pagination with JOINs
- `crud_resolve_id!`, `crud_resolve_ids!` — ID resolution
- `in_transaction!` — transaction wrapper (auto-acquires write lock)

## Cross-Database Development Rules

The project supports SQLite, PostgreSQL, and MySQL simultaneously. SQLite's "everything is TEXT" model and loose type checking hides type mismatches that PostgreSQL's strict typing will reject. Follow these rules to stay portable:

1. **Placeholders** — Use `Driver::ph(N)` for all SQL placeholders. Never hardcode `?` (SQLite/MySQL) or `$N` (PostgreSQL).
2. **Timestamps** — `.bind()` with `now_utc()` (`DateTime<Utc>`), never `now_str()` (returns `String`). PG rejects `text → TIMESTAMPTZ` implicit conversion.
3. **Aggregates** — Wrap `COUNT(*)` and `SUM(...)` in `Driver::cast_int(...)`. PG's `SUM(bigint)` returns `numeric`; MySQL returns `DECIMAL` — both fail to decode as `i64`.
4. **Booleans** — `.bind(true)`/`.bind(false)` for BOOLEAN columns; use `TRUE`/`FALSE` in SQL literals. Never `.bind(1_i64)` or `= 1` in SQL.
5. **Dialect functions** — Use `Driver::` helpers (`greatest()`, `now_fn()`, `date_trunc_day()`, `auto_increment_pk()`, `cast_int()`, `cast_ts()`, `like_op()`, `current_date()`). Never hand-write dialect-specific SQL keywords.
6. **WHERE clause placeholders** — When combining SET + WHERE (e.g., `db_update`, `db_increment`), WHERE clause placeholders must be offset by the number of SET parameters. Use `build_where_clause_with_offset(where_json, set_param_count)`.
7. **Test data uniqueness** — Always use `crate::utils::id::new_id()` for usernames, slugs, names, etc. in test inserts. Never use fixed strings like `"testuser"` — they collide on shared databases.
8. **Test INSERTs must include `id`** — `BIGINT PRIMARY KEY` does not auto-increment on PostgreSQL/MySQL (unlike SQLite's `INTEGER PRIMARY KEY`). Always bind an explicit `id` from `new_id()`.
9. **Test assertions** — Never assert `COUNT(*) == N` on a shared database (accumulated data from prior tests inflates the count). Filter by unique IDs or use `>=` comparisons.
10. **Returning clause** — MySQL does not support `RETURNING`. Use `rows_affected()` instead of `RETURNING id` for cross-database claim/dedup patterns.

## Style

- `cargo fmt` and `cargo clippy` are authoritative.
- Public items require `///` doc comments.
- Handler → Service → Model layering enforced.

## Frontend Rules

- **Always use official shadcn (Base UI) components first. Never hand-build components with the same name as shadcn.**
  - Install: `npx shadcn@latest add <component>` (run in `frontend/admin/`)
  - For custom styling, pass `className` at the call site — never modify official source in `components/ui/`
  - If the official component doesn't fit, create a new component with a different name — never overwrite the official file
