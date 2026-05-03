# PocketBase 底层技术架构深度分析

> 对标项目：PocketBase v0.37.1（Go） vs raisfast（Rust）
> 目的：分析 PocketBase 的核心架构设计，识别值得借鉴的特性，制定实施优先级。

## 一、架构概览

| 维度 | PocketBase (Go) | raisfast (Rust) |
|------|----------------|-----------------|
| 语言 | Go 1.25 | Rust 2024 |
| 数据库 | 嵌入式 SQLite（modernc.org/sqlite，纯 Go 无 CGO） | 嵌入式 SQLite（sqlx + libsqlite3-sys） |
| HTTP 框架 | 自研路由（`core.ServeEvent.Router`） | Axum 0.8 |
| 二进制体积 | ~12MB（单文件可执行） | ~28MB |
| 扩展方式 | Go 框架 / 内嵌 JS VM（goja） | Lua / JS / WASM 插件 |
| 无状态 JWT | 不存储 token，纯 HS256 无状态 | 存储 refresh token 到 DB |
| 实时推送 | SSE 内建 | 无 |
| Admin UI | 内建（`/_/`） | 无（仅有博客前台） |

## 二、核心设计理念

### 1. 极简单文件部署

PocketBase 的核心哲学是 **"下载即用"**：单个二进制文件包含所有功能（SQLite、Web UI、API server、Auth、文件上传），零配置启动。Go 的交叉编译特性使其天然适合这个模型。

### 2. 双数据库架构

PocketBase 使用两个 SQLite 文件：

- **`data.db`** — 业务数据（collections/records）
- **`auxiliary.db`** — 日志和临时系统数据

这样做的好处是日志写入不会干扰业务查询的 WAL 锁，值得借鉴。

### 3. Collection-Driven Schema

PocketBase 的核心抽象是 **Collection**（等价于 raisfast 的 ContentType）。每个 Collection 有：

- 字段定义（schema）
- 5 条 API Rule（list/view/create/update/delete）
- 自动的 CRUD REST API

这与 raisfast 的 TOML schema 驱动方式高度一致。

## 三、值得借鉴的 6 个关键设计

### 借鉴 1：表达式级 API Rule 系统（高优先级）

这是 PocketBase 最强大的功能。每个 Collection 的 5 条 Rule（listRule/viewRule/createRule/updateRule/deleteRule）都是**运行时表达式**，不是简单的公开/私有标志。

```
// 示例：只返回已发布且作者为自己的文章
status = "published" && author = @request.auth.id

// 示例：只有管理员可以删除
@request.auth.role = "admin"

// 支持跨 collection 查询
@collection.orders.user ?= @request.auth.id
```

**PocketBase 的 filter 引擎**（`ganigeorgiev/fexpr`）支持：

- 比较：`= != > >= < <=`
- 模糊匹配：`~ !~`（LIKE）
- 多值操作：`?= ?!= ?~`（ANY/AT LEAST ONE OF）
- `@request.*` 上下文：auth user、body、query、headers
- `@collection.*` 跨表引用
- `@now` 时间宏
- `:isset` / `:changed` / `:length` / `:each` 修饰符
- `geoDistance()` 地理距离函数
- `strftime()` 时间格式化

**raisfast 现状**：`check_api_access()` 只是简单的枚举（public/authenticated/admin/none），没有表达式级别的过滤。

**建议**：实现一个 `RuleEngine`，将 API Rule 解析为 SQL WHERE 子句。可以先支持基础子集：

- 字段比较：`status = "published"`
- 认证上下文：`@request.auth.id`
- 逻辑组合：`&&` / `||`

### 借鉴 2：SSE 实时订阅（高优先级）

PocketBase 内建了 **Server-Sent Events (SSE)** 实时订阅：

```
GET /api/realtime          → 建立 SSE 连接
POST /api/realtime         → 设置订阅主题
```

- 订阅粒度：整个 Collection 或单条 Record
- 自动权限检查：订阅 Collection 用 listRule，订阅 Record 用 viewRule
- 5 分钟无消息自动断开（防泄漏连接）

**raisfast 现状**：没有实时推送能力。

**建议**：基于 Axum 的 `tokio-stream` + SSE 实现。架构：

- `EventBus` 已存在，可复用
- 新增 `/api/realtime` SSE endpoint
- 客户端发送 `POST /api/realtime` 设置订阅
- 每次 CUD 操作后，通过 EventBus 广播 → SSE 推送

### 借鉴 3：skipTotal 优化（中优先级，低工作量）

PocketBase 的列表 API 支持 `?skipTotal=true` 参数，跳过 `COUNT(*)` 查询，`totalItems` 和 `totalPages` 返回 `-1`。

**为什么重要**：SQLite 的 `COUNT(*)` 在大数据集上会全表扫描。当用户只是翻页浏览时，第二页开始不需要总数。

**raisfast 现状**：每次列表查询都执行 `SELECT COUNT(*)`。

**建议**：在 `ContentQuery` 加 `skip_total: bool` 字段，`ListParams` 加 `skip_total` 参数，条件跳过 count 查询。

### 借鉴 4：Batch API（中优先级）

PocketBase 支持 `POST /api/batch`，在一个事务中批量执行多个 create/update/upsert/delete 操作：

```json
{
  "requests": [
    { "method": "POST", "url": "/api/collections/posts/records", "body": {"title": "test1"} },
    { "method": "PATCH", "url": "/api/collections/posts/records/xxx", "body": {"title": "test2"} },
    { "method": "DELETE", "url": "/api/collections/posts/records/yyy" }
  ]
}
```

- 整个 batch 在单个事务中执行，失败全部回滚
- 减少 HTTP 往返次数

**raisfast 现状**：没有批量操作 API。

**建议**：新增 `POST /api/v1/batch` endpoint，接收请求数组，在单个 SQLite 事务中执行。

### 借鉴 5：无状态 JWT（低优先级，架构差异）

PocketBase **不存储任何 JWT token**：

- Access token：HS256 签发后不存 DB
- 没有 refresh token 机制
- "登出" = 客户端删除 token
- `authRefresh` 接口不废止旧 token，只是返回新 token

**raisfast 现状**：有完整的 refresh token 机制（存 DB），更安全但也更慢。

**建议**：这是架构取舍，不需要改。raisfast 的双 token 方案更适合需要强安全控制的场景（企业 CMS），这是差异化优势。

### 借鉴 6：内置 Admin Dashboard UI（高优先级，差异化机会）

PocketBase 的杀手级特性是开箱即用的 Admin UI（`/_/`），功能包括：

- Collection 可视化管理（增删改字段）
- Record 数据浏览和编辑
- API Rule 配置界面
- Auth 用户管理
- 日志查看
- 系统设置

**raisfast 现状**：前端 `web/` 是博客前台，不是管理后台。

**建议**：基于现有的 Next.js + shadcn/ui 搭建 Admin Dashboard，核心页面：

1. Collection Manager（字段拖拽编辑）
2. Record Browser（表格 + 筛选）
3. API Rule Editor（表达式编辑器）
4. 用户管理

## 四、性能优化对照

| 优化项 | PocketBase | raisfast 现状 |
|-------|-----------|--------------|
| SQLite PRAGMA | busy_timeout=10000, WAL, synchronous=NORMAL, cache_size=-32000 | 已优化（Phase 1.1） |
| JWT 缓存 | 每次解析但 HS256 开销极小 | 已缓存 DecodingKey（Phase 1.2） |
| Filter→SQL 编译 | `fexpr` 解析为 AST → 直接映射 SQL | 尚未实现表达式过滤 |
| 列表缓存 | 无内置缓存（纯 SQL） | DashMap TTL 缓存（Phase 1.3） |
| 并发数据结构 | Go map + sync.RWMutex | DashMap + ArcSwap（Phase 2.1/2.2） |
| Relation 批量化 | WHERE IN 批量查询 | 已实现批量 OneToMany/ManyToMany（Phase 2.3） |

## 五、PocketBase 核心依赖栈

| 组件 | 库 | 说明 |
|------|-----|------|
| SQLite 驱动 | modernc.org/sqlite | 纯 Go，无 CGO |
| 数据库抽象 | pocketbase/dbx | 自研 query builder |
| JS 引擎 | dop251/goja | ECMAScript 2020 |
| 表达式解析 | ganigeorgiev/fexpr | 自研 filter 表达式引擎 |
| JWT | golang-jwt/jwt/v5 | HS256 |
| 密码哈希 | golang.org/x/crypto | bcrypt |
| 图片处理 | disintegration/imaging | 缩略图生成 |
| MIME 检测 | gabriel-vasile/mimetype | 文件类型识别 |
| 邮件发送 | domodwyer/mailyak | SMTP |
| CLI | spf13/cobra | 命令行框架 |
| 文件监听 | fsnotify/fsnotify | 热重载 |

## 六、实施优先级建议

| 优先级 | 功能 | 工作量 | 用户价值 |
|-------|------|-------|---------|
| P0 | skipTotal 优化 | 1-2h | 立即提升列表查询性能 |
| P1 | SSE 实时订阅 | 1-2d | 前端实时更新的核心能力 |
| P1 | 表达式级 API Rule | 3-5d | 安全模型质变，对标 Strapi |
| P2 | Batch API | 1-2d | 减少前端 HTTP 往返 |
| P2 | Admin Dashboard UI | 1-2 周 | 可用性的关键差异 |
| P3 | 双数据库分离 | 0.5d | 日志写入不影响业务性能 |
