# AGENTS.md

## Project

Blog system built with Rust + Axum + SQLite. Early stage — scaffold only, no implementation yet.

- **Crate name:** `rust-blog`
- **Rust edition:** 2024
- **Product & architecture spec:** `docs/guide.md`

## Commands

```bash
cargo build                          # compile
cargo test                           # run tests
cargo fmt --check                    # format check
cargo clippy -- -D warnings          # lint (zero warnings required)
```

Planned (once sqlx is wired):
```bash
cargo sqlx migrate run               # run DB migrations
cargo sqlx prepare                   # update sqlx-data.json for offline compile
```

## Architecture (target)

As described in `docs/guide.md`:

- **src/main.rs** — server entrypoint
- **src/handlers/** — axum route handlers (thin: extract params, call service, return response)
- **src/services/** — business logic layer
- **src/models/** — data structures and DB queries (sqlx)
- **src/middleware/** — JWT auth, rate limiting
- **src/errors/** — unified `AppError` (thiserror) implementing `IntoResponse`
- **src/config/** — env/config loading
- **migrations/** — sqlx SQL migration files

## Key Constraints

- **`unsafe` is banned.** Use `#![deny(unsafe_code)]` at crate root.
- **No `unwrap()` / `expect()`** in non-test code. Use `?` or explicit error handling.
- **Error handling pattern:** `thiserror` for `AppError` enum at handler boundaries; `anyhow` for internal service error propagation.
- **Database:** SQLite (via sqlx with compile-time query checking). All timestamps stored as TEXT in ISO 8601.
- **Primary keys:** UUID v7 (time-sortable).
- **Auth:** JWT (HS256) with short-lived access tokens + DB-stored refresh tokens.

## Style

- `cargo fmt` and `cargo clippy` are authoritative — no custom rustfmt/clippy config.
- Public items require `///` doc comments.
- Handler → Service → Model layering enforced; handlers must not contain business logic.
