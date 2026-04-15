# 架构审计报告

> 审计时间：2026-04-15
>
> 基于 Phase 8-14 全部完成后的代码状态。

## 1. 总体评估

**评级：中等偏上**——骨架扎实，Schema-driven 扩展能力强，但缺少生产级配套（审计、可观测性、缓存）。

### 做得好的

| 领域 | 评价 | 关键文件 |
|------|------|----------|
| 分层架构 | Handler→Service→Repository 三层清晰一致 | `src/handlers/` `src/services/` `src/repositories/` |
| Content Type 引擎 | TOML 定义即得自动建表 + CRUD API + Admin UI | `src/content_type/` |
| 多租户隔离 | `tenant_id` 贯穿 repo/service/handler 三层，`ResolvedTenant` 中间件统一解析 | `src/db/tenant.rs` `src/middleware/tenant.rs` |
| 插件系统 | WASM/JS/Lua 三引擎，HookPoint 泛化，VFS 隔离 | `src/plugins/` |
| 错误处理 | 统一 `AppError` 枚举 + i18n + JSON envelope | `src/errors/app_error.rs` |
| 输入校验 | `validator` crate + DTO 层一致调用 `validation::validate()` | `src/handlers/dto.rs` `src/errors/validation.rs` |
| 事件驱动 | EventBus + Worker + Cron 完整闭环 | `src/eventbus/` `src/worker/` |
| RBAC | 细粒度权限矩阵，条件权限（`author_id == $user.id`） | `src/services/rbac.rs` `src/middleware/permission.rs` |

### 成为复杂系统的关键短板

| 短板 | 严重度 | 说明 |
|------|--------|------|
| 审计日志 | **高** | EventBus 有事件但无持久化订阅者，admin 操作无 "谁做了什么" 记录 |
| 可观测性 | **高** | 无 `#[instrument]`、无 metrics、无 request ID 串联 |
| 缓存层不完整 | **高** | 只有 posts 有缓存，worker 缓存失效 handler 是 stub |
| 事务覆盖 | 中 | media 删除（DB+文件）非原子，options 批量更新无事务 |
| API 版本策略 | 中 | 只有 `/api/v1`，无 v2 迁移计划 |
| Webhook 不完整 | 中 | 无订阅管理、无 HMAC 签名、无事件过滤 |
| 测试覆盖 | 中 | Options/Plugin/RBAC/Stats/Tenant Admin 无集成测试 |
| 限流策略 | 低 | 内存级，无法多实例共享，admin 无独立限流 |

---

## 2. 详细分析

### 2.1 错误处理

**状态：强——设计完善，有少量缺口**

`AppError` 枚举（`src/errors/app_error.rs:49`）有 6 个变体：`BadRequest`、`Unauthorized`、`Forbidden`、`NotFound`、`Conflict`、`Internal`，带 `#[non_exhaustive]` 防止破坏性变更。

`From<sqlx::Error>` 自动映射：`RowNotFound` → `NotFound`，唯一约束违反 → `Conflict`。

`IntoResponse` impl 将每个变体映射到 HTTP 状态码 + i18n 消息 + 结构化 JSON body。

**缺口：**

- **缺少 `TooManyRequests`（429）变体。** 限流中间件（`src/middleware/rate_limit.rs:246-259`）手动构造 JSON 响应，绕过了 `AppError` 管线和 i18n。
- **缺少 `PayloadTooLarge`（413）变体。** `RequestBodyLimitLayer` 产生默认 Axum 响应，不走 `AppError` 格式。
- **缺少 `MethodNotAllowed`（405）和 `ServiceUnavailable`（503）变体。**
- **`sqlx::Error::RowNotFound` 映射到 `NotFound("resource")`**（`src/errors/app_error.rs:227`）丢失了具体资源类型上下文。

### 2.2 事务支持

**状态：关键路径有事务，部分多步操作缺失**

已有事务的操作：

| 文件 | 行号 | 操作 |
|------|------|------|
| `src/repositories/sqlx_post.rs` | 104, 116 | Post 创建/更新 + tag 同步 |
| `src/services/auth.rs` | 316 | Refresh token 轮换（删除旧 + 插入新） |
| `src/worker/sqlite_queue.rs` | 113 | Job 出队 |
| `src/worker/scheduler.rs` | 370, 633 | Cron 清理和种子数据 |

**缺失事务但需要的场景：**

- **Media 删除**（`src/services/media.rs`）— 先删 DB 记录再删磁盘文件，两步非原子
- **Options 批量更新**（`src/services/options.rs`）— 逐条执行，无整体事务
- **Content Type 字段校验 + 写入** — 校验通过后写入，中间无事务保护

### 2.3 分页一致性

**状态：基本一致，两个端点缺少分页**

`PaginationParams`（`src/utils/pagination.rs`）统一用于 6 个列表端点：posts（公开+admin）、comments、media、users。

**不一致：**

- **`tag::list`** 和 **`category::list`** 返回 `Vec<T>` 无分页，数据量大时成为性能瓶颈
- Post 列表 handler 手动构造分页参数（`.max(1)` `.clamp()`），其他 handler 直接用 `Query(mut params)`
- `PaginatedData` 无 `total_pages` 字段或 `Link` header

### 2.4 输入校验

**状态：强——模式一致，有少量缺口**

所有接受 body 的 handler 统一调用 `validation::validate(&req)?`（`src/errors/validation.rs:42`），DTO 用 `validator` crate 注解。

**缺口：**

- Post/Comment 的 `status` 字段无枚举校验，任意字符串（如 `"hacked"`）都会被接受
- `category_id`、`tag_ids[]` 等 UUID 字段无格式校验
- Media 上传无 MIME 类型/扩展名白名单
- `RefreshRequest.refresh_token` 无 `length(min = 1)` 校验

### 2.5 可观测性

**状态：日志良好，无 metrics，无分布式追踪**

**已有：**

- `tower_http::TraceLayer` 记录请求/响应（method, URI, status, latency）
- 业务逻辑关键点有 `tracing::info!` / `tracing::error!`
- `AppError::into_response` 用 `tracing::warn!`（4xx）和 `tracing::error!`（5xx）

**缺失：**

- **无 `#[instrument]`**——全代码库 0 使用，无法 span 级关联 handler→service→repository 调用链
- **无 metrics**——无 Prometheus / OpenTelemetry，无法追踪请求率、错误率、P95/P99 延迟
- **无 request ID**——`TraceLayer` 不注入 `X-Request-ID`，无法跨服务关联
- **无 pool utilization 监控**——无法知道连接池使用率
- **日志字段不一致**——部分用格式化字符串，部分用结构化字段

### 2.6 限流

**状态：可用但有限**

`RateLimiterSet`（`src/middleware/rate_limit.rs:153`）包含 4 个命名限流器：`global`、`register`、`login`、`comment`，滑动窗口算法。

**限制：**

- **纯内存**——多实例部署时限流独立，等效于倍增限额
- **4 个限流器硬编码**——新增需改 struct + `from_config()` + 中间件函数
- **Admin 端点无独立限流**——全局 60 req/min 覆盖，持有 admin token 的攻击者可暴力请求
- **IP 提取信任 `X-Forwarded-For`**——无配置化信任代理
- **响应无 `Retry-After` / `X-RateLimit-*` header**
- **RateLimitStore trait 已定义但只有 MemoryStore 实现**

### 2.7 缓存

**状态：仅 posts 有缓存，失效 handler 是 stub**

`CacheStore` trait + `MemoryCache` 实现（`src/cache/mod.rs`）。`CachedPostRepository<P>` 装饰器（`src/repositories/cached_post.rs`）为 posts 提供读穿透缓存 + 写穿透失效，租户感知 cache key。

**缺口：**

- **`InvalidateCacheHandler` 是 stub**——`src/worker/handlers/cache.rs:35` 只有 `// TODO` 注释和日志，不实际删除缓存
- **只有 posts 有缓存**——categories、tags、users、comments、media、options 无缓存
- **Options 缓存无 TTL 刷新**——启动时加载到 HashMap，更新时写穿透，但无定期刷新机制
- **MemoryCache 无后台清理线程**——过期条目仅在 get/set 时惰性跳过
- **无 Redis 后端**——CacheStore trait 可扩展但只有内存实现

### 2.8 数据库连接管理

**状态：可用但配置最小**

`init_pool`（`src/db/connection.rs:14`）配置 `max_connections`，SQLite 设置 `journal_mode = WAL` + `foreign_keys = ON`。

**缺失：**

- 无 `min_connections` 配置
- 无 `acquire_timeout`——请求可能无限等待连接
- 无 `idle_timeout` / `max_lifetime`
- 无连接池利用率监控
- SQLite WAL 模式仅在启动时设置一次，非 per-connection
- 无数据库重连逻辑

### 2.9 测试覆盖

**状态：覆盖面广，深度中等**

| Handler | 有集成测试？ | 测试数 |
|---------|-------------|--------|
| health | 有 | 1 |
| auth | 有 | 9 |
| user | 有 | 7 |
| category | 有 | 5 |
| tag | 有 | 4 |
| post | 有 | 12 |
| comment | 有 | 8 |
| media | 有 | 4 |
| rss | 有 | 2 |
| cron | 有 | 6 |
| sse | 有 | 2 |
| tenant_e2e | 有 | 4 |
| **options** | **无** | 0 |
| **plugin** | **无** | 0 |
| **rbac** | **无** | 0 |
| **stats** | **无** | 0 |
| **tenant admin** | **部分**（仅 e2e 间接测试） | — |

单元测试总计 422 个，覆盖 content_type、plugins、search、worker 等核心模块。

**缺口：**

- Options、Plugin、RBAC、Stats、Tenant Admin CRUD 无集成测试
- 无负面/边界测试（限流溢出、并发请求等）
- 无负载/压力测试
- `src/cache/mod.rs`、`src/errors/validation.rs` 无单元测试

### 2.10 API 版本策略

**状态：单版本，无迁移策略**

所有路由嵌套在 `/api/v1`（`src/server/mod.rs:335`），无版本中间件、无 Accept header 协商、无版本感知序列化。

添加 `/api/v2` 需要复制或重构整个路由注册块（204-315 行）。无版本迁移策略文档。

### 2.11 Webhook 系统

**状态：基础出站 webhook**

`WebhookNotifyHandler`（`src/worker/handlers/webhook.rs`）通过 job queue 发起 HTTP POST，10 秒超时，非 2xx 触发重试。

**缺口：**

- 无 webhook 订阅管理 API（无法动态注册/注销 URL）
- 无 HMAC 签名（接收方无法验证真实性）
- 无事件类型过滤（无法订阅特定事件）
- 无死信队列 UI（永久失败的 webhook 无检查/重放界面）
- 无出站限流

### 2.12 审计日志

**状态：缺失——无持久化审计轨迹**

- **无 `audit_log` 表**——所有 migration 中均不存在
- **无 audit 模块**——`src/` 中无任何 audit 相关文件
- **EventBus 无持久化订阅者**——唯一订阅者转发给插件系统，不做审计持久化
- **Admin 操作无日志记录**——用户角色变更、插件启停、配置修改、租户 CRUD、RBAC 变更、Cron 调度变更均无审计记录
- **无 "谁做了什么、什么时候" 追踪**

---

## 3. 快速扩展能力评估

### 能快速扩展的

| 扩展方向 | 说明 |
|----------|------|
| 新 Content Type | 创建 TOML 文件即得自动建表 + CRUD API + Admin 页面 |
| 新插件 | 写 plugin.toml + 入口文件，自动加载 |
| 新租户 | INSERT 一行数据即生效 |
| 前端新页面 | Admin 页面复用统一 layout + API client |
| 新字段类型 | FieldType 枚举加一个变体 + 前端一个组件 |

### 扩展时会卡住的

| 场景 | 卡点 |
|------|------|
| 高并发 | 缓存不完整 + 无 Redis 后端 |
| 多实例部署 | 限流/缓存全内存不共享 |
| 安全合规 | 无审计日志 |
| 运维排查 | 无分布式追踪、无 metrics |
| API 演进 | 无版本策略 |
| 外部集成 | Webhook 无订阅管理 |

---

## 4. 改进优先级建议

| 优先级 | 改进项 | 工作量估计 | 收益 |
|--------|--------|-----------|------|
| **P0** | 审计日志（EventBus 持久化订阅者 + audit_log 表） | 2-3 天 | 安全合规必需 |
| **P0** | 可观测性（`#[instrument]` + Prometheus metrics + request ID） | 3-5 天 | 运维必需 |
| **P0** | 缓存补全（Category/Tag/Options 缓存 + InvalidateCacheHandler 实现） | 2-3 天 | 性能扩展基础 |
| **P1** | 测试补全（Options/RBAC/Stats/Plugin/Tenant 集成测试） | 3-5 天 | 质量保障 |
| **P1** | Webhook 订阅管理 API + HMAC 签名 | 2-3 天 | 外部集成能力 |
| **P2** | AppError 补全（429/413/405 变体） | 0.5 天 | 一致性 |
| **P2** | 事务补全（media 删除、options 批量、content type 校验+写入） | 1-2 天 | 数据一致性 |
| **P2** | Redis 缓存/限流后端 | 3-5 天 | 多实例部署 |
| **P2** | API 版本策略文档 + v2 路由架构 | 1-2 天 | 长期演进 |
| **P3** | 分页补全（tags/categories 加分页） | 0.5 天 | 性能 |
| **P3** | 输入校验补全（status 枚举、UUID 格式、MIME 白名单） | 1 天 | 安全 |
| **P3** | DB 连接池配置完善（acquire_timeout、min_connections） | 0.5 天 | 稳定性 |

---

## 5. 与同类系统对比

| 能力 | 本系统 | Strapi | WordPress |
|------|--------|--------|-----------|
| Schema-driven Content Type | TOML → 自动建表/API | JSON Schema → 自动建表/API | PHP 注册 → 自动建表 |
| 插件系统 | WASM/JS/Lua 三引擎沙箱 | JS 插件（无沙箱） | PHP 插件（无沙箱） |
| 多租户 | tenant_id 隔离 | 单租户（Enterprise 有） | Multisite |
| RBAC | 细粒度 + 条件权限 | 细粒度 | 基于角色 |
| 全文搜索 | Tantivy（内置） | 外部 | 外部 |
| 审计日志 | 缺失 | 有 | 有插件 |
| 可观测性 | 缺失 | 有 | 有插件 |
| API 版本 | 无 | 无 | 无 |
| Webhook | 基础出站 | 有 | 有插件 |
| 缓存 | 仅 posts | 内存 + Redis 可选 | 对象缓存 + Redis |
| Admin UI | Next.js（前后端分离） | React SPA | PHP SSR |
| 性能（RPS） | Rust 级（待压测） | Node.js 级 | PHP 级 |
