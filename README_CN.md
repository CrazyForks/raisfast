<p align="center">
  <h1 align="center">RaisFast</h1>
  <p align="center">
    <strong>最快的 CMS，最简单的部署。</strong>
  </p>
  <p align="center">
    基于 Rust 的高性能 BaaS 与 headless CMS，内置博客、电商、钱包、支付、多租户 SaaS。JS / Rhai / Lua / WASM 四引擎插件无限扩展。<br>
    单二进制文件，零依赖，零 GC。下载即运行。
  </p>
</p>

---

> **早期 Alpha 阶段 — v1.0 前 API 可能变更。**
> 稳定版 v1.0 计划于 2026 年 Q3 发布。

---

## 为什么选 raisfast？

**单文件，全能力**
一个二进制，无需 Node.js、无需 Docker、无需运行时。
博客、电商、钱包、支付从数据库到 API 原生内置，不是插件拼装，是骨骼。

**Rust 性能，零 GC 稳定**
读延迟亚毫秒，长时间运行性能零退化。
没有 GC 停顿，没有内存泄漏，没有凌晨三点的 OOM 告警。

**4 套插件引擎，取 Strapi 之长**
JS、Rhai、Lua、WASM 四层扩展，覆盖从脚本到编译型的完整光谱。
享受动态语言的开发效率，拥有 Rust 的性能基座。

---

## 内置功能

| 模块 | 功能 |
|------|------|
| **博客 / CMS** | 文章、页面、分类、标签、评论、媒体、RSS、站点地图 |
| **电商** | 购物车、订单、商品变体、优惠券 |
| **钱包与支付** | 多币种钱包、支付宝 / 微信支付 / Stripe / Dodo / Creem |
| **OAuth** | GitHub、Google 等社交登录 |
| **工作流** | 任务队列、定时任务、AOP 切面、事件总线 |
| **内容类型** | 通过 TOML 定义动态 Schema，自动生成 CRUD API |
| **认证** | JWT (HS256) + Refresh Token + API Token + RBAC |
| **多租户** | 可选租户隔离，支持 SaaS 场景 |
| **管理后台** | 现代 React 仪表盘（嵌入二进制，零配置） |
| **插件引擎** | JS (QuickJS) / Rhai / Lua (mlua) / WASM (wasmtime) |
| **搜索** | 全文搜索（Tantivy） |
| **多数据库** | SQLite / PostgreSQL / MySQL 零改动切换 |

---

## 快速开始

```bash
# 克隆
git clone https://github.com/RaisFast/raisfast.git
cd raisfast

# 编译运行（SQLite，默认）
cargo run --features "db-sqlite plugin-all search-tantivy"

# 服务启动在 http://localhost:9898
# 管理后台在 http://localhost:9898/admin
# Swagger 文档在 http://localhost:9898/swagger-ui
```

### 首次启动

首次启动时，raisfast 会自动：
1. 创建所有数据库表
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
├── middleware/           # 认证、限流、CORS、指标
├── plugins/             # 插件引擎（WASM/JS/Rhai/Lua）
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
Handler → Service → Model (SQL)
                ↘ 外部服务: Storage / Cache / Search / EventBus
```

- Handler 不包含业务逻辑
- Service 编排 Model 和外部服务
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

---

## 插件系统

```bash
plugins/
├── my-plugin/
│   ├── plugin.toml      # 插件清单
│   ├── main.js          # JavaScript (QuickJS)
│   ├── main.lua         # Lua (mlua)
│   ├── main.rhai        # Rhai
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

所有配置通过环境变量或 `.env`：

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
BUILTIN_TENANTABLE=false

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
| 插件运行时 | wasmtime / rquickjs / mlua / rhai |
| 管理后台 | React 19 + Vite + shadcn/ui |
| 桌面端 | Tauri |
| 嵌入式资源 | rust-embed |

---

## 项目状态

| 组件 | 状态 |
|------|------|
| 核心 API | ✅ 可用 |
| 管理后台 | ✅ 可用 |
| 认证（JWT + OAuth + API Token） | ✅ 可用 |
| 多数据库 | ✅ 可用 |
| 插件引擎（JS/Rhai/Lua/WASM） | ✅ 可用 |
| Content Type 系统 | ✅ 可用 |
| 电商（购物车/订单/支付） | ✅ 可用 |
| 钱包 | ✅ 可用 |
| 任务队列 + Cron | ✅ 可用 |
| Tauri 桌面端 | ✅ 可用 |
| AOP 切面 | ✅ 可用 |
| Serverless 适配器 | 🔧 设计中 |
| 插件市场 | 📋 计划中 |

---

## 许可证

采用 [Apache License 2.0](LICENSE) 许可。

---

## 参与贡献

欢迎贡献！请阅读 [CONTRIBUTING.md](CONTRIBUTING.md) 了解详情。

---

<p align="center">
  用 ❤️ 和 Rust 构建
</p>
