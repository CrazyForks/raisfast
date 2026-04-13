# rust-blog 常用命令
#
# 用法: just <recipe>
# 帮助: just --list

set dotenv-load

db        := "sqlite"
db_url    := "sqlite:./data/blog.db"

# ── 默认 ──────────────────────────────────────────────────────────

default:
    @just --list

# ── 编译 ──────────────────────────────────────────────────────────

# 编译检查（默认 SQLite）
check *FLAGS:
    DATABASE_URL={{db_url}} cargo check --features "db-{{db}}" {{FLAGS}}

# 编译发布版本
build *FLAGS:
    DATABASE_URL={{db_url}} cargo build --release --features "db-{{db}}" {{FLAGS}}

# ── 代码质量 ──────────────────────────────────────────────────────

# 格式化
fmt:
    cargo fmt

# 格式化检查
fmt-check:
    cargo fmt --check

# Lint
lint:
    DATABASE_URL={{db_url}} cargo clippy --features "db-{{db}}" -- -D warnings

# 全部质量检查（fmt + lint）
qa: fmt-check lint

# ── 测试 ──────────────────────────────────────────────────────────

# 运行所有测试
test *FLAGS:
    DATABASE_URL={{db_url}} cargo test --features "db-{{db}}" {{FLAGS}}

# 仅运行单元测试
test-unit:
    DATABASE_URL={{db_url}} cargo test --lib --features "db-{{db}}"

# 仅运行集成测试
test-integration:
    DATABASE_URL={{db_url}} cargo test --test api_tests --features "db-{{db}}"

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
    DATABASE_URL={{db_url}} cargo sqlx prepare

# 验证离线编译（不依赖 DATABASE_URL）
check-offline:
    cargo check --features "db-{{db}}"

# ── 运行 ──────────────────────────────────────────────────────────

# 启动开发服务器
dev:
    DATABASE_URL={{db_url}} cargo run --features "db-{{db}}"

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

# ── 插件 ──────────────────────────────────────────────────────────

# 编译所有示例插件为 WASM 并复制到 plugins/ 目录
plugins-build:
    @echo "Building seo-optimizer..."
    cd plugins-examples/seo-optimizer && cargo build --target wasm32-unknown-unknown --release
    @echo "Building content-filter..."
    cd plugins-examples/content-filter && cargo build --target wasm32-unknown-unknown --release
    @mkdir -p plugins/seo-optimizer plugins/content-filter
    cp plugins-examples/seo-optimizer/target/wasm32-unknown-unknown/release/seo_optimizer.wasm plugins/seo-optimizer/
    cp plugins-examples/seo-optimizer/plugin.toml plugins/seo-optimizer/
    cp plugins-examples/content-filter/target/wasm32-unknown-unknown/release/content_filter.wasm plugins/content-filter/
    cp plugins-examples/content-filter/plugin.toml plugins/content-filter/
    @echo "Done. Plugins ready in plugins/"
