# raisfast common commands
#
# Usage: just <recipe>
# Help:  just --list

set dotenv-load

#db        := "sqlite"
#db_url    := "sqlite:./storage/db/raisfast.db?mode=rwc"
db        := "mysql"
db_url    := "mysql://root:root@localhost:3306/raisfast"
plugin_type := "all"

# ── Default ───────────────────────────────────────────────────────

default:
    @just --list

features := "db-" + db + " plugin-js plugin-rhai search-tantivy payment-all"

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

# Lint
lint:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo clippy --no-default-features --features "{{features}}" -- -D warnings

# Full quality check (fmt + lint)
qa: fmt-check lint

# ── Tests ─────────────────────────────────────────────────────────

# Run all tests
test *FLAGS:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo test --no-default-features --features "{{features}}" {{FLAGS}}

# Run unit tests only
test-unit:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo test --lib --no-default-features --features "{{features}}"

# Run integration tests only
test-integration:
    SQLX_OFFLINE=true DATABASE_URL={{db_url}} cargo test --test api_tests --no-default-features --features "{{features}}"

# ── Database ──────────────────────────────────────────────────────

# Create SQLite database and run migrations
db-init:
    mkdir -p data
    sqlite3 {{db_url}} < migrations/001_init.sql
    sqlite3 {{db_url}} < migrations/002_add_indexes.sql

# Recreate database (dangerous: deletes existing data)
db-reset:
    rm -f data/blog.db
    just db-init

# Run CLI migrations
db-migrate:
    DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}" -- db migrate

# Backup database
db-backup:
    DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}" -- db backup ./backups

# Generate sqlx offline query metadata
db-prepare:
    DATABASE_URL={{db_url}} cargo sqlx prepare -- --no-default-features --features "{{features}}"

# Verify offline compilation (no DATABASE_URL required)
check-offline:
    SQLX_OFFLINE=true cargo check --no-default-features --features "{{features}}"

# ── Run ───────────────────────────────────────────────────────────

# Start development server
dev:
    DATABASE_URL={{db_url}} cargo run --no-default-features --features "{{features}}"

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
