<div align="center">
  <a href="./README.md">English</a> | <a href="./README_CN.md">简体中文</a>
</div>

---

<p align="center">
  <img src="https://www.raisfast.com/icon.svg" alt="RaisFast" width="140" />
</p>

<h1 align="center">RaisFast — The fastest CMS, easiest to deploy</h1>

<p align="center">
  <strong>最快的 CMS，最简单的部署。</strong>
  <br />
  Rust 驱动的高性能 BaaS 与 headless CMS · 内置博客 / 电商 / 钱包 / 支付 / 工作流 / 多租户 SaaS
  <br />
  JS / Rhai / Lua / WASM 四引擎插件 · MCP 原生 · 单二进制 · 零 GC
  <br />
  <a href="https://raisfast.com">官网</a> ·
  <a href="https://raisfast.com/docs">文档</a> ·
  <a href="https://github.com/RaisFast/raisfast/releases">下载</a> ·
  <a href="https://github.com/RaisFast/raisfast/discussions">讨论</a> ·
  <a href="#快速开始">快速开始</a>
</p>

<p align="center">
  <a href="https://github.com/RaisFast/raisfast/stargazers"><img src="https://img.shields.io/github/stars/RaisFast/raisfast?style=social" alt="GitHub Stars" /></a>
  <a href="https://github.com/RaisFast/raisfast/releases"><img src="https://img.shields.io/github/v/release/RaisFast/raisfast?color=blue" alt="Latest Release" /></a>
  <a href="https://github.com/RaisFast/raisfast/actions/workflows/ci.yml"><img src="https://github.com/RaisFast/raisfast/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
  <a href="https://github.com/RaisFast/raisfast"><img src="https://img.shields.io/badge/Rust-edition%202024-orange" alt="Rust Edition 2024" /></a>
  <a href="./LICENSE"><img src="https://img.shields.io/github/license/RaisFast/raisfast?color=success" alt="License" /></a>
  <a href="https://github.com/RaisFast/raisfast"><img src="https://img.shields.io/badge/platform-macOS%20%C2%B7%20Linux%20%C2%B7%20Windows-blue" alt="Platforms" /></a>
</p>

---

> **早期 Alpha 阶段 — v1.0 前 API 可能变更。**
> 稳定版 v1.0 计划于 2026 年 Q3 发布。

---

## 为什么选 raisfast？

**单文件，全能力**
一个二进制，无需 Node.js、无需 Docker、无需运行时。
博客、电商、钱包、支付从数据库到 API 原生内置——不是插件拼装，是骨骼。

**Rust 性能，零 GC 稳定**
读延迟亚毫秒，长时间运行性能零退化。没有 GC 停顿，没有内存泄漏，没有凌晨三点的 OOM 告警。

**四套插件引擎，取 Strapi 之长**
JS、Rhai、Lua、WASM 四层扩展，覆盖从脚本到编译型的完整光谱。享受动态语言的开发效率，拥有 Rust 的性能基座。

**为 AI 而生**
内置 MCP（Model Context Protocol）服务器，Claude 等 AI 客户端可直接读写内容、调用工具。

---

## 内置功能

| 模块 | 功能 |
|------|------|
| **博客 / CMS** | 文章、页面、分类、标签、评论、媒体、RSS、站点地图 |
| **电商** | 购物车、订单、商品变体、优惠券、运费模板 |
| **钱包与支付** | 多币种钱包；支付宝 / 微信支付 / Stripe / Dodo / Creem，统一支付路由 |
| **工作流** | 工作流引擎 + 任务队列 + Cron 调度 + 事件总线 |
| **内容类型** | TOML 定义动态 Schema、分组、批量操作、规则引擎，自动生成 CRUD API |
| **认证授权** | JWT (HS256) + Refresh Token + API Token + 多角色 RBAC |
| **OAuth** | GitHub / Google / 微信 等社交登录 |
| **MCP** | Model Context Protocol 服务器（Streamable HTTP + stdio），对接 AI 客户端 |
| **GraphQL** | 可选 GraphQL API（async-graphql） |
| **Webhook** | 事件驱动的 Webhook 投递系统 |
| **通知** | 邮件（SMTP）/ SMS 通知 |
| **多租户** | 可选租户隔离 + 内置反向代理，支撑 SaaS 场景 |
| **插件引擎** | JS (QuickJS) / Rhai / Lua (mlua) / WASM (wasmtime)，虚拟文件系统、热加载 |
| **搜索** | 全文搜索（Tantivy） |
| **管理后台** | React 19 + shadcn/ui 仪表盘（嵌入二进制，零配置） |
| **桌面端** | Tauri 封装，跨平台桌面应用 |
| **隧道** | 内置 tunnel 客户端，一键暴露本地端口到公网 |
| **多数据库** | SQLite / PostgreSQL / MySQL 零改动切换 |
| **可观测性** | 结构化日志、Prometheus 指标、请求 ID、审计日志、Panic Webhook |

---

## 快速开始

```bash
# 克隆
git clone https://github.com/raisfast/raisfast.git
cd raisfast

# 准备配置（按需修改 .env）
cp .env.example .env

# 编译运行（使用默认 feature：SQLite + 搜索 + JS/Lua/Rhai + OpenAPI + Proxy + Tunnel + MCP）
cargo run

# 服务默认监听 http://localhost:9898
# 管理后台在  /admin
# Swagger 在  /swagger-ui
# 健康检查    /api/v1/healthz
```

> 想要全量能力（追加 WASM 引擎、支付、S3 存储）？
> ```bash
> cargo run --features "plugin-wasm payment-all storage-s3"
> ```

### 首次启动

首次启动时 raisfast 会自动：
1. 创建所有数据库表（嵌入 `SCHEMA_SQL`）
2. 初始化默认角色、权限和站点配置
3. 启动 API + 管理后台

创建管理员账户（CLI 子命令）：

```bash
cargo run -- user create --email admin@example.com --username admin --password your-password --role admin
```

### 用 just（推荐工作流）

仓库内置 `justfile`，封装了常用的构建 / 测试 / 数据库命令：

```bash
just dev          # 在线模式开发（SQL 编译期校验）
just build        # 发布构建
just qa           # fmt 检查 + clippy
just test         # 跑全部测试
just db-init      # 建库并载入 schema
just db-migrate   # 执行迁移
```

切换后端只需改一个环境变量：`RAISFAST_DB=postgres just test`（默认 `postgres`，可选 `sqlite` / `mysql`）。

### Docker 部署

单容器（SQLite 本地存储）：

```bash
docker build -t raisfast .
docker run -p 9898:9898 -v ./data:/app/data raisfast
```

完整栈（`docker-compose.yml`，含 RustFS / S3 兼容对象存储）：

```bash
docker compose up -d
# backend  → http://localhost:9898
# frontend → http://localhost:3000
# rustfs (S3) → http://localhost:9001
```

---

## 架构

raisfast 是一个 Cargo workspace：

```
raisfast/
├── crates/
│   ├── core/           # 主程序 crate（raisfast）：业务与服务实现
│   └── derive/         # 过程宏 crate（raisfast-derive）：CRUD / Where DSL
├── frontend/
│   ├── admin/          # React + shadcn 管理后台（构建产物嵌入二进制）
│   ├── sdk/            # 官方 TypeScript SDK
│   ├── registry/       # 插件 / 内容类型注册中心（Cloudflare Workers）
│   ├── templates/      # 起步模板（blog / ecommerce / payment 等）
│   └── raisfast.com/   # 官网与文档站
├── migrations/         # 三套 schema（sqlite / postgres / mysql）
├── extensions/         # 内置示例：content_types/ + plugins/
├── plugin-sdk/         # 插件开发 SDK（JS / Lua 类型与助手）
├── plugin-wit/         # WASM 组件接口定义（.wit）
├── adminui/            # 预构建管理后台资源（rust-embed 嵌入）
├── templates/          # 应用 / 插件 / 邮件 / 代码生成模板
├── locales/            # i18n（en / zh-CN）
├── deploy/             # Fly.io / Vercel 部署配置
└── docs/ · dev-docs/   # 架构文档 / 设计稿
```

`crates/core/src/` 模块概览：

```
src/
├── main.rs · lib.rs · app.rs     # 入口 + AppState 组装
├── server/                       # HTTP 服务器 + 路由注册 + OpenAPI
├── cli/                          # 子命令（server/db/user/ct/plugin/...）
├── handlers/                     # 路由处理器（薄层：提取参数 → 调 Service → 响应）
├── services/                     # 业务逻辑层（资源所有权校验 / Policy）
├── models/                       # 数据结构 + SQL 查询（sqlx + CRUD 宏）
├── dto/                          # 数据传输对象
├── commands/                     # CQRS 命令处理器
├── middleware/                   # 认证 / 限流 / CORS / 租户 / 指标 / 安全头
├── plugins/                      # 4 引擎插件系统（JS/Rhai/Lua/WASM）+ VFS
├── content_type/                 # 动态内容类型 + 规则引擎 + 导入导出
├── payment/                      # 支付提供商 + 路由 + 加密
├── workflow/                     # 工作流引擎
├── worker/                       # 任务队列 + Cron 调度器
├── graphql/                      # GraphQL API
├── mcp/                          # MCP（Model Context Protocol）服务器
├── proxy/                        # 多租户反向代理
├── tauri/                        # Tauri 桌面端集成
├── oauth/                        # OAuth 提供者
├── notifier/                     # 邮件 / SMS 通知
├── event/ · eventbus.rs          # 事件总线
├── webhook/                      # Webhook 系统
├── audit.rs · cache.rs           # 审计日志 / 缓存（moka）
├── protocols/                    # AOP 协议（ownable / tenantable / timestampable …）
├── db/                           # 连接池 / 方言 / Schema / 多租户 / 写锁
├── storage/                      # 文件存储（本地 / S3）
├── search/                       # 全文搜索（Tantivy）
├── config/                       # 基于环境变量的配置
├── errors/                       # 统一 AppError（thiserror）
├── types/ · utils/               # 共享类型 / 工具函数
└── admin_spa.rs                  # 嵌入式管理后台（rust-embed）
```

### 分层设计

```
Handler → Service → Model (SQL)
                ↘ 外部服务: Storage / Cache / Search / EventBus / Webhook
```

- **Handler** 不含业务逻辑，是唯一的鉴权入口（`ensure_*`）
- **Service** 编排 Model 和外部服务，只做资源所有权校验（Policy）
- **Model** 只含数据结构和 SQL 查询，提供 `tx_*` 变体参与事务

---

## CLI 命令

raisfast 自带完整的命令行（clap）：

```bash
raisfast                               # 等同于 raisfast server start
raisfast server start|stop|restart|status
raisfast db migrate|rollback|backup
raisfast user create|list|passwd|delete|disable|enable
raisfast ct new|check|types            # 内容类型管理 + TS 类型生成
raisfast plugin new|check              # 插件脚手架 + 校验
raisfast codegen model                 # 由 schema.sql 生成 model 脚手架
raisfast route ...                     # 路由检查
raisfast doctor                        # 系统诊断
raisfast mcp serve                     # 以 stdio 启动 MCP 服务器（接 Claude Desktop 等）
raisfast proxy start|check             # 多租户反向代理
raisfast tunnel <local-port>           # 暴露本地端口到公网
```

---

## 切换数据库

零代码改动，只换 feature flag：

```bash
# SQLite（默认）
cargo build --no-default-features --features "db-sqlite"

# PostgreSQL
cargo build --no-default-features --features "db-postgres"

# MySQL
cargo build --no-default-features --features "db-mysql"
```

> **注意：** PostgreSQL / MySQL 的 `BIGINT PRIMARY KEY` 不会自增；sqlx 过程宏在编译期需要在线数据库校验。详见 [`AGENTS.md`](AGENTS.md) 的「Postgres first-run」与跨库开发规范。

---

## Feature Flags

```bash
# 数据库后端（选其一；默认 db-sqlite）
--features "db-sqlite | db-postgres | db-mysql"

# 插件运行时（默认已含 plugin-js / plugin-lua / plugin-rhai）
--features "plugin-all"          # 追加 WASM 引擎
--features "plugin-wasm"         # 仅追加 WASM

# 支付（默认关闭）
--features "payment-all"         # 支付宝 + 微信 + Stripe + Dodo + Creem
--features "payment-stripe"      # 或按需单选

# 其他可选能力
--features "storage-s3"          # S3 / RustFS 对象存储
--features "tls"                 # 内置 HTTPS（rustls）
--features "cron-system"         # 系统 Shell 脚本 Cron
--features "tauri"               # Tauri 桌面模式
--features "export-types"        # 导出 TypeScript 类型（ts-rs）
```

**默认开启：** `db-sqlite`, `search-tantivy`, `plugin-js`, `plugin-lua`, `plugin-rhai`, `openapi`, `proxy`, `tunnel`, `mcp`

---

## 插件系统

```
extensions/plugins/
└── my-plugin/
    ├── plugin.toml      # 插件清单
    ├── main.js          # JavaScript（QuickJS）
    ├── main.lua         # Lua（mlua）
    ├── main.rhai        # Rhai
    └── main.wasm        # WASM（wasmtime，遵循 plugin-wit/plugin.wit 契约）
```

`plugin.toml` 示例：

```toml
[plugin]
name = "my-plugin"
version = "0.1.0"
entry = "main.js"

[permissions]
http = ["GET"]
db = ["read"]
hooks = ["post_created", "comment_created"]
```

- **插件 SDK**：`plugin-sdk/` 提供 JS / Lua 的类型定义与助手；WASM 插件遵循 `plugin-wit/plugin.wit` 接口契约。
- **虚拟文件系统**：每个插件拥有独立 VFS（`PLUGIN_VFS_*` 配额可调）。
- **示例**：`extensions/` 内置 crm、forum、seo-rhai 等示例插件与内容类型，可直接参考。

---

## 配置

所有配置通过环境变量或 `.env`（完整清单见 `.env.example`）：

```bash
# ── 服务器 ──────────────────────────────
APP_HOST=0.0.0.0
APP_PORT=9898                       # 默认 9898
APP_ENV=development                 # development | production
APP_TIMEZONE=UTC                    # IANA，如 Asia/Shanghai
# APP_KEY=                           # 首次启动自动生成（AES-GCM 加密用）
# CORS_ORIGINS=https://your.domain   # 多个用逗号分隔
# BASE_URL=http://localhost:9898     # 公开访问地址（用于 RSS / 媒体链接）

# ── 数据库 ──────────────────────────────
DATABASE_URL=sqlite:./storage/db/raisfast.db?mode=rwc
# DB_POOL_SIZE=5
# STORAGE_ROOT_DIR=./storage         # 本地文件根目录（uploads/logs/search_index/vfs/db）

# ── 认证 ────────────────────────────────
JWT_SECRET=change-me-in-production-at-least-32-chars
JWT_ACCESS_EXPIRES=900               # 15 分钟
JWT_REFRESH_EXPIRES=604800           # 7 天

# ── 存储 ────────────────────────────────
# STORAGE_DRIVER=local               # local | s3
# MAX_UPLOAD_SIZE=104857600
# S3_ENDPOINT=http://rustfs:9000     # 仅 STORAGE_DRIVER=s3 时
# S3_ACCESS_KEY= · S3_SECRET_KEY= · S3_BUCKET= · S3_REGION= · S3_PUBLIC_URL=

# ── 插件 ────────────────────────────────
# PLUGIN_DIR=./extensions/plugins
# PLUGIN_VFS_ROOT= · PLUGIN_VFS_MAX_FILE_SIZE= · PLUGIN_VFS_MAX_TOTAL_SIZE=
# PLUGIN_WASM_POOL_SIZE=4 · PLUGIN_LUA_POOL_SIZE=4 · PLUGIN_JS_POOL_SIZE=4

# ── Worker / Cron ───────────────────────
# WORKER_ENABLED=false · WORKER_CONCURRENCY=2
# CRON_SEED_ENABLED=false · CRON_LOG_RETENTION_DAYS=30
# CRON_SCHEDULES=[{"label":"...","job_type":"...","cron_expr":"...","enabled":true}]

# ── API 开关 ────────────────────────────
# GRAPHQL_ENABLED=true               # /api/v1/graphql
# WEBSOCKET_ENABLED=true             # /api/v1/ws

# ── 内置模块开关 ────────────────────────
# BUILTIN_BLOG=true · BUILTIN_PAGES=true · BUILTIN_MEDIA=true
# BUILTIN_FULLTEXT=true · BUILTIN_WORKFLOW=true · BUILTIN_TENANTABLE=false

# ── 限流（max_requests/window_secs）─────
# RATE_LIMIT_GLOBAL_MAX=60 · RATE_LIMIT_LOGIN_MAX=10 ...
```

---

## 部署

| 平台 | 方式 |
|------|------|
| **Docker** | `docker build` 或 `docker compose up`（含 RustFS） |
| **Fly.io** | `just deploy-fly`（配置见 `deploy/fly/`） |
| **Vercel** | 脚本 `deploy/vercel.sh` |
| **裸机 / 二进制** | `cargo build --release`，单文件分发 |

---

## 技术栈

| 层 | 技术 |
|----|------|
| 语言 | Rust（edition 2024） |
| HTTP 框架 | Axum 0.8 |
| 数据库 | SQLx 0.9（SQLite / PostgreSQL / MySQL） |
| GraphQL | async-graphql 7 |
| 认证 | JWT（HS256）+ Argon2 |
| 缓存 | moka |
| 搜索 | Tantivy 0.26 |
| 插件运行时 | wasmtime / rquickjs / mlua / rhai |
| ID 生成 | ferroid（Snowflake + 乘法逆元密码 + base62） |
| 邮件 / 模板 | lettre / Tera / Comrak（Markdown） |
| 管理后台 | React 19 + Vite + shadcn/ui（Base UI） |
| 桌面端 | Tauri 2 |
| 对象存储 | AWS SDK for S3 / RustFS |
| 嵌入式资源 | rust-embed |

---

## 项目状态

| 组件 | 状态 |
|------|------|
| 核心 API + 管理后台 | ✅ 可用 |
| 认证（JWT + OAuth + API Token + RBAC） | ✅ 可用 |
| 多数据库（SQLite / PG / MySQL） | ✅ 可用 |
| 插件引擎（JS / Rhai / Lua / WASM） | ✅ 可用 |
| Content Type 系统（+ 规则引擎） | ✅ 可用 |
| 电商（购物车 / 订单 / 运费模板） | ✅ 可用 |
| 钱包 + 多支付路由 | ✅ 可用 |
| 工作流引擎 | ✅ 可用 |
| 任务队列 + Cron | ✅ 可用 |
| MCP 服务器 | ✅ 可用 |
| GraphQL API | ✅ 可用 |
| Webhook + 通知 | ✅ 可用 |
| 多租户 + 反向代理 | ✅ 可用 |
| 隧道（tunnel） | ✅ 可用 |
| Tauri 桌面端 | 🔧 开发中 |
| 插件市场 | 📋 计划中 |

---

## 许可证

采用 [Apache License 2.0](LICENSE) 许可。

---

## 参与贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 与 [AGENTS.md](AGENTS.md)（架构约束与跨库开发规范）了解详情。

---

<p align="center">
  用 ❤️ 和 Rust 构建
</p>
