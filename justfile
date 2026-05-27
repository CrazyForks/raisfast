# raisfast 常用命令
#
# 用法: just <recipe>
# 帮助: just --list

set dotenv-load

db        := "sqlite"
db_url    := "sqlite:./storage/db/raisfast.db?mode=rwc"
plugin_type := "all"

# ── 默认 ──────────────────────────────────────────────────────────

default:
    @just --list

features := "db-" + db + " plugin-js plugin-rhai search-tantivy"

# ── 编译 ──────────────────────────────────────────────────────────

# 编译检查（默认 SQLite）
check *FLAGS:
    DATABASE_URL={{db_url}} cargo check --features "{{features}}" {{FLAGS}}

# 编译发布版本
build *FLAGS:
    DATABASE_URL={{db_url}} cargo build --release --features "{{features}}" {{FLAGS}}

# 编译发布版本（含 Admin UI）
build-full *FLAGS:
    cd frontend && pnpm install --frozen-lockfile
    cd frontend/admin && pnpm build
    DATABASE_URL={{db_url}} cargo build --release --features "{{features}}" {{FLAGS}}

# ── 代码质量 ──────────────────────────────────────────────────────

# 格式化
fmt:
    cargo fmt

# 格式化检查
fmt-check:
    cargo fmt --check

# Lint
lint:
    DATABASE_URL={{db_url}} cargo clippy --features "{{features}}" -- -D warnings

# 全部质量检查（fmt + lint）
qa: fmt-check lint

# ── 测试 ──────────────────────────────────────────────────────────

# 运行所有测试
test *FLAGS:
    DATABASE_URL={{db_url}} cargo test --features "{{features}}" {{FLAGS}}

# 仅运行单元测试
test-unit:
    DATABASE_URL={{db_url}} cargo test --lib --features "{{features}}"

# 仅运行集成测试
test-integration:
    DATABASE_URL={{db_url}} cargo test --test api_tests --features "{{features}}"

# ── 数据库 ────────────────────────────────────────────────────────

# 创建 SQLite 数据库并运行迁移
db-init:
    mkdir -p data
    sqlite3 {{db_url}} < migrations/001_init.sql
    sqlite3 {{db_url}} < migrations/002_add_indexes.sql

# 重新创建数据库（危险：删除现有数据）
db-reset:
    rm -f data/blog.db
    just db-init

# 运行 CLI 迁移
db-migrate:
    DATABASE_URL={{db_url}} cargo run -- db migrate

# 备份数据库
db-backup:
    DATABASE_URL={{db_url}} cargo run -- db backup ./backups

# 生成 sqlx 离线查询元数据
db-prepare:
    DATABASE_URL={{db_url}} cargo sqlx prepare -- --features "{{features}}"

# 验证离线编译（不依赖 DATABASE_URL）
check-offline:
    cargo check --features "{{features}}"

# ── 运行 ──────────────────────────────────────────────────────────

# 启动开发服务器
dev:
    DATABASE_URL={{db_url}} cargo run --features "{{features}}"

# ── 数据库后端切换 ────────────────────────────────────────────────

# 用 PostgreSQL 编译检查
pg-check:
    cargo check --features "db-postgres"

# 用 MySQL 编译检查
mysql-check:
    cargo check --features "db-mysql"

# ── 完整 CI 流水线 ────────────────────────────────────────────────

# CI: fmt → lint → test（确保所有检查通过）
ci: fmt-check lint test

# ── 部署 ──────────────────────────────────────────────────────────

fly_target := "x86_64-unknown-linux-musl"
fly_image := "raisfast-fly"

# 安装 cross（Rust 交叉编译工具）
install-cross:
    cargo install cross --git https://github.com/cross-rs/cross

# 交叉编译 Linux 二进制并部署到 fly.io
build-cross:
    @echo "Cross-compiling for Linux via cross..."
    cross build --release --features "{{features}}" --target {{fly_target}}

# 使用已编译的二进制直接部署到 fly.io（跳过编译）
deploy-fly:
    @echo "Building Docker image..."
    docker build --platform linux/amd64 -t {{fly_image}} -f deploy/fly/Dockerfile .
    @echo "Deploying to fly.io..."
    fly deploy --local-only -c deploy/fly/fly.toml --image {{fly_image}}
