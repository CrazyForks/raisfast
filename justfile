# rust-blog 常用命令
#
# 用法: just <recipe>
# 帮助: just --list

set dotenv-load

db        := "sqlite"
db_url    := "sqlite:./data/blog.db"
plugin_type := "all"

# ── 默认 ──────────────────────────────────────────────────────────

default:
    @just --list

# ── 编译 ──────────────────────────────────────────────────────────

# 编译检查（默认 SQLite）
check *FLAGS:
    DATABASE_URL={{db_url}} cargo check --features "db-{{db}} plugin-{{plugin_type}}" {{FLAGS}}

# 编译发布版本
build *FLAGS:
    DATABASE_URL={{db_url}} cargo build --release --features "db-{{db}} plugin-{{plugin_type}}" {{FLAGS}}

# ── 代码质量 ──────────────────────────────────────────────────────

# 格式化
fmt:
    cargo fmt

# 格式化检查
fmt-check:
    cargo fmt --check

# Lint
lint:
    DATABASE_URL={{db_url}} cargo clippy --features "db-{{db}} plugin-{{plugin_type}}" -- -D warnings

# Lint（含 Tantivy 搜索）
lint-search:
    DATABASE_URL={{db_url}} cargo clippy --features "db-{{db}} plugin-{{plugin_type}} search-tantivy" -- -D warnings

# 全部质量检查（fmt + lint）
qa: fmt-check lint

# ── 测试 ──────────────────────────────────────────────────────────

# 运行所有测试
test *FLAGS:
    DATABASE_URL={{db_url}} cargo test --features "db-{{db}} plugin-{{plugin_type}}" {{FLAGS}}

# 运行所有测试（含 Tantivy 搜索）
test-search *FLAGS:
    DATABASE_URL={{db_url}} cargo test --features "db-{{db}} plugin-{{plugin_type}} search-tantivy" {{FLAGS}}

# 仅运行单元测试
test-unit:
    DATABASE_URL={{db_url}} cargo test --lib --features "db-{{db}} plugin-{{plugin_type}}"

# 仅运行集成测试
test-integration:
    DATABASE_URL={{db_url}} cargo test --test api_tests --features "db-{{db}} plugin-{{plugin_type}}"

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
    DATABASE_URL={{db_url}} cargo sqlx prepare --features "db-{{db}} plugin-{{plugin_type}}"

# 验证离线编译（不依赖 DATABASE_URL）
check-offline:
    cargo check --features "db-{{db}} plugin-{{plugin_type}}"

# ── 运行 ──────────────────────────────────────────────────────────

# 启动开发服务器
dev:
    DATABASE_URL={{db_url}} cargo run --features "db-{{db}} plugin-{{plugin_type}}"

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

# 编译所有示例 WASM 插件并复制到 plugins/ 目录
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
    @echo "Done. WASM plugins ready in plugins/"

# 复制 JS 插件到 plugins/ 目录
plugins-js-build:
    @echo "Copying JS plugins..."
    @mkdir -p plugins/welcome-email
    @cp plugins-examples-js/welcome-email/plugin.toml plugins/welcome-email/
    @cp plugins-examples-js/welcome-email/index.js plugins/welcome-email/
    @mkdir -p plugins/seo-optimizer-js
    @cp plugins-examples-js/seo-optimizer-js/plugin.toml plugins/seo-optimizer-js/
    @cp plugins-examples-js/seo-optimizer-js/index.js plugins/seo-optimizer-js/
    @echo "Done. JS plugins ready in plugins/"

# 编译 TypeScript JS 插件（需要 esbuild）
plugins-ts-build:
    @echo "Compiling TypeScript plugins..."
    @npx esbuild plugins-examples-js/seo-optimizer-js/src/index.ts \
        --outfile=plugins-examples-js/seo-optimizer-js/index.js \
        --bundle --format=iife --target=es2021
    @echo "Done."

# 扫描 plugins-examples-lua/ 下所有含 plugin.toml 的子目录，复制到 plugins/
# 扫描 plugins-examples-lua/ 下含 plugin.toml 的子目录，复制到 plugins/
plugins-lua-build:
    #!/usr/bin/env bash
    echo "Scanning plugins-examples-lua/..."
    count=0
    for dir in plugins-examples-lua/*/; do
        if [ -f "${dir}plugin.toml" ]; then
            name=$(basename "$dir")
            echo "  $name"
            mkdir -p "plugins/$name"
            cp -r "$dir"* "plugins/$name/"
            count=$((count + 1))
        else
            echo "  Skipping $(basename "$dir"): missing plugin.toml"
        fi
    done
    echo "Done. $count Lua plugin(s) copied to plugins/"

# 编译/复制所有插件（WASM + JS + Lua）
plugins-all: plugins-build plugins-js-build plugins-lua-build
