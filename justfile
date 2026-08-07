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
# Or just edit the defaults below.

db        := env_var_or_default("RAISFAST_DB", "mysql")

# Derive connection strings from `db` if not explicitly set.
default_db_url := if db == "sqlite" { "sqlite:storage/db/raisfast.db?mode=rwc" } \
                  else if db == "mysql" { "mysql://root:root@localhost:3306/raisfast" } \
                  else { "postgres://postgres:postgres@localhost:5432/raisfast" }

default_test_db_url := if db == "sqlite" { "sqlite::memory:" } \
                       else if db == "mysql" { "mysql://root:root@localhost:3306/raisfast_test" } \
                       else { "postgres://postgres:postgres@localhost:5432/raisfast_test" }

db_url        := env_var_or_default("RAISFAST_DB_URL", default_db_url)
test_db_url   := env_var_or_default("RAISFAST_TEST_DB_URL", default_test_db_url)

plugin_type := "all"

# ── Default ───────────────────────────────────────────────────────

default:
    @just --list

features := "db-" + db + " plugin-js plugin-rhai search-tantivy payment-all tunnel mcp cron-system"
features_csv := "db-" + db + ",plugin-js,plugin-rhai,search-tantivy,payment-all,tunnel,mcp,cron-system"

# ── Build ─────────────────────────────────────────────────────────

# Check compilation (default SQLite)
check *FLAGS:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo check --no-default-features --features "{{features}}" {{FLAGS}}

# Build release binary
build *FLAGS:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo build --release --no-default-features --features "{{features}}" {{FLAGS}}

# Build release binary with Admin UI (auto-detects frontend/admin)
build-full *FLAGS:
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
lint:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo clippy --tests --no-default-features --features "{{features}}" -- -D warnings

# Full quality check (fmt + lint)
qa: fmt-check lint

# ── Tests ─────────────────────────────────────────────────────────

# SQLite tests run in parallel; PostgreSQL/MySQL share a single test DB so
# they run serially to avoid DDL/catalog races during concurrent schema apply.
pg_or_mysql := if db == "sqlite" { "" } else { "-- --test-threads=1" }

# Run all tests
test *FLAGS:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test --no-default-features --features "{{features}}" {{FLAGS}} {{ pg_or_mysql }}

# Run unit tests only
test-unit:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test --lib --no-default-features --features "{{features}}" {{ pg_or_mysql }}

# Run integration tests only
test-integration:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} RAISFAST_TEST_DB_URL={{test_db_url}} cargo test --test api_tests --no-default-features --features "{{features}}" {{ pg_or_mysql }}

# ── Database Backend Switch ───────────────────────────────────────

# ── Database ──────────────────────────────────────────────────────

# Schema file path for the active database backend.
schema_file := if db == "sqlite" { "migrations/sqlite/schema.sqlite.sql" } else if db == "postgres" { "migrations/postgres/schema.postgres.sql" } else { "migrations/mysql/schema.mysql.sql" }

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
# Uses the test DB (same schema as dev DB, guaranteed fresh).
db-prepare:
    SQLX_OFFLINE=false DATABASE_URL={{test_db_url}} cargo sqlx prepare --workspace -- --tests --no-default-features --features "{{features}}"

# Recreate the test database (PostgreSQL/MySQL only; SQLite uses :memory:)
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

# Verify offline compilation (no DATABASE_URL required)
check-offline:
    SQLX_OFFLINE=true cargo check --no-default-features --features "{{features}}"

# ── Run ───────────────────────────────────────────────────────────

# Install cargo-watch (auto-recompile on file change)
install-watch:
    cargo install cargo-watch

# Start development server (online mode: validates SQL against live DB at compile time)
dev:
    SQLX_OFFLINE=false DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}"

# Start development server with auto-reload on code change.
# -w crates:         only watch source code (skip frontend/, .sqlx/, etc.)
# --delay 1s:         debounce rapid saves (Ctrl-S spam) into one rebuild
# --no-restart:       don't kill/restart if the binary exits on its own
# --ignore tests:     skip test files to avoid triggering on test edits
dev-watch:
    SQLX_OFFLINE=false DATABASE_URL={{db_url}} RUST_LOG=info cargo watch -w crates --delay 1s --no-restart -x "run --no-default-features --features {{features_csv}}"

# ── Database Backend Switch ───────────────────────────────────────

# Check compilation with PostgreSQL
pg-check:
    SQLX_OFFLINE=true cargo check --features "db-postgres"

# Check compilation with MySQL
mysql-check:
    SQLX_OFFLINE=true cargo check --features "db-mysql"

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
