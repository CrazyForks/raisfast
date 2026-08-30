# raisfast common commands
#
# Usage: just <recipe>
# Help:  just --list

set dotenv-load

# ── Database Backend Configuration ────────────────────────────────
#
# Override defaults via environment variables:
#   RAISFAST_DB         — "sqlite" | "postgres" | "mysql"
#   RAISFAST_DB_URL     — connection string for dev DB
#   RAISFAST_TEST_DB_URL — connection string for test DB
#
# Or just edit the default below.
#
# Switch backend:  RAISFAST_DB=mysql just test-all

db        := env_var_or_default("RAISFAST_DB", "sqlite")

# Derive connection strings from `db` if not explicitly set.
default_db_url := if db == "sqlite" { "sqlite:storage/db/raisfast.db?mode=rwc" } \
                  else if db == "mysql" { "mysql://root:root@localhost:3306/raisfast" } \
                  else { "postgres://postgres:postgres@localhost:5432/raisfast" }

default_test_db_url := if db == "sqlite" { "sqlite::memory:" } \
                       else if db == "mysql" { "mysql://root:root@localhost:3306/raisfast_test" } \
                       else { "postgres://postgres:postgres@localhost:5432/raisfast_test" }

db_url        := env_var_or_default("RAISFAST_DB_URL", default_db_url)
test_db_url   := env_var_or_default("RAISFAST_TEST_DB_URL", default_test_db_url)

# Per-backend sqlx offline cache directory (avoids cross-contamination).
# `cargo sqlx prepare` writes to .sqlx/, so we keep per-backend copies in
# .sqlx-{db}/ and symlink .sqlx → .sqlx-{db} before each compile.
sqlx_cache := ".sqlx-" + db

# Schema file path for the active database backend.
schema_file := if db == "sqlite" { "migrations/sqlite/schema.sqlite.sql" } else if db == "postgres" { "migrations/postgres/schema.postgres.sql" } else { "migrations/mysql/schema.mysql.sql" }

# Feature flags derived from backend.
features     := "db-" + db + " plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system integration-stream integration-imap"
features_csv := "db-" + db + ",plugin-js,plugin-rhai,search-tantivy,payment-all,tunnel,mcp,cron-system,integration-stream,integration-imap"

# Tests run in parallel on SQLite; PG/MySQL share a single test DB so they run
# serially to avoid DDL/catalog races during concurrent schema apply.
test_threads := if db == "sqlite" { "" } else { "-- --test-threads=1" }

# ── Default ───────────────────────────────────────────────────────

default:
    @just --list

# ── sqlx cache symlink helper ─────────────────────────────────────

# Link .sqlx → .sqlx-{db} so sqlx macros read the correct backend cache.
# Called automatically before every compile/test recipe.
_link-cache:
    #!/usr/bin/env bash
    set -euo pipefail
    CACHE="{{sqlx_cache}}"
    if [ -d "$CACHE" ]; then
        rm -rf .sqlx
        ln -s "$CACHE" .sqlx
    elif [ -d .sqlx ] && [ ! -L .sqlx ]; then
        echo ">> WARNING: .sqlx exists but $CACHE does not. Run: just db-prepare"
    fi

# ── Build ─────────────────────────────────────────────────────────

# Check compilation
check *FLAGS: _link-cache
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo check --no-default-features --features "{{features}}" {{FLAGS}}

# Build release binary
build *FLAGS: _link-cache
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo build --release --no-default-features --features "{{features}}" {{FLAGS}}

# Build release binary with Admin UI (auto-detects frontend/admin)
build-full *FLAGS: _link-cache
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d "frontend/admin" ]; then
        echo ">> Building Admin UI from source..."
        cd frontend/admin && npm ci
        npm run build
        cd ../..
        rm -rf adminui
        cp -r frontend/admin/dist adminui
        echo ">> Admin UI built and copied to adminui/"
    else
        echo ">> frontend/admin not found, using existing adminui/ as-is"
    fi
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo build --release --no-default-features --features "{{features}}" {{FLAGS}}

# ── Code Quality ──────────────────────────────────────────────────

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt --check

# Lint (including tests)
lint: _link-cache
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo clippy --tests --no-default-features --features "{{features}}" -- -D warnings

# Full quality check (fmt + lint)
qa: fmt-check lint

# ── Tests ─────────────────────────────────────────────────────────

# Run all raisfast tests (excludes bore-cli e2e tests that need a bore server)
# ID_ENCODING=false keeps tests hermetic: `set dotenv-load` exports the dev
# server's .env (ID_ENCODING=true) into recipes, which breaks tests that pass
# plain-digit ids through `parse_id`.
test *FLAGS: _link-cache
    SQLX_OFFLINE=true ID_ENCODING=false DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test -p raisfast --no-default-features --features "{{features}}" {{FLAGS}} {{test_threads}}

# Run unit tests only
test-unit: _link-cache
    SQLX_OFFLINE=true ID_ENCODING=false DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test -p raisfast --lib --no-default-features --features "{{features}}" {{test_threads}}

# Run integration tests only
test-integration: _link-cache
    SQLX_OFFLINE=true ID_ENCODING=false DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test -p raisfast --test api --no-default-features --features "{{features}}" {{test_threads}}

# ── Database ──────────────────────────────────────────────────────

# Create database (if needed) and load schema
db-init:
    #!/usr/bin/env bash
    set -euo pipefail
    DB="{{db}}"
    DB_URL="{{db_url}}"
    SCHEMA="{{schema_file}}"
    if [ "$DB" = "sqlite" ]; then
        mkdir -p storage/db
        sqlite3 "$(echo "$DB_URL" | sed 's/sqlite://;s/?.*//')" < "$SCHEMA"
    elif [ "$DB" = "postgres" ]; then
        psql "postgresql://postgres:postgres@localhost:5432/postgres" -tc "SELECT 1 FROM pg_database WHERE datname = 'raisfast'" | grep -q 1 || \
            psql "postgresql://postgres:postgres@localhost:5432/postgres" -c "CREATE DATABASE raisfast"
        psql "$DB_URL" -f "$SCHEMA"
    else
        mysql -u root -proot -e "CREATE DATABASE IF NOT EXISTS raisfast"
        mysql -u root -proot raisfast < "$SCHEMA"
    fi
    echo ">> Database initialized ($DB)"

# Recreate database (dangerous: deletes existing data)
db-reset:
    #!/usr/bin/env bash
    set -euo pipefail
    DB="{{db}}"
    DB_URL="{{db_url}}"
    if [ "$DB" = "sqlite" ]; then
        rm -f "$(echo "$DB_URL" | sed 's/sqlite://;s/?.*//')"
    elif [ "$DB" = "postgres" ]; then
        psql "postgresql://postgres:postgres@localhost:5432/postgres" -c "DROP DATABASE IF EXISTS raisfast"
    else
        mysql -u root -proot -e "DROP DATABASE IF EXISTS raisfast"
    fi
    just db-init

# Run CLI migrations
db-migrate:
    DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}" -- db migrate

# Backup database
db-backup:
    DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}" -- db backup ./backups

# Generate sqlx offline query metadata for production AND test code.
# cargo sqlx prepare writes to .sqlx/, then we move it to .sqlx-{db}/.
db-prepare:
    #!/usr/bin/env bash
    set -euo pipefail
    DB="{{db}}"
    TEST_URL="{{test_db_url}}"
    SCHEMA="{{schema_file}}"
    CACHE_DIR="{{sqlx_cache}}"
    rm -rf .sqlx
    rm -rf "$CACHE_DIR"
    if [ "$DB" = "sqlite" ]; then
        TMPDB="$(mktemp -d)/raisfast_prepare.db"
        sqlite3 "$TMPDB" < "$SCHEMA"
        SQLX_OFFLINE=false DATABASE_URL="sqlite:$TMPDB" cargo sqlx prepare --workspace -- --tests --no-default-features --features "{{features}}"
        rm -rf "$(dirname "$TMPDB")"
    else
        SQLX_OFFLINE=false DATABASE_URL="$TEST_URL" cargo sqlx prepare --workspace -- --tests --no-default-features --features "{{features}}"
    fi
    mv .sqlx "$CACHE_DIR"
    ln -sf "$CACHE_DIR" .sqlx
    echo ">> sqlx cache written to $CACHE_DIR ($(ls "$CACHE_DIR" | wc -l | tr -d ' ') queries)"

# Recreate the test database (SQLite uses :memory:, no setup needed)
test-db-init:
    #!/usr/bin/env bash
    set -euo pipefail
    DB="{{db}}"
    TEST_URL="{{test_db_url}}"
    SCHEMA="{{schema_file}}"
    if [ "$DB" = "sqlite" ]; then
        echo "SQLite uses :memory: — no test DB setup needed"
    elif [ "$DB" = "postgres" ]; then
        psql "postgresql://postgres:postgres@localhost:5432/postgres" -c "DROP DATABASE IF EXISTS raisfast_test"
        psql "postgresql://postgres:postgres@localhost:5432/postgres" -c "CREATE DATABASE raisfast_test"
        psql "$TEST_URL" -f "$SCHEMA"
    else
        mysql -u root -proot -e "DROP DATABASE IF EXISTS raisfast_test"
        mysql -u root -proot -e "CREATE DATABASE raisfast_test"
        mysql -u root -proot raisfast_test < "$SCHEMA"
    fi

# ── Run ───────────────────────────────────────────────────────────

# Install cargo-watch (auto-recompile on file change)
install-watch:
    cargo install cargo-watch

# Start development server (online mode: validates SQL against live DB at compile time)
dev:
    SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}"

# Start development server with auto-reload on code change.
# -w crates:   only watch source code (skip frontend/, .sqlx-*/, etc.)
# --delay 1s:  debounce rapid saves into one rebuild
# --no-restart: don't kill/restart if the binary exits on its own
dev-watch:
    SQLX_OFFLINE=false DATABASE_URL={{db_url}} RUST_LOG=info cargo watch -w crates --delay 1s --no-restart -x "run --no-default-features --features {{features_csv}}"

# ── Backend-Specific Checks ───────────────────────────────────────

# Check compilation with PostgreSQL
pg-check:
    RAISFAST_DB=postgres just check

# Check compilation with MySQL
mysql-check:
    RAISFAST_DB=mysql just check

# Check compilation with SQLite
sqlite-check:
    RAISFAST_DB=sqlite just check

# ── Full CI Pipeline ──────────────────────────────────────────────

# CI: fmt → lint → test (ensure all checks pass)
ci: fmt-check lint test

# One-shot: reset test DB → regenerate sqlx cache → run all tests
test-all: test-db-init db-prepare test

# ── Deploy ────────────────────────────────────────────────────────

fly_target := "x86_64-unknown-linux-musl"
fly_image := "raisfast-fly"

# Install cross (Rust cross-compilation tool)
install-cross:
    cargo install cross --git https://github.com/cross-rs/cross

# Cross-compile Linux binary for fly.io
build-cross:
    @echo "Cross-compiling for Linux via cross..."
    cross build --release --features "{{features}}" --target {{fly_target}}

# Deploy pre-built binary to fly.io (skip compilation)
deploy-fly:
    @echo "Building Docker image..."
    docker build --platform linux/amd64 -t {{fly_image}} -f deploy/fly/Dockerfile .
    @echo "Deploying to fly.io..."
    fly deploy --local-only -c deploy/fly/fly.toml --image {{fly_image}}
