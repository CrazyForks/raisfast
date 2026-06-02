# raisfast common commands
#
# Usage: just <recipe>
# Help:  just --list

set dotenv-load

db        := "sqlite"
db_url    := "sqlite:./storage/db/raisfast.db?mode=rwc"
plugin_type := "all"

# ── Default ───────────────────────────────────────────────────────

default:
    @just --list

features := "db-" + db + " plugin-js plugin-rhai search-tantivy"

# ── Build ─────────────────────────────────────────────────────────

# Check compilation (default SQLite)
check *FLAGS:
    DATABASE_URL={{db_url}} cargo check --features "{{features}}" {{FLAGS}}

# Build release binary
build *FLAGS:
    DATABASE_URL={{db_url}} cargo build --release --features "{{features}}" {{FLAGS}}

# Build release binary with Admin UI (auto-detects frontend/admin)
build-full *FLAGS:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -d "frontend/admin" ]; then
        echo ">> Building Admin UI from source..."
        cd frontend && pnpm install --frozen-lockfile
        cd admin && pnpm build
        cd ../..
        rm -rf adminui
        cp -r frontend/admin/dist adminui
        echo ">> Admin UI built and copied to adminui/"
    else
        echo ">> frontend/admin not found, using existing adminui/ as-is"
    fi
    DATABASE_URL={{db_url}} cargo build --release --features "{{features}}" {{FLAGS}}

# ── Code Quality ──────────────────────────────────────────────────

# Format code
fmt:
    cargo fmt

# Check formatting
fmt-check:
    cargo fmt --check

# Lint
lint:
    DATABASE_URL={{db_url}} cargo clippy --features "{{features}}" -- -D warnings

# Full quality check (fmt + lint)
qa: fmt-check lint

# ── Tests ─────────────────────────────────────────────────────────

# Run all tests
test *FLAGS:
    DATABASE_URL={{db_url}} cargo test --features "{{features}}" {{FLAGS}}

# Run unit tests only
test-unit:
    DATABASE_URL={{db_url}} cargo test --lib --features "{{features}}"

# Run integration tests only
test-integration:
    DATABASE_URL={{db_url}} cargo test --test api_tests --features "{{features}}"

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
    DATABASE_URL={{db_url}} cargo run -- db migrate

# Backup database
db-backup:
    DATABASE_URL={{db_url}} cargo run -- db backup ./backups

# Generate sqlx offline query metadata
db-prepare:
    DATABASE_URL={{db_url}} cargo sqlx prepare -- --features "{{features}}"

# Verify offline compilation (no DATABASE_URL required)
check-offline:
    cargo check --features "{{features}}"

# ── Run ───────────────────────────────────────────────────────────

# Start development server
dev:
    DATABASE_URL={{db_url}} cargo run --features "{{features}}"

# ── Database Backend Switch ───────────────────────────────────────

# Check compilation with PostgreSQL
pg-check:
    cargo check --features "db-postgres"

# Check compilation with MySQL
mysql-check:
    cargo check --features "db-mysql"

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
