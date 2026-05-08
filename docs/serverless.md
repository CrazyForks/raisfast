# raisfast Serverless 部署适配方案

> 版本：1.0 · 日期：2026-05-08
>
> 目标：一套代码同时支持传统服务器和 Serverless 部署，
> 通过 `serverless` feature flag 零改动切换运行模式。

---

## 1. 产品定位

### 1.1 市场空白

| 竞品 | 语言 | Serverless | 冷启动 | 单实例 RPS | 插件系统 |
|------|------|------------|--------|-----------|---------|
| Strapi | Node.js | Vercel 有限 | ~500ms | ~2K | JS only |
| Payload | Node.js | Vercel only | ~500ms | ~2K | JS only |
| Directus | Node.js | 不支持 | ~800ms | ~1.5K | JS only |
| Ghost | Node.js | 不支持 | ~600ms | ~1K | 闭源 |
| WordPress | PHP | 不支持 | ~1s | ~500 | PHP only |
| **raisfast** | **Rust** | **全平台** | **<5ms** | **50K+** | **WASM/JS/Lua** |

**核心差异化**：Rust 性能 + Serverless 部署 + 多语言插件 + 多数据库 = 市场唯一。

### 1.2 支持的部署目标

| 平台 | 运行时 | 数据库 | 存储 | 冷启动 |
|------|--------|--------|------|--------|
| AWS Lambda | `provided` (AL2) | RDS / Aurora | S3 | <5ms |
| Vercel Edge | `wasm32` | Vercel Postgres | R2 / S3 | <5ms |
| Cloudflare Workers | `wasm32` | D1 / Hyperdrive | R2 | <5ms |
| Netlify Functions | `provided` | Neon / PlanetScale | S3 | <10ms |
| Fly.io / Railway | Docker | 内置 PostgreSQL | Volume | <5ms |
| 传统 VPS / Docker | 原生 | SQLite / PG / MySQL | 本地 / S3 | N/A |

---

## 2. 架构总览

### 2.1 双模式架构

```
┌─────────────────────────────────────────────────────────┐
│                    业务逻辑层（共享）                      │
│  models/ · services/ · handlers/ · content_type/        │
│  repositories/ · middleware/ · errors/ · oauth/          │
├─────────────────────────────────────────────────────────┤
│                   IO 抽象层（trait）                       │
│  Storage · CacheStore · SearchEngine · RateLimitStore    │
│  EventBus · JobQueue · EmailSender · SmsSender           │
├──────────────────┬──────────────────────────────────────┤
│  传统模式（默认）  │         Serverless 模式               │
├──────────────────┼──────────────────────────────────────┤
│ TcpListener       │ Lambda handler / Vercel / Workers    │
│ LocalStorage      │ S3Storage / R2Storage                │
│ MemoryCache       │ RedisCache / D1Cache                  │
│ MemoryRateLimit   │ RedisRateLimit                        │
│ Tantivy (disk)    │ Tantivy (RAM) / Meilisearch          │
│ tokio::spawn ×10  │ 请求内同步执行                         │
│ broadcast EventBus│ 同步 fan-out / 外部队列                │
│ 文件日志           │ stdout JSON only                      │
│ plugin 磁盘加载    │ 编译时嵌入 / DB 加载                   │
│ ServeDir          │ CDN / S3 presigned URL                │
└──────────────────┴──────────────────────────────────────┘
```

### 2.2 编译切换

```toml
# 传统服务器（默认）
cargo build

# Serverless 模式
cargo build --features serverless --no-default-features

# Serverless + PostgreSQL + S3
cargo build --features "serverless db-postgres storage-s3" --no-default-features
```

---

## 3. 改造清单

### 3.1 Feature Flag 设计

```toml
[features]
default = ["db-sqlite", "search-tantivy", "plugin-js", "plugin-lua", "openapi"]
serverless = []                    # 启用 serverless 运行模式

# IO 后端（可独立选择）
storage-s3     = ["aws-sdk-s3", "aws-config"]
cache-redis    = ["redis"]         # 新增
rate-limit-redis = ["redis"]       # 新增
search-external = []               # 新增：外部搜索服务（Meilisearch 等）
```

`serverless` feature 本身不引入新依赖，只改变**运行时行为**：

- 禁用所有 `tokio::spawn` 后台任务
- 事件处理从异步订阅改为同步 inline
- 日志只输出 stdout
- Worker/Cron 改为请求内单次触发

---

### 3.2 改造项详细清单

按依赖关系排序，前面的项是后续项的前置条件。

---

#### 3.2.1 运行模式开关

**文件**: `src/config/app.rs`

新增字段：

```rust
pub serverless: bool,   // 读 SERVERLESS 环境变量，默认 false
```

**效果**：所有需要条件行为的代码通过 `config.serverless` 判断，
而非到处写 `#[cfg(feature = "serverless")]`。编译时 feature 控制依赖，
运行时 config 控制行为。

---

#### 3.2.2 HTTP 入口适配器

**现状**: `src/server.rs` 使用 `TcpListener::bind` + `axum::serve`。

**改造**: 将 Router 构建与服务器启动分离。

```
src/server.rs (当前):
  start() → TcpListener → axum::serve(router)

改造后:
  build_app() → Router          ← 共享，不变
  start_standalone()             ← 传统模式
  start_serverless()             ← 新增：返回 Router 或 handler
```

**新增文件**: `src/serverless.rs`

| 平台 | 入口 | 说明 |
|------|------|------|
| AWS Lambda | `lambda_http::run(router.into_service())` | 使用 `aws-lambda-rust-runtime` |
| Vercel | `#[vercel_runtime]` handler | 使用 `vercel_runtime` crate |
| Cloudflare Workers | `workers-rs` entry | 使用 `workers` crate |
| 通用 | 导出 `make_router()` 函数 | 任何平台调用即可 |

**关键**：axum Router 是纯数据结构，与运行时无关。
只需将 Router 转为各平台的 handler 类型。

---

#### 3.2.3 后台任务同步化

**现状**: 8-10 个 `tokio::spawn` + `loop {}` 死循环。

| 任务 | 文件 | Serverless 方案 |
|------|------|----------------|
| 事件→插件 Hook | `server.rs:1365` | `emit_with_aspects()` 已同步执行 Hook，改 EventBus::emit() 也同步调用插件 |
| 事件→审计日志 | `server.rs:1418` | 同步写入 audit_log 表（INSERT 很快） |
| 事件→Webhook 投递 | `server.rs:1574` | 同步 HTTP POST + 超时 5s，失败记入 jobs 表 |
| 事件→Job 入队 | `worker/enqueuer.rs:24` | 同步入队（INSERT 很快） |
| Cron 调度器 | `worker/scheduler.rs:353` | 每次请求检查 + 外部 cron trigger |
| Worker 执行器 | `worker/runner.rs:46` | 请求内单次 dequeue + execute |
| Rate-limit 清理 | `server.rs:1155` | 在 check() 内惰性清理，不启动后台任务 |
| 日志清理 | `main.rs:37` | 不需要（serverless 无文件日志） |
| 插件文件监视 | `plugins.rs:350` | 不启动（serverless 禁用 hot-reload） |

**核心改造**: `EventBus` 新增 `emit_sync()` 方法，在 `serverless` 模式下
直接调用所有订阅者（审计、webhook、入队），不经过 broadcast channel。

```
传统模式:  emit() → broadcast channel → 各 spawn 的 subscriber
Serverless: emit_sync() → 直接调用 audit_write() + webhook_deliver() + job_enqueue()
```

---

#### 3.2.4 文件系统消除

**原则**: 所有文件 IO 走已有的 `Storage` trait，不直接调用 `std::fs`。

| 当前位置 | 用途 | 改造方案 |
|----------|------|----------|
| `storage/local.rs` | 上传文件 | 已有 S3Storage，serverless 默认用 S3 |
| `worker/handlers/thumbnail.rs:70` | 缩略图 | 改调 `Storage::put()` |
| `worker/handlers/sitemap.rs:80` | sitemap.xml | 改调 `Storage::put()` |
| `content_type/schema.rs:850-855` | schema TOML | 改为 DB 存储或编译时嵌入 |
| `content_type/schema.rs:1343` | 迁移 SQL 文件 | 改为 DB 存储或编译时嵌入 |
| `search/tantivy.rs:35` | 搜索索引目录 | 改为 `Index::create_in_ram` |
| `logging.rs:62,102` | 日志文件 | serverless 模式只输出 stdout |
| `plugins.rs:442+` | 插件磁盘加载 | 编译时嵌入或从 DB/Storage 加载 |
| `server.rs:1081` | ServeDir 静态文件 | 改为 CDN / S3 presigned URL 重定向 |
| `cli/server_cmd.rs:18` | PID 文件 | serverless 不写 PID |

---

#### 3.2.5 日志系统

**现状**: `src/logging.rs` — DailyRollingWriter 写 JSON 日志到 `./logs/`。

**改造**:

```rust
// logging.rs
if config.serverless {
    // 只初始化 stdout subscriber，不创建文件 appender
    init_stdout_only();
} else {
    // 当前逻辑：stdout + 文件
    init_with_file_logging();
}
```

Serverless 平台自动捕获 stdout：
- AWS Lambda → CloudWatch Logs
- Vercel → Vercel Logs
- Cloudflare Workers → Workers Logs

---

#### 3.2.6 搜索引擎

**现状**: `search-tantivy` feature 使用本地文件系统索引。

**改造方案**:

| 方案 | 适用场景 | 工作量 |
|------|----------|--------|
| `Index::create_in_ram` | 小型站点（<10万文档） | 低 |
| Meilisearch / OpenSearch | 大型站点 | 中 |
| Cloudflare Workers AI | Workers 部署 | 中 |

新增 `search-external` feature，通过 HTTP API 调用外部搜索服务。

`SearchEngine` trait 已有，只需新增实现。

---

#### 3.2.7 缓存系统

**现状**: 只有 `MemoryCache`（moka）。

**改造**: 新增 `cache-redis` feature。

```rust
// cache.rs 新增
#[cfg(feature = "cache-redis")]
pub struct RedisCache { /* redis::aio::Connection */ }

#[cfg(feature = "cache-redis")]
impl CacheStore for RedisCache {
    async fn get(&self, key: &str) -> Option<String> { /* GET */ }
    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) { /* SET EX */ }
    // ...
}
```

`CacheStore` trait 不需要改，只新增实现。

---

#### 3.2.8 限流系统

**现状**: 只有 `MemoryStore`（DashMap），跨进程不共享。

**改造**: 新增 `rate-limit-redis` feature。

```rust
// rate_limit.rs 新增
#[cfg(feature = "rate-limit-redis")]
pub struct RedisRateLimitStore { /* ... */ }

// 滑动窗口算法：SORTED SET + ZADD + ZCOUNT + ZREMRANGEBYSCORE
```

---

#### 3.2.9 插件系统

**现状**: 从 `{plugin_dir}/` 磁盘加载 + `notify` 文件监视热更新。

**改造**:

| 改造项 | 方案 |
|--------|------|
| 插件源 | 编译时嵌入（`include_bytes!`）或从 DB/Storage 加载 |
| 热更新 | serverless 模式禁用 |
| WASM 编译缓存 | `OnceLock` 懒初始化，冷启动时编译一次 |
| JS/Lua VM | `OnceLock` 懒初始化 |
| 插件 metrics | 写入 DB 或丢弃 |

**关键**：冷启动时加载插件的开销。
WASM 编译约 50-100ms/module，JS/Lua VM 约 10-20ms。
插件数量 < 20 时，总开销 < 500ms，可接受。

---

#### 3.2.10 静态资源服务

**现状**: `ServeDir` 直接从磁盘提供 `/uploads/*` 和 `/static/*`。

**改造**:

```
/uploads/{key}  →  302 重定向到 Storage::url() (S3/R2 presigned URL)
/static/*       →  编译时嵌入或 CDN
/admin/*        →  rust-embed（已实现）
```

---

#### 3.2.11 Content Type Schema

**现状**: 从 `{content_type_dir}/*.toml` 文件读取 schema 定义。

**改造**: 新增 `content_type_source` 配置项：

```env
# 传统模式
CONTENT_TYPE_SOURCE=dir
CONTENT_TYPE_DIR=./content_types

# Serverless 模式
CONTENT_TYPE_SOURCE=db        # 从 content_types 表读取
```

需要新增 DB 表存储 schema 定义（TOML 文本）。
首次启动时自动导入已有的 TOML 文件到 DB。

---

### 3.3 改造项依赖关系

```
3.2.1 运行模式开关 (config.serverless)
  │
  ├── 3.2.2 HTTP 入口适配器
  │
  ├── 3.2.3 后台任务同步化
  │     └── 依赖：EventBus 改造
  │
  ├── 3.2.4 文件系统消除
  │     ├── thumbnail/sitemap → Storage::put()
  │     ├── 搜索索引 → in-ram
  │     ├── content-type schema → DB
  │     └── plugin → 编译时嵌入/DB
  │
  ├── 3.2.5 日志系统 (stdout only)
  │
  ├── 3.2.6 搜索引擎 (in-ram / external)
  │
  ├── 3.2.7 缓存系统 (Redis)
  │
  ├── 3.2.8 限流系统 (Redis)
  │
  ├── 3.2.9 插件系统 (懒加载 / 禁用热更新)
  │
  ├── 3.2.10 静态资源 (CDN/S3)
  │
  └── 3.2.11 Content Type Schema (DB)
```

---

## 4. 实施路线

### Phase 1：核心跑通（3 天）

让 raisfast 在 AWS Lambda 上跑通一个完整请求。

| 步骤 | 内容 | 预计耗时 |
|------|------|----------|
| 1 | `serverless` feature flag + `config.serverless` | 2h |
| 2 | 日志 stdout-only 模式 | 1h |
| 3 | 禁用所有 `tokio::spawn` 后台任务 | 2h |
| 4 | EventBus 同步模式 (`emit_sync`) | 4h |
| 5 | Worker/Cron 请求内单次触发 | 4h |
| 6 | thumbnail/sitemap 改走 Storage::put() | 2h |
| 7 | Tantivy in-ram 模式 | 2h |
| 8 | `ServeDir` → S3 redirect | 2h |
| 9 | AWS Lambda 适配器 + 端到端测试 | 4h |

**里程碑**：`curl https://xxx.lambda-url.aws.com/api/v1/posts` 返回数据。

### Phase 2：状态外部化（3 天）

消除所有进程内状态依赖。

| 步骤 | 内容 | 预计耗时 |
|------|------|----------|
| 1 | Redis CacheStore 实现 | 1d |
| 2 | Redis RateLimitStore 实现 | 1d |
| 3 | Content Type Schema DB 存储 | 0.5d |
| 4 | Plugin 编译时嵌入 | 0.5d |

**里程碑**：多实例并发请求，缓存/限流/配置一致。

### Phase 3：多平台适配（3 天）

| 步骤 | 内容 | 预计耗时 |
|------|------|----------|
| 1 | Vercel 适配器 | 1d |
| 2 | Cloudflare Workers 适配器 | 1d |
| 3 | Meilisearch SearchEngine 实现 | 0.5d |
| 4 | 文档 + 部署模板 (SAM / Serverless Framework) | 0.5d |

**里程碑**：一键部署到 AWS / Vercel / Cloudflare。

### Phase 4：生产优化（持续）

- 冷启动优化：lazy init → eager preload
- 外部搜索：OpenSearch / Elasticsearch / Typesense
- 外部队列：SQS / EventBridge / Cloudflare Queues
- CDN 集成：CloudFront / Cloudflare CDN
- 监控：OpenTelemetry / Prometheus remote write

---

## 5. 技术风险与应对

| 风险 | 影响 | 应对 |
|------|------|------|
| 冷启动慢（插件编译） | >2s 时用户体验差 | 预编译 WASM 到 DB；保持实例温热（最低并发 1） |
| SQLite 不支持并发写入 | Lambda 多实例冲突 | Serverless 模式强制 PostgreSQL/D1 |
| 请求内同步 webhook 增加延迟 | 慢 webhook 拖慢请求 | 超时 5s + 异步入队失败重试 |
| Redis 额外依赖 | 部署复杂度增加 | 提供 MemoryCache 降级模式（单实例可用） |
| Tantivy in-ram 内存占用 | 大索引 OOM | 限制索引大小 / 使用外部搜索 |
| Content Type Schema 从 DB 加载变慢 | 冷启动 +1s | OnceLock 缓存 + 懒加载 |

---

## 6. 配置参考

### 6.1 传统服务器模式

```env
DATABASE_URL=sqlite:./data/raisfast.db
STORAGE_DRIVER=local
UPLOAD_DIR=./uploads
LOG_DIR=./logs
WORKER_ENABLED=true
PLUGIN_DIR=./plugins
CONTENT_TYPE_DIR=./content_types
BUILTIN_TENANTABLE=false
```

### 6.2 AWS Lambda + PostgreSQL + S3

```env
SERVERLESS=true
DATABASE_URL=postgres://user:pass@aurora-cluster.region.rds.amazonaws.com/raisfast
STORAGE_DRIVER=s3
S3_BUCKET=raisfast-media
AWS_REGION=us-east-1
REDIS_URL=redis://cache.xxxxxx.ng.0001.use1.cache.amazonaws.com:6379
SEARCH_DRIVER=tantivy    # in-ram
WORKER_ENABLED=false     # 请求内触发
PLUGIN_HOT_RELOAD=false
CONTENT_TYPE_SOURCE=db
LOG_DRIVER=stdout
```

### 6.3 Cloudflare Workers + D1 + R2

```env
SERVERLESS=true
DATABASE_URL=d1://raisfast-db
STORAGE_DRIVER=r2
R2_BUCKET=raisfast-media
SEARCH_DRIVER=noop
WORKER_ENABLED=false
PLUGIN_HOT_RELOAD=false
CONTENT_TYPE_SOURCE=db
LOG_DRIVER=stdout
```

---

## 7. 新增 trait / 实现

### 7.1 EventBus 同步模式

```rust
impl EventBus {
    /// Serverless 模式：同步调用所有订阅者
    pub fn emit_sync(&self, event: Event, handlers: &[SyncEventHandler]) {
        for handler in handlers {
            let _ = handler(event.clone());  // 忽略错误，继续下一个
        }
    }
}
```

### 7.2 RedisCache（新增）

```rust
#[cfg(feature = "cache-redis")]
pub struct RedisCache {
    client: redis::Client,
}

#[cfg(feature = "cache-redis")]
impl CacheStore for RedisCache {
    async fn get(&self, key: &str) -> Option<String>;
    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>);
    async fn delete(&self, key: &str);
    async fn delete_prefix(&self, prefix: &str) -> u64;
}
```

### 7.3 RedisRateLimitStore（新增）

```rust
#[cfg(feature = "rate-limit-redis")]
pub struct RedisRateLimitStore {
    client: redis::Client,
}

#[cfg(feature = "rate-limit-redis")]
impl RateLimitStore for RedisRateLimitStore {
    async fn check(&self, key: &str, config: &RateLimitConfig) -> bool;
    async fn cleanup_expired(&self, window_secs: u64);
}
```

### 7.4 ExternalSearchEngine（新增）

```rust
#[cfg(feature = "search-external")]
pub struct ExternalSearchEngine {
    base_url: String,
    api_key: String,
}

#[cfg(feature = "search-external")]
impl SearchEngine for ExternalSearchEngine {
    // HTTP 调用 Meilisearch / OpenSearch / Elasticsearch
}
```

---

## 8. 预期性能

### 8.1 冷启动时间估算

| 阶段 | 耗时 | 说明 |
|------|------|------|
| 二进制加载 | <1ms | Rust 编译为原生码 |
| DB 连接池 | 5-20ms | PostgreSQL TCP 握手 |
| Schema 检查 | <1ms | `_migrations` 表已存在 → 跳过 |
| Options 加载 | 2-5ms | 首次 SELECT → 缓存 |
| Content Type 加载 | 5-10ms | 从 DB 读 TOML → 解析 |
| Plugin 加载 | 50-200ms | WASM 编译 + JS/Lua VM init |
| Tantivy 索引 | 50-500ms | 从 DB 重建 RAM 索引（视数据量） |
| **总计** | **100-750ms** | 首次请求；后续请求 <1ms |

对比：
- Node.js CMS 冷启动：500ms-2s
- Go CMS 冷启动：50-200ms
- raisfast 冷启动：100-750ms（插件/索引加载是瓶颈）

### 8.2 热请求性能

| 指标 | 传统模式 | Serverless |
|------|----------|-----------|
| API 响应 P50 | <1ms | <2ms（多一次 Redis/DB） |
| API 响应 P99 | <5ms | <10ms |
| 单实例 RPS | 50K+ | 受平台限制 |

---

## 9. 文件改动总览

### 新增文件

| 文件 | 说明 |
|------|------|
| `src/serverless.rs` | Serverless 入口适配器 |
| `src/cache/redis.rs` | Redis CacheStore 实现 |
| `src/middleware/rate_limit_redis.rs` | Redis RateLimitStore 实现 |
| `src/search/external.rs` | 外部搜索服务实现 |
| `src/storage/r2.rs` | Cloudflare R2 Storage 实现 |
| `src/content_type/db_loader.rs` | Content Type 从 DB 加载 |
| `src/plugins/embedded.rs` | 编译时插件嵌入 |

### 修改文件

| 文件 | 改动 |
|------|------|
| `Cargo.toml` | 新增 `serverless`、`cache-redis`、`rate-limit-redis`、`search-external` features |
| `src/config/app.rs` | 新增 `serverless`、`log_driver`、`content_type_source` 字段 |
| `src/logging.rs` | 新增 stdout-only 模式 |
| `src/eventbus.rs` | 新增 `emit_sync()` 方法 |
| `src/server.rs` | 条件禁用 `tokio::spawn`，Router 与启动分离 |
| `src/lib.rs` | 条件选择缓存/限流/存储后端 |
| `src/main.rs` | 条件禁用日志清理 loop |
| `src/worker/handlers/thumbnail.rs` | 改走 Storage::put() |
| `src/worker/handlers/sitemap.rs` | 改走 Storage::put() |
| `src/search/tantivy.rs` | 新增 in-ram 模式 |
| `src/plugins.rs` | 条件禁用 hot-reload，新增嵌入式加载 |
| `src/content_type/schema.rs` | 新增 DB 加载路径 |
| `src/storage.rs` | 新增 R2 factory |
| `src/cache.rs` | 新增 Redis 实现 |
| `src/middleware/rate_limit.rs` | 新增 Redis 实现 |

### 不改动的文件

| 目录/文件 | 说明 |
|-----------|------|
| `src/models/` | 数据结构，纯逻辑 |
| `src/services/` | 业务逻辑，已通过 trait 隔离 IO |
| `src/handlers/` | 路由处理，只调 service + 返回 JSON |
| `src/repositories/` | 数据访问，已用 sqlx + dialect 抽象 |
| `src/errors/` | 错误定义，纯逻辑 |
| `src/oauth/` | OAuth 流程，纯 HTTP |
| `src/db/dialect.rs` | SQL 方言，已适配多 DB |
| `src/macros.rs` | 宏定义，纯逻辑 |
| `migrations/` | Schema 文件，不变 |
| `frontend/admin/` | Admin SPA，不变 |

---

## 10. 结论

### 可行性：✅ 完全可行

项目已具备良好抽象基础：
- `Storage` / `CacheStore` / `SearchEngine` / `RateLimitStore` / `JobQueue` trait 全部就位
- Repository 层已通过 `Pool` 类型别名 + `dialect::translate()` 实现多 DB 适配
- 业务逻辑与 IO 层完全分离

### 风险：可控

核心风险只有冷启动延迟，通过以下手段可控：
- 保持温热实例（所有平台都支持）
- 预编译插件（消除 WASM 编译开销）
- 懒加载搜索索引（首次搜索时才构建）

### 投入产出

- **开发投入**：~9 个工作日
- **市场价值**：成为唯一支持 Serverless 的 Rust Headless CMS
- **用户价值**：零运维 + 按需付费 + 自动扩缩容 + 全球边缘部署
