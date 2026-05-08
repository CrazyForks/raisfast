<p align="center">
  <h1 align="center">raisfast</h1>
  <p align="center">
    <strong>Rust 高性能 Headless CMS · Serverless · 桌面端</strong>
  </p>
  <p align="center">
    <em>一个 CMS，三种部署，零妥协。</em>
  </p>
</p>

---

> **⚠️ 早期 Alpha 阶段 — 尚未达到生产可用状态。**
>
> 本项目正在积极开发中，API 随时可能变更。
> 提前开源是为了确立先发权并收集反馈。
> 稳定版 v1.0 计划于 2026 年 Q3 发布。

---

## raisfast 是什么？

raisfast 是一个完全用 Rust 构建的**高性能 Headless CMS 和 API 引擎**。一套代码支持三种运行模式：

| 模式 | 适用场景 | 数据库 | 存储 |
|------|----------|--------|------|
| **桌面端** (Tauri) | 个人博客、本地开发 | SQLite（内嵌） | 本地文件系统 |
| **Serverless** | 团队协作、零运维 | PostgreSQL / MySQL / D1 | S3 / R2 |
| **自部署** | 企业私有化、完全可控 | SQLite / PostgreSQL / MySQL | 本地 / S3 |

**没有任何其他 CMS 能从同一套代码实现这三种部署方式。**

### 为什么选 Rust？

| 指标 | raisfast (Rust) | Node.js CMS (Strapi/Payload) |
|------|-----------------|------------------------------|
| 冷启动 | <5ms | ~500ms |
| 内存占用 | <50MB | ~300MB |
| 产物体积 | ~15MB | ~200MB (node_modules) |
| 单实例 RPS | 50K+ | ~2K |
| 插件沙箱 | ✅ WASM + JS + Lua | 仅 JS |

---

## 功能特性

### 核心
- **REST API** — 文章、页面、分类、标签、评论、媒体的完整 CRUD
- **管理后台** — 现代 React 仪表盘（编译进二进制，零配置）
- **认证** — JWT (HS256) + Refresh Token + OAuth (GitHub) + 短信登录
- **RBAC** — 基于角色的细粒度权限控制
- **多租户** — 可选的 `BUILTIN_TENANTABLE` 模式，支持 SaaS 场景

### 内容管理
- **Content Type 系统** — 通过 TOML 定义自定义内容类型
- **AOP 切面** — 自动时间戳、软删除、所有权、锁定、发布、排序、Slug 等 11 种协议
- **区块编辑器** — 页面构建器 + 可复用区块
- **媒体库** — 上传、缩略图、尺寸检测
- **RSS/Atom** — 自动生成订阅源
- **Sitemap** — 自动生成站点地图

### 可扩展性
- **插件引擎** — 三种运行时：WASM (wasmtime)、JavaScript (QuickJS)、Lua (mlua)
- **Hook 系统** — 文章、评论、媒体、用户等生命周期钩子
- **事件总线** — 进程内事件系统，支持插件协作
- **定时任务** — 插件可注册 Cron 定时任务

### 基础设施
- **多数据库** — SQLite / PostgreSQL / MySQL 零改动切换
- **SQL 方言层** — 自动占位符转换（`?` → `$1`）、时间函数、UPSERT 语法
- **任务队列** — 内置 SQLite 任务队列，支持重试、死信、Cron 调度
- **Webhook** — HTTP 回调订阅系统事件
- **审计日志** — 追踪所有管理操作
- **限流** — 可配置的按端点限流
- **Swagger UI** — 自动生成的 OpenAPI 文档

---

## 快速开始

### 前置条件

- Rust 1.85+ (edition 2024)
- SQLite 3.x（默认），或 PostgreSQL / MySQL

### 编译运行

```bash
# 克隆
git clone https://github.com/snkzhong/raisfast.git
cd raisfast

# 编译运行（SQLite，默认）
cargo run --features "db-sqlite plugin-all search-tantivy"

# 服务启动在 http://localhost:9898
# 管理后台在 http://localhost:9898/admin
# Swagger 文档在 http://localhost:9898/swagger-ui
```

### 首次启动

首次启动时，raisfast 会自动：
1. 创建所有数据库表（`schema.sqlite.sql`）
2. 初始化默认角色、权限和站点配置
3. 启动 API + 管理后台

创建管理员账户：

```bash
cargo run -- db seed admin@example.com admin your-password
```

### Docker 部署

```bash
docker build -t raisfast .
docker run -p 9898:9898 -v ./data:/app/data raisfast
```

---

## 架构

```
src/
├── main.rs              # CLI 入口
├── server.rs            # HTTP 服务器 + 路由注册
├── lib.rs               # AppState 组装
├── handlers/            # 路由处理器（薄层：提取参数 → 调用 Service → 返回响应）
├── services/            # 业务逻辑层
├── models/              # 数据结构 + SQL 查询
├── repositories/        # Repository 模式（trait + Sqlx 实现）
├── middleware/           # 认证、限流、CORS、指标
├── plugins/             # 插件引擎（WASM/JS/Lua）
├── content_type/        # 动态内容类型系统
├── worker/              # 任务队列 + Cron 调度器
├── db/                  # 连接池、SQL 方言、Schema
├── config/              # 基于环境变量的配置
├── errors/              # 统一 AppError（thiserror）
├── storage/             # 文件存储（本地 / S3）
├── search/              # 全文搜索（Tantivy）
├── oauth/               # OAuth 提供者
├── protocols/           # AOP 协议定义
├── aspects/             # AOP 切面引擎
└── admin_spa.rs         # 嵌入式管理后台（rust-embed）
```

### 分层设计

```
Handler → Service → Repository → Model (SQL)
                  ↘ 外部服务: Storage / Cache / Search / EventBus
```

- Handler 不包含业务逻辑
- Service 编排 Repository 和外部服务
- Model 只包含数据结构和 SQL 查询

---

## 切换数据库

零代码改动，只换 feature flag：

```bash
# SQLite（默认）
cargo build --features "db-sqlite"

# PostgreSQL
cargo build --features "db-postgres"

# MySQL
cargo build --features "db-mysql"
```

SQL 方言层（`src/db/dialect.rs`）自动处理：
- 占位符转换：`?` → `$1, $2, ...`（PostgreSQL）
- 时间函数：`datetime('now')` → `NOW()`
- 日期运算：`datetime('now', '-N days')` → `NOW() - INTERVAL 'N days'`
- UPSERT：`ON CONFLICT ... DO UPDATE` → `ON DUPLICATE KEY UPDATE`（MySQL）
- RETURNING：`RETURNING *` → MySQL 下禁用

---

## 插件系统

支持三种语言编写插件：

```
plugins/
├── my-plugin/
│   ├── plugin.toml      # 插件清单
│   ├── main.js          # JavaScript (QuickJS)
│   ├── main.lua         # Lua (mlua)
│   └── main.wasm        # WASM (wasmtime)
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

---

## 配置

所有配置通过环境变量：

```bash
# 数据库
DATABASE_URL=sqlite:./data/raisfast.db

# 服务器
PORT=9898
HOST=0.0.0.0

# 认证
JWT_SECRET=your-secret-key
JWT_ACCESS_TTL=900          # 15 分钟
JWT_REFRESH_TTL=604800      # 7 天

# 存储
STORAGE_DRIVER=local         # local | s3
UPLOAD_DIR=./uploads

# 多租户
BUILTIN_TENANTABLE=false     # 启用后所有内置表加 tenant_id 列

# 搜索
SEARCH_DRIVER=tantivy        # tantivy | noop

# 插件
PLUGIN_DIR=./plugins
PLUGIN_HOT_RELOAD=true
```

---

## 技术栈

| 层 | 技术 |
|----|------|
| 语言 | Rust (edition 2024) |
| HTTP 框架 | Axum 0.8 |
| 数据库 | SQLx 0.8 (SQLite / PostgreSQL / MySQL) |
| 认证 | JWT (HS256) + Argon2 |
| 搜索 | Tantivy |
| 插件运行时 | wasmtime / rquickjs / mlua |
| 管理后台 | React 19 + Vite + shadcn/ui |
| 桌面端 | Tauri |
| 嵌入式资源 | rust-embed |

---

## 项目状态

| 组件 | 状态 |
|------|------|
| 核心 API | ✅ 可用 |
| 管理后台 | ✅ 可用 |
| 认证（JWT + OAuth） | ✅ 可用 |
| 多数据库 | ✅ 可用 |
| 插件引擎（JS/Lua） | ✅ 可用 |
| 插件引擎（WASM） | ✅ 可用 |
| Content Type 系统 | ✅ 可用 |
| Tauri 桌面端 | ✅ 可用 |
| 任务队列 + Cron | ✅ 可用 |
| Serverless 适配器 | 🔧 设计中 |
| Redis 缓存 | 🔧 计划中 |
| 插件市场 | 📋 计划中 |
| SDK（JS/Python） | 📋 计划中 |

---

## 许可证

raisfast 采用双重许可：

- **核心框架**：[MIT 许可证](LICENSE)
- **商业模块**（SaaS 托管、插件市场、企业功能）：[BSL 1.1](LICENSE-COMMERCIAL)

详见 [LICENSE](LICENSE)。

---

## 参与贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。

注意：本项目处于早期 Alpha 阶段，v1.0 之前 API 可能会有较大变动。

---

<p align="center">
  用 ❤️ 和 Rust 构建
</p>
