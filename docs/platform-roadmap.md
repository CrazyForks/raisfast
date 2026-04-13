# 平台化演进路线图

> 基于当前架构的扩展性评估，分析从「博客 API」演进为「可扩展平台」的差距与路径。

---

## 1. 当前架构评估

### 1.1 已具备的扩展基础

| 维度 | 现状 | 评级 |
|------|------|------|
| 分层架构 | Handler → Service → Model，职责清晰 | 优 |
| 插件系统 | WASM 沙箱、11 个 Hook、热加载、安全模型 | 优 |
| 多数据库 | SQLite/PostgreSQL/MySQL + 编译时 SQL 检查 | 优 |
| 错误处理 | 统一 AppError + i18n + IntoResponse | 优 |
| 认证 | JWT + Refresh Token + 角色权限 | 良 |
| 测试覆盖 | 125 个测试（65 单元 + 60 集成） | 良 |
| 代码质量 | `#![deny(unsafe_code)]`、零 clippy 警告 | 优 |

### 1.2 核心判断

当前系统是一个**功能完整的博客 API**，但还不是一个**平台框架**。

差距在于缺少一层「平台基础设施」：

```
当前:  HTTP → Handler → Service → Model → DB
                                 ↓
                              Plugin (WASM)

目标:  HTTP → Handler → Service → Model → DB
                ↓           ↓        ↓
             Cache      EventBus   FullText
              (Redis)   (通知/任务) (Meilisearch)
                ↓           ↓
             Plugin     Background
             System      Workers
```

---

## 2. 必须补齐的短板

### P0 — 不补就做不了复杂平台

#### 2.1 缓存层（Redis）

**现状：** 无任何缓存。所有查询直接打到数据库。

**问题：** 热点数据（文章列表、分类树、标签云）在高并发下会成为瓶颈。复杂平台的配置、会话、计数器都需要缓存。

**方案：**

```rust
// src/cache/mod.rs
#[async_trait]
pub trait CacheStore: Send + Sync {
    async fn get(&self, key: &str) -> Option<String>;
    async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> AppResult<()>;
    async fn delete(&self, key: &str) -> AppResult<()>;
    async fn incr(&self, key: &str, delta: i64) -> AppResult<i64>;
}

// src/cache/memory.rs — 开发阶段用
pub struct MemoryCache { /* HashMap + TTL */ }

// src/cache/redis.rs — 生产环境用
pub struct RedisCache { /* redis::aio::Connection */ }
```

**集成点：**

- `services/post.rs` — 文章详情缓存（slug → post），列表缓存（分页参数 → 结果集）
- `services/auth.rs` — Token 黑名单、用户权限缓存
- `services/comment.rs` — 评论计数缓存
- `middleware/rate_limit.rs` — 已有 `RateLimitStore` trait，补充 `RedisStore` 实现

#### 2.2 后台任务队列

**现状：** 无异步任务机制。所有操作在请求线程同步完成。

**问题：** 定时发布、邮件发送、图片处理、统计分析、Webhook 回调都需要异步执行。

**方案：**

```rust
// src/worker/mod.rs
#[async_trait]
pub trait JobQueue: Send + Sync {
    async fn enqueue(&self, job: Job) -> AppResult<()>;
    async fn dequeue(&self) -> Option<Job>;
}

pub enum Job {
    SendEmail { to: String, subject: String, body: String },
    ProcessImage { media_id: String, operations: Vec<ImageOp> },
    ScheduledPublish { post_id: String, publish_at: String },
    WebhookNotify { url: String, payload: serde_json::Value },
    RebuildSearchIndex { post_ids: Vec<String> },
    GenerateSitemap,
}
```

**实现选择：**

| 方案 | 优点 | 缺点 |
|------|------|------|
| `tokio::spawn` + 内存队列 | 零依赖，简单 | 进程重启丢失 |
| SQLite 持久化队列 | 无额外依赖，可靠 | 性能上限低 |
| Redis Stream | 成熟、支持消费者组 | 引入 Redis 依赖 |
| 独立 sidekiq-rs / tower-worker | 专业方案 | 重量级 |

**建议：** 先实现 SQLite 持久化队列（与现有架构一致），后期按需切换 Redis Stream。

#### 2.3 全站搜索

**现状：** `services/post.rs` 使用 `LIKE '%keyword%'` 全表扫描，无索引利用。

**问题：** 文章量上千后搜索变慢，无法支持高亮、分词、相关性排序。

**方案 A — SQLite FTS5（推荐起步）：**

```sql
-- migrations/003_fts5.sql
CREATE VIRTUAL TABLE IF NOT EXISTS posts_fts USING fts5(
    title,
    content,
    content=posts,
    content_rowid=rowid
);

-- 触发器自动同步
CREATE TRIGGER posts_ai AFTER INSERT ON posts BEGIN
    INSERT INTO posts_fts(rowid, title, content) VALUES (new.rowid, new.title, new.content);
END;
```

```rust
// models/search.rs
pub async fn search_posts(pool: &Pool, query: &str, page: i64, page_size: i64) -> AppResult<(Vec<PostSearchResult>, i64)> {
    // SELECT * FROM posts_fts WHERE posts_fts MATCH ? ORDER BY rank LIMIT ? OFFSET ?
}
```

**方案 B — Meilisearch（后期升级）：**

当文章量超过 10 万或需要中文分词时切换。通过插件 Hook `OnPostCreated`/`OnPostUpdated`/`OnPostDeleted` 同步索引。

#### 2.4 数据库事务

**现状：** `sync_tags` 等多步操作缺少事务包裹，中间失败会导致数据不一致。

**修复：**

```rust
// services/post.rs
pub async fn create_post(pool: &Pool, /* ... */) -> AppResult<PostResponse> {
    let mut tx = pool.begin().await?;

    let post = sqlx::query_scalar!("INSERT INTO posts ...")
        .fetch_one(&mut *tx)
        .await?;

    for tag_id in &tag_ids {
        sqlx::query!("INSERT INTO posts_tags ...", post.id, tag_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;
    Ok(post)
}
```

---

### P1 — 不补就做不好

#### 2.5 实时推送（WebSocket / SSE）

**现状：** 纯请求-响应模式，无服务端推送能力。

**场景：** 新评论通知作者、审核状态变更通知、在线访客计数、协作编辑。

**方案：**

```rust
// src/server/mod.rs — 新增 WebSocket 路由
.route("/ws", get(ws_handler))
.route("/api/v1/events", get(sse_handler))

// src/services/event.rs
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

pub enum Event {
    NewComment { post_slug: String, comment: CommentResponse },
    PostPublished { post: PostResponse },
    SystemNotification { message: String },
}
```

**推荐 SSE（Server-Sent Events）起步：**

- 比 WebSocket 简单，单向推送足够
- 浏览器原生支持，无需额外库
- axum 集成简单（`axum::response::sse::Sse`）

#### 2.6 通知系统

**现状：** 无任何通知机制。

**方案：**

```rust
// src/services/notification.rs
pub struct NotificationService {
    email: Option<EmailSender>,
    webhook: Option<WebhookSender>,
    in_app: InAppNotifier,
}

pub trait Notifier: Send + Sync {
    async fn send(&self, notification: &Notification) -> AppResult<()>;
}

pub struct Notification {
    pub recipient: String,
    pub event: NotificationEvent,
    pub channels: Vec<Channel>, // Email, InApp, Webhook
}
```

**通知触发点（复用现有 Hook）：**

| 事件 | 通知对象 | 渠道 |
|------|---------|------|
| 新评论 | 文章作者 | InApp + Email |
| 评论审核通过 | 评论者 | Email |
| 文章发布 | 订阅者 | Email + Webhook |
| 用户注册 | 管理员 | InApp |
| 登录异常 | 用户 | Email |

#### 2.7 媒体处理管线

**现状：** 文件原样存储，无压缩、缩略图、格式转换。

**方案：**

```rust
// src/services/media.rs — 扩展
pub enum ImageOperation {
    Resize { width: u32, height: u32 },
    ConvertToWebP,
    GenerateThumbnail { size: u32 },
    Compress { quality: u8 },
}

pub async fn process_upload(
    pool: &Pool,
    user_id: &str,
    file: Bytes,
    filename: &str,
) -> AppResult<MediaResponse> {
    // 1. 校验文件类型和大小
    // 2. 图片：生成缩略图 + WebP 转换 + 压缩
    // 3. 存储原始文件和处理后文件
    // 4. 记录数据库
    // 5. 可选：上传到 S3
}
```

#### 2.8 API 版本化策略

**现状：** `/api/v1` 硬编码前缀，无版本管理策略。

**方案：**

```rust
// server/mod.rs
let api_v1 = axum::Router::new()
    .route("/posts", get(post::list).post(post::create))
    // ...

let api_v2 = axum::Router::new()
    .route("/posts", get(post_v2::list).post(post_v2::create))
    // ... breaking changes

let app = axum::Router::new()
    .nest("/api/v1", api_v1)
    .nest("/api/v2", api_v2)
```

**版本兼容原则：**

- 新增字段：向前兼容，不算 breaking change
- 删除/重命名字段：新版本
- 新增端点：在当前版本添加
- 废弃端点：响应头 `Deprecation: true`，至少保留一个大版本

#### 2.9 审计日志

**现状：** 无操作记录。

**方案：**

```sql
-- migrations/004_audit_log.sql
CREATE TABLE IF NOT EXISTS audit_logs (
    id TEXT PRIMARY KEY,
    user_id TEXT,
    action TEXT NOT NULL,        -- 'post.create', 'user.update_role', 'comment.delete'
    resource_type TEXT NOT NULL, -- 'post', 'user', 'comment', 'media'
    resource_id TEXT,
    detail TEXT,                 -- JSON 格式的变更详情
    ip_address TEXT,
    user_agent TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_logs(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_logs(created_at);
```

```rust
// src/middleware/audit.rs
pub async fn audit_middleware(
    // 记录所有写操作（POST/PUT/DELETE）
    // 自动提取 user_id、action、resource 信息
)
```

---

### P2 — 做大时才需要

#### 2.10 水平扩展

**现状：** 单实例部署，内存存储（限流器、会话）。

**需要改造：**

| 组件 | 当前 | 目标 |
|------|------|------|
| 限流器 | `MemoryStore` | `RedisStore`（trait 已预留） |
| 会话/缓存 | 无 | Redis |
| 文件存储 | 本地磁盘 | S3 兼容对象存储 |
| 搜索 | SQLite FTS5 | Meilisearch（独立服务） |
| 任务队列 | 内存 | Redis Stream / 独立 worker |

**改造后架构：**

```
                    ┌─────────┐
                    │  Nginx  │ (负载均衡)
                    └────┬────┘
            ┌────────────┼────────────┐
            ▼            ▼            ▼
     ┌──────────┐ ┌──────────┐ ┌──────────┐
     │ App :3001│ │ App :3002│ │ App :3003│  (无状态实例)
     └────┬─────┘ └────┬─────┘ └────┬─────┘
          │             │             │
          └─────────────┼─────────────┘
                        │
              ┌─────────┼─────────┐
              ▼         ▼         ▼
          ┌──────┐ ┌────────┐ ┌──────┐
          │ Redis│ │ PostgreSQL│ │ S3  │
          └──────┘ └────────┘ └──────┘
```

#### 2.11 可观测性

**现状：** `tracing` 日志已有，但缺 metrics 和分布式追踪。

**方案：**

```toml
# Cargo.toml 新增
[dependencies]
prometheus = "0.13"
metrics = "0.24"
metrics-exporter-prometheus = "0.16"
```

**关键指标：**

| 类别 | 指标 |
|------|------|
| HTTP | 请求量、延迟分布、错误率（按路由/方法/状态码） |
| 数据库 | 连接池使用率、查询延迟、慢查询 |
| 插件 | 加载时间、Hook 执行时间、内存占用 |
| 业务 | 文章发布量、评论量、注册量、活跃用户 |
| 系统 | CPU、内存、磁盘、网络 |

#### 2.12 内容版本与草稿自动保存

**现状：** 只有 draft/published 两种状态，无版本历史。

**方案：**

```sql
-- migrations/005_post_revisions.sql
CREATE TABLE IF NOT EXISTS post_revisions (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL REFERENCES posts(id) ON DELETE CASCADE,
    revision_number INTEGER NOT NULL,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    excerpt TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(post_id, revision_number)
);
```

- 每次更新文章时自动创建一个 revision
- 支持对比任意两个版本
- 支持回滚到指定版本
- 草稿自动保存：前端定时 POST，后端 upsert

#### 2.13 CI/CD + Docker

**现状：** guide.md 有 Dockerfile 模板但未实际创建。

**需要创建：**

- `Dockerfile` — 多阶段构建（builder → runtime）
- `docker-compose.yml` — 开发环境（app + redis + postgres）
- `.github/workflows/ci.yml` — fmt + clippy + test + build
- `docker-compose.prod.yml` — 生产部署模板

---

## 3. 架构改造要点

### 3.1 事件总线（EventBus）

当前 Plugin Hook 是同步链式调用，无法解耦。平台级系统需要事件驱动架构。

```rust
// src/eventbus/mod.rs
use tokio::sync::broadcast;

pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
}

#[derive(Debug, Clone)]
pub enum Event {
    PostCreating { data: PostCreatingData },
    PostCreated { post_id: String },
    PostUpdated { post_id: String, old: PostSnapshot, new: PostSnapshot },
    PostDeleted { post_id: String },
    CommentCreated { comment_id: String, post_slug: String },
    UserRegistered { user_id: String },
    LoginAttempt { email: String, success: bool, ip: String },
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub async fn emit(&self, event: Event) {
        let _ = self.tx.send(Arc::new(event));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.tx.subscribe()
    }
}
```

**订阅者：**

```
EventBus.emit(PostCreated)
    ├── PluginManager.dispatch_action(PostCreated)  // 插件 Hook
    ├── NotificationService.notify(author)           // 邮件/站内通知
    ├── SearchIndexer.index(post)                    // 搜索索引同步
    ├── AuditLogger.log("post.create", ...)          // 审计日志
    └── CacheInvalidator.invalidate("posts:*")       // 缓存失效
```

### 3.2 Repository 抽象

当前 Model 层直接写 SQL，耦合数据库实现。复杂平台需要抽象层以便 mock、缓存、换引擎。

```rust
// src/repositories/mod.rs
#[async_trait]
pub trait PostRepository: Send + Sync {
    async fn find_by_slug(&self, slug: &str) -> AppResult<Option<Post>>;
    async fn list(&self, filter: PostFilter, page: i64, page_size: i64) -> AppResult<Paginated<Post>>;
    async fn create(&self, input: CreatePostInput) -> AppResult<Post>;
    async fn update(&self, id: &str, input: UpdatePostInput) -> AppResult<Post>;
    async fn delete(&self, id: &str) -> AppResult<()>;
}

// src/repositories/sqlx_post.rs
pub struct SqlxPostRepository { pool: Pool }

// src/repositories/cached_post.rs
pub struct CachedPostRepository<P: PostRepository> {
    inner: P,
    cache: Arc<dyn CacheStore>,
}
```

**好处：**

- `CachedPostRepository` 装饰器模式，为任意 Repository 添加缓存
- 测试时 mock `PostRepository`，隔离数据库
- 未来切换 ORM 或搜索引擎时只改 Repository 实现

### 3.3 配置热更新

当前 `AppConfig` 启动时一次性加载。平台级系统需要运行时配置变更。

```rust
// src/config/runtime.rs
pub struct RuntimeConfig {
    inner: Arc<RwLock<AppConfig>>,
}

impl RuntimeConfig {
    pub async fn reload(&self, new_config: AppConfig) {
        *self.inner.write().await = new_config;
    }

    pub async fn get(&self) -> tokio::sync::RwLockReadGuard<'_, AppConfig> {
        self.inner.read().await
    }
}
```

**可热更新的配置项：**

- 每页文章数
- 限流阈值
- 插件启用/禁用
- CORS 白名单
- 功能开关（Feature Flags）

---

## 4. 演进路线

### 阶段 1 — 基础设施（2-3 周）

```
Week 1:
  ├── SQLite 事务包裹关键操作（sync_tags、create_post、delete_post）
  ├── FTS5 全文搜索（零依赖，SQLite 内建）
  └── EventBus trait + broadcast 实现

Week 2:
  ├── CacheStore trait + MemoryCache 实现
  ├── 文章列表/详情缓存集成
  └── 后台任务队列（SQLite 持久化方案）

Week 3:
  ├── 媒体处理管线（image crate：缩略图 + WebP）
  ├── 审计日志表 + 中间件
  └── 集成测试覆盖新功能
```

### 阶段 2 — 核心功能（2-3 周）

```
Week 4:
  ├── SSE 实时推送（新评论、通知）
  ├── 通知系统框架（Email + InApp）
  └── 邮件发送（lettre crate）

Week 5:
  ├── 内容版本/修订历史
  ├── 草稿自动保存 API
  └── API 版本化策略落地

Week 6:
  ├── Admin Dashboard API（统计、管理）
  ├── RedisStore 实现（限流器 + 缓存）
  └── 集成测试 + 性能测试
```

### 阶段 3 — 平台化（按需）

```
  ├── Repository 抽象层
  ├── 配置热更新
  ├── Prometheus metrics 集成
  ├── Docker + docker-compose + CI/CD
  ├── Meilisearch 替换 FTS5
  ├── S3 对象存储支持
  └── 水平扩展验证（多实例部署测试）
```

---

## 5. 依赖新增预估

### 阶段 1

| 依赖 | 用途 | 体积影响 |
|------|------|---------|
| `tokio` (broadcast) | EventBus | 已有，零增长 |
| `image` | 图片处理 | +~3MB 编译 |
| 无新增 | FTS5 是 SQLite 内建 | 零 |

### 阶段 2

| 依赖 | 用途 | 体积影响 |
|------|------|---------|
| `lettre` | 邮件发送 | +~2MB |
| `redis` | 缓存/限流/队列 | +~1MB |

### 阶段 3

| 依赖 | 用途 | 体积影响 |
|------|------|---------|
| `prometheus` | 指标采集 | +~1MB |
| `rust-s3` | 对象存储 | +~2MB |
| `meilisearch-sdk` | 搜索引擎客户端 | +~0.5MB |

---

## 6. 风险与注意事项

### 6.1 性能

- WASM 插件每次 Hook 调用有 ~10-100μs 开销，高频场景需评估
- Redis 引入网络延迟，缓存策略需权衡 TTL 和一致性
- EventBus broadcast 需控制容量，避免内存溢出

### 6.2 兼容性

- FTS5 需要 SQLite 编译时启用（大部分发行版已默认启用）
- `image` crate 的 WebP 编码需要 `webp` feature flag
- Meilisearch 需要独立部署，增加运维复杂度

### 6.3 迁移策略

- 所有新功能通过 feature flag 控制，不影响现有功能
- 缓存默认关闭（`MemoryCache`），生产环境启用 Redis
- 搜索默认 FTS5，通过配置切换 Meilisearch
- 每个阶段完成后确保 `cargo test` + `cargo clippy` 全部通过
