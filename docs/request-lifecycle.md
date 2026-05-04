# 请求生命周期

本文档描述一个 HTTP 请求从 TCP 接受到响应返回的完整生命周期，包括中间件链、路由、Handler、Service、数据库访问等各环节。

## 1. 运行时模型

| 项目 | 值 |
|------|-----|
| Runtime | tokio multi-thread（`#[tokio::main]`，默认 CPU 核数 worker threads） |
| HTTP 引擎 | hyper（通过 `axum::serve`） |
| 连接池 | sqlx，默认 `max_connections=5`（`DB_POOL_SIZE` 环境变量可调） |
| EventBus | `tokio::sync::broadcast`，容量 256 |
| Worker 并发度 | 默认 2（`WORKER_CONCURRENCY` 环境变量可调） |

## 2. 完整流程图

以 `GET /api/v1/posts?q=rust&page=1` 为例：

```
┌─────────────────────────────────────────────────────────────────┐
│                    TCP ACCEPT（非阻塞，epoll/kqueue）             │
│  tokio multi-threaded runtime 接受连接                           │
│  hyper 为每个连接 spawn 一个独立 async task                      │
└──────────────────────┬──────────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────────┐
│                    HTTP/1.1 请求解析（hyper）                     │
└──────────────────────┬──────────────────────────────────────────┘
                       │
          ┌────────────┴────────────┐
          │  Axum Layer 链（外→内）  │
          │  最后注册的最先执行       │
          └────────────┬────────────┘
                       │
    ┌──────────────────┼──────────────────────────────────┐
    │                  ▼                                  │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L1] aop_http_layer                      │      │
    │  │ AspectEngine::dispatch_http_before()     │      │
    │  │ 可能短路返回 JSON 响应                     │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L2] security_headers                    │      │
    │  │ 注入 X-Content-Type-Options, X-Frame-    │      │
    │  │ Options, HSTS, Referrer-Policy 等         │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L3] CorsLayer                           │      │
    │  │ OPTIONS 预检 / Access-Control-* 头        │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L4] TraceLayer（tower-http）             │      │
    │  │ 创建 tracing span "request"              │      │
    │  │ 日志：--> request start                   │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L5] inject_request_id                   │      │
    │  │ 生成 UUID v7 → X-Request-ID 响应头        │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L6] track_metrics                       │      │
    │  │ http_requests_total{active}++             │      │
    │  │ 启动 wall-clock 计时器                    │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     ▼                              │
    │  ┌──────────────────────────────────────────┐      │
    │  │ [L7] locale_middleware                    │      │
    │  │ ?lang= / Accept-Language → task_local     │      │
    │  └──────────────────┬───────────────────────┘      │
    │                     │                              │
    │     ┌───────────────┴───────────────┐              │
    │     │  进入 /api/v1 嵌套路由         │              │
    │     └───────────────┬───────────────┘              │
    │                     │                              │
    │  ┌──────────────────┼──────────────────┐           │
    │  │                  ▼                  │           │
    │  │  ┌──────────────────────────────┐   │           │
    │  │  │ [IL1] RequestBodyLimitLayer  │   │           │
    │  │  │ 2MB body 大小限制            │   │           │
    │  │  └──────────────┬───────────────┘   │           │
    │  │                 ▼                   │           │
    │  │  ┌──────────────────────────────┐   │           │
    │  │  │ [IL2] Extension(limiters)    │   │           │
    │  │  │ 注入 RateLimiterSet          │   │           │
    │  │  └──────────────┬───────────────┘   │           │
    │  │                 ▼                   │           │
    │  │  ┌──────────────────────────────┐   │           │
    │  │  │ [IL3] global_rate_limit      │   │           │
    │  │  │ IP 滑动窗口 60req/60s        │   │           │
    │  │  │ API Token 额外限流           │   │           │
    │  │  └──────────────┬───────────────┘   │           │
    │  │                 │                   │           │
    │  │                 ▼                   │           │
    │  │  ┌──────────────────────────────┐   │           │
    │  │  │      路由匹配（Router）        │   │           │
    │  │  │  GET /api/v1/posts           │   │           │
    │  │  │  → handlers::post::list      │   │           │
    │  │  └──────────────┬───────────────┘   │           │
    │  │                 │                   │           │
    └──│─────────────────│───────────────────│───────────┘
       │                 ▼                   │
       │  ┌──────────────────────────────┐   │
       │  │   Handler 参数提取（顺序）    │   │
       │  │                              │   │
       │  │  1. AuthUser                 │   │
       │  │     ├ JWT Bearer 验签        │   │
       │  │     ├ API Token 查 DB        │   │
       │  │     ├ 租户解析               │   │
       │  │     └ 永不 reject            │   │
       │  │                              │   │
       │  │  2. State(state)             │   │
       │  │     └ AppState clone（Arc）  │   │
       │  │                              │   │
       │  │  3. Query(query)             │   │
       │  │     └ ?q=rust&page=1         │   │
       │  └──────────────┬───────────────┘   │
       │                 ▼                   │
       │  ┌──────────────────────────────┐   │
       │  │   Handler Body              │   │
       │  │                              │   │
       │  │  PaginationParams::          │   │
       │  │    from_options(1, None)     │   │
       │  │         │                    │   │
       │  │         ▼                    │   │
       │  │  post_service::list_posts()  │   │
       │  └──────────────┬───────────────┘   │
       │                 │                   │
       │     ┌───────────┴───────────┐       │
       │     │   Service Layer       │       │
       │     │                       │       │
       │     │  ┌─────────────────┐  │       │
       │     │  │ Tantivy 全文搜索 │  │       │
       │     │  │ → post_ids[]    │  │       │
       │     │  └────────┬────────┘  │       │
       │     │           ▼           │       │
       │     │  ┌─────────────────┐  │       │
       │     │  │ sqlx 连接池     │  │       │
       │     │  │ acquire() await │  │       │
       │     │  └────────┬────────┘  │       │
       │     │           ▼           │       │
       │     │  ┌─────────────────┐  │       │
       │     │  │ SQL JOIN 查询   │  │       │
       │     │  │ posts + users   │  │       │
       │     │  │ + categories    │  │       │
       │     │  │ WHERE id IN(?)  │  │       │
       │     │  └────────┬────────┘  │       │
       │     │           ▼           │       │
       │     │  ┌─────────────────┐  │       │
       │     │  │ SQL 批量查 tags │  │       │
       │     │  │ 避免 N+1       │  │       │
       │     │  └────────┬────────┘  │       │
       │     │           ▼           │       │
       │     │  组装 PostResponse[]  │       │
       │     │  返回 (items, total)  │       │
       │     └───────────┬───────────┘       │
       │                 │                   │
       │                 ▼                   │
       │  ┌──────────────────────────────┐   │
       │  │ Ok(ApiResponse::success(     │   │
       │  │   PaginatedData { ... }      │   │
       │  │ ))                           │   │
       │  └──────────────┬───────────────┘   │
       │                 │                   │
       │                 ▼                   │
       │  ┌──────────────────────────────┐   │
       │  │ IntoResponse → JSON 序列化    │   │
       │  │ Content-Type: application/json│  │
       │  └──────────────┬───────────────┘   │
       │                 │                   │
       └─────────────────│───────────────────┘
                         │
          ┌──────────────┴──────────────┐
          │  响应原路返回（内→外）        │
          └──────────────┬──────────────┘
                         │
                         ▼
    ┌──────────────────────────────────────────────────────┐
    │ [L7] locale: task_local scope 结束                    │
    │ [L6] metrics: 记录 histogram + status counter          │
    │ [L5] request_id: X-Request-ID header 已注入           │
    │ [L4] TraceLayer: log "<-- request done" latency_ms     │
    │ [L3] CORS: 注入 Access-Control-* 头                   │
    │ [L2] security_headers: 注入安全头                      │
    │ [L1] aop_http: dispatch_http_after()                   │
    └──────────────────────┬────────────────────────────────┘
                         │
                         ▼
    ┌──────────────────────────────────────────────────────┐
    │ hyper 写入 HTTP 响应到 TCP socket                      │
    │ keep-alive 或 close（取决于 HTTP 版本）                │
    └──────────────────────────────────────────────────────┘
```

## 3. 写请求额外流程

以 `POST /api/v1/posts`（创建文章）为例，Service 层有额外步骤：

```
Handler Body
  │
  ├── 参数校验（validation::validate）         ← 内存，微秒级
  │
  ├── Plugin dispatch_filter(PostCreating)     ← 插件可修改请求数据
  │
  ├── 业务逻辑
  │   ├── slug 生成 + 唯一性检查               ← 1 次 DB 查询
  │   ├── excerpt 自动提取                     ← Markdown → plain text
  │   └── 全文索引更新（Tantivy）               ← 写入内存索引
  │
  ├── Repository INSERT                        ← 1 次 DB 写入
  │
  ├── EventBus::emit(PostCreated)              ← 同步 broadcast，微秒级
  │   │
  │   ├── → Event Subscriber（插件系统）        ← 异步 tokio task
  │   ├── → Audit Subscriber（审计日志）        ← 异步 tokio task
  │   └── → Webhook Subscriber（HTTP 投递）     ← 异步 tokio task
  │       └── WorkerEnqueuer → jobs 表          ← 异步
  │
  └── 返回 ApiResponse
```

## 4. 关键节点一览

| # | 节点 | 所在层 | 耗时量级 | 阻塞类型 | 说明 |
|---|------|--------|---------|---------|------|
| 1 | TCP accept | Runtime | μs | 非阻塞 | epoll/kqueue，无限流 |
| 2 | HTTP 解析 | hyper | μs | 非阻塞 | HTTP/1.1 解帧 |
| 3 | aop_http before | Middleware | μs~ms | 非阻塞 | Aspect 遍历，通常 0~5 个 |
| 4 | CORS 检查 | Middleware | μs | 非阻塞 | preflight 直接返回 |
| 5 | TraceLayer span | Middleware | μs | 非阻塞 | tracing span 创建 |
| 6 | request_id 生成 | Middleware | μs | 非阻塞 | UUID v7 |
| 7 | metrics 计数 | Middleware | μs | 非阻塞 | Prometheus atomic |
| 8 | locale 检测 | Middleware | μs | 非阻塞 | Header 解析 |
| 9 | RequestBodyLimit | Middleware | μs | 非阻塞 | 仅检查 Content-Length |
| 10 | **global_rate_limit** | Middleware | **μs~ms** | 非阻塞 | DashMap 滑动窗口，但 IP 解析需读 Header |
| 11 | **AuthUser 提取** | Extractor | **μs~ms** | 非阻塞/可能 DB | JWT 验签纯 CPU；API Token 需 1 次 DB 查询 |
| 12 | State clone | Extractor | ns | 非阻塞 | Arc 浅拷贝 |
| 13 | Query 解析 | Extractor | μs | 非阻塞 | URL decode + serde |
| 14 | validation 校验 | Handler | μs | 非阻塞 | validator crate |
| 15 | **Tantivy 全文搜索** | Service | **ms~10ms** | 非阻塞 | 内存索引，取决于索引大小 |
| 16 | **sqlx 连接池 acquire** | Repository | **μs~5s** | 异步等待 | 池满时排队，acquire_timeout=5s |
| 17 | **SQL 查询执行** | Repository | **ms~100ms** | spawn_blocking | SQLite 是 C 库，sqlx 用 blocking thread |
| 18 | SQL 批量查 tags | Repository | ms | spawn_blocking | 避免 N+1，1 次批量查询 |
| 19 | JSON 序列化 | IntoResponse | μs~ms | 非阻塞 | serde_json，取决于数据量 |
| 20 | EventBus emit | Service | μs | 非阻塞 | broadcast send |
| 21 | **媒体 URL 生成** | Handler | **ms*N** | 非阻塞 | S3 presigned URL 列表中逐个生成 |
| 22 | **CachedPostRepository** | Repository | μs~ms | 非阻塞 | DashMap 缓存，命中时跳过 DB |

## 5. 潜在性能瓶颈

| # | 节点 | 风险等级 | 问题描述 | 优化建议 |
|---|------|---------|---------|---------|
| 1 | **SQLite 连接池** | 🔴 高 | 默认仅 5 连接，高并发下大量请求排队等 acquire，超过 5s 返回 500 | 调大 `DB_POOL_SIZE`；或切换 PostgreSQL |
| 2 | **SQLite 写锁** | 🔴 高 | WAL 模式下读写并发，但写-写仍是串行的，`busy_timeout=5000` 可能不够 | 高写场景考虑 PostgreSQL；或批量写入 |
| 3 | **媒体 URL 列表** | 🟡 中 | 列表 API 中每个 media 项逐个调用 `storage.url()` 生成 presigned URL，100 项 = 100 次 HMAC 计算 | 批量 URL 生成；或用固定 URL 模式 |
| 4 | **内存分页** | 🟡 中 | tenant/rbac/plugin 的 list() 加载全量数据到内存再切片，数据量大时浪费 | 改为 SQL LIMIT/OFFSET |
| 5 | **API Token 认证** | 🟡 中 | 每次请求查 DB 验证 token，高频场景增加 DB 压力 | 内存缓存 token→role 映射，TTL 过期 |
| 6 | **AOP dispatch** | 🟢 低 | 每个请求遍历 aspect 列表，通常 ≤5 个，O(n) | 无需优化 |
| 7 | **EventBus 慢消费者** | 🟢 低 | broadcast 容量 256，慢消费者会被 Lagged 丢事件 | 已有 warn 日志；必要时增大队列 |
| 8 | **Tantivy 搜索** | 🟢 低 | 内存索引搜索速度快，但索引未持久化，重启需重建 | 定期 save-to-disk |
| 9 | **无连接数限制** | 🟡 中 | `axum::serve` 未配置最大并发连接数，极端情况可能耗尽资源 | 加 `tower::limit::concurrency` |
| 10 | **`into_make_service()`** | 🟡 中 | 未使用 `into_make_service_with_connect_info()`，无法直接获取客户端 IP | 改用 `with_connect_info::<SocketAddr>()` |

## 6. 后台任务

以下任务与 HTTP 服务器共享同一个 tokio runtime：

| 任务 | 启动位置 | 并发模型 | 关停信号 |
|------|---------|---------|---------|
| 日志清理 | `main.rs` | 单 task，每 3600s | 无 |
| Rate limiter 清理 | `server.rs` | 单 task，每 300s | `watch::Receiver` |
| Event subscriber（插件） | `spawn_event_subscriber` | 单 task，broadcast recv | `watch::Receiver` |
| Audit subscriber | `spawn_audit_subscriber` | 单 task，broadcast recv | `watch::Receiver` |
| Webhook subscriber | `spawn_webhook_subscriber` | 单 recv，per-delivery spawn | `watch::Receiver` |
| CronScheduler | `server.rs` | 单 task，轮询间隔可配 | 无 |
| WorkerRunner | `server.rs` | `worker_concurrency` 并发 | 无 |
| Plugin 文件监控 | `plugins.rs` | `notify::RecommendedWatcher` + debounced | graceful degradation |

## 7. 状态共享模型

```
AppState (Clone = Arc 浅拷贝)
├── pool: SqlitePool              ← Arc 内部，跨 task 共享
├── config: Arc<AppConfig>        ← 不可变
├── jwt_decoding_key              ← 不可变
├── plugins: Arc<PluginManager>   ← 内部有 RwLock<Watcher> + DashMap<instances>
├── eventbus: EventBus            ← broadcast::Sender（Clone）
├── post_repo: Arc<dyn PostRepo>  ← CachedPostRepository → DashMap 缓存
├── *_repo: Arc<dyn Repo>         ← 各 repo 内部持有 pool.clone()
├── search: Arc<dyn SearchEngine> ← Tantivy 索引有 RwLock
├── content_type_registry: Arc    ← DashMap + RwLock
├── aspect_engine: Arc            ← DashMap + RwLock
├── rbac: Arc<RbacService>        ← pool + 内存缓存
├── storage: Arc<dyn Storage>     ← S3 client 或 LocalFS
├── cms_cache: Arc<DashMap>       ← 无锁缓存
└── ...
```

所有共享状态通过 `Arc` 或内部 `DashMap`/`RwLock` 保证线程安全，无 `Mutex` 热点。

## 8. Content Type 动态 CRUD 生命周期

Content Type 路由基于 TOML 定义的 Schema 动态生成 CRUD API，无需手写 Handler。

### 8.1 路由注册

启动时 `register_content_routes()` 为每个 ContentTypeSchema 注册固定 axum 路由：

```
/cms/{plural}              GET  → list
/cms/{plural}              POST → create
/cms/{plural}/{id_or_slug} GET  → get
/cms/{plural}/{id}         PUT  → update
/cms/{plural}/{id}         DELETE → delete
/admin/cms/{plural}        GET  → admin list
/admin/cms/{plural}/{id}   GET  → admin detail
```

热加载的类型通过 catch-all 路由 `{*path}` 匹配，走 `dynamic_cms_handler`。

### 8.2 列表请求流程

以 `GET /api/v1/cms/articles?page=1&include=author` 为例：

```
Middleware 链（同内置 API，见第 2 节）
  │
  ▼
路由匹配 → GET /cms/articles
  │
  ▼
Handler（闭包）
  │
  ├── 1. Schema 查找
  │     registry.get("articles")         ← ArcSwap 无锁读，O(1)
  │     → ContentTypeSchema (Arc)
  │
  ├── 2. API 访问控制
  │     check_api_access(ct.api.list.access, &auth)
  │     → Public / Member / Admin / None
  │
  ├── 3. 查询构建
  │     ├── build_rule_sql()             ← 编译 API Rule 为 SQL WHERE
  │     │   （Rule 已在注册时预解析为 CachedRules）
  │     │   Auth 变量 @request.auth.id 实时替换
  │     ├── 构造 ContentQuery
  │     │   page/page_size/sort/filters/status/search
  │     └── cms_cache 查找（DashMap TTL 缓存）
  │         命中 → 直接返回
  │
  ├── 4. AOP before_read
  │     aspect_engine.dispatch_data_before_read()
  │
  ├── 5. SQL 执行（ContentRepository::find）
  │     ├── 列名计算（首次 PRAGMA table_info，后续缓存）
  │     ├── WHERE 构建
  │     │   ├── deleted_at IS NULL           ← soft_deletable
  │     │   ├── tenant_id = ?                ← 多租户隔离
  │     │   ├── status = 'published'         ← draft_publish 类型
  │     │   ├── 字段筛选（?title=xxx）
  │     │   └── API Rule WHERE
  │     ├── ORDER BY（sort 参数或 default_sort）
  │     ├── LIMIT/OFFSET
  │     ├── [ASYNC] COUNT(*) 查询            ← 第 1 次 DB 查询
  │     ├── [ASYNC] SELECT 查询              ← 第 2 次 DB 查询
  │     └── row_to_value() 逐行转换
  │
  ├── 6. 关联展开（resolve_relations）
  │     ├── ManyToOne: 批量 SELECT ... WHERE id IN (?)
  │     ├── OneToMany: 批量 SELECT ... WHERE fk IN (?)
  │     └── ManyToMany: 批量 JOIN through 表
  │     每个 relation 类型 = 1 次额外 DB 查询（已避免 N+1）
  │
  ├── 7. strip_meta()  — 移除 __meta 字段
  │
  ├── 8. AOP after_read
  │
  └── 9. 写入 cms_cache → 返回 PaginatedData
```

### 8.3 创建请求流程

以 `POST /api/v1/cms/articles` 为例：

```
Handler
  │
  ├── 1. Schema 查找 + API 访问控制
  │
  ├── 2. Plugin filter: ContentCreating     ← 插件可修改 body
  │
  ├── 3. AOP before_create
  │
  ├── 4. ContentRepository::create
  │     ├── [ASYNC] BEGIN TRANSACTION
  │     ├── validate_create_tx()
  │     │   ├── 必填检查、类型检查、enum 校验
  │     │   ├── pattern（正则）校验
  │     │   └── [ASYNC] check_unique_fields()   ← 逐字段 SELECT COUNT
  │     ├── UUID v7 生成
  │     ├── inject_auto_fill()                 ← user_id/role/tenant_id
  │     ├── 动态 INSERT INTO table (cols) VALUES (...)
  │     ├── [ASYNC] COMMIT
  │     └── [ASYNC] find_by_id() 回读
  │
  ├── 5. cms_cache 失效（cms:articles:*）
  │
  ├── 6. AOP after_create
  │
  └── 7. Plugin action: ContentCreated        ← 异步 fire-and-forget
```

### 8.4 关键节点

| # | 节点 | 耗时 | 阻塞 | 说明 |
|---|------|------|------|------|
| C1 | Schema 查找 | μs | 非阻塞 | ArcSwap 无锁读 |
| C2 | API Rule 编译 | μs | 非阻塞 | 预解析 + 实时变量替换 |
| C3 | **cms_cache 查找** | μs | 非阻塞 | DashMap TTL 缓存，命中跳过全部 DB |
| C4 | **COUNT + SELECT** | ms~100ms | spawn_blocking | 2 次 DB 查询 |
| C5 | **关联展开** | ms | spawn_blocking | 每个 relation 类型 1 次批量查询 |
| C6 | **unique 校验** | ms | spawn_blocking | 每个唯一字段 1 次 COUNT 查询 |
| C7 | 动态 INSERT | ms | spawn_blocking | 含事务 begin/commit |
| C8 | strip_meta | μs | 非阻塞 | 移除 __meta 字段 |

### 8.5 Content Type 性能瓶颈

| # | 瓶颈 | 风险 | 优化建议 |
|---|------|------|---------|
| C1 | **COUNT + SELECT 双查询** | 🟡 中 | 用 `COUNT(*) OVER()` 窗口函数合并为单查询 |
| C2 | **API Rule 每请求编译** | 🟡 中 | 编译结果缓存（键 = rule hash + auth id） |
| C3 | **cms_cache 无大小上限** | 🟡 中 | 加 LRU 淘汰或 max capacity |
| C4 | **unique 校验串行** | 🟢 低 | 合并为单条 `SELECT ... OR ...` 查询 |

## 9. 插件动态路由生命周期

插件路由通过 `.fallback(handle_plugin_route)` 注册，是最后一个路由匹配点。

### 9.1 路由匹配机制

```
所有 axum 路由不匹配
  │
  ▼
.fallback(handle_plugin_route)       ← 兜底处理器
  │
  ├── 提取 path + method + headers
  ├── [ASYNC] 读取 body（上限 1MB）
  │
  ▼
PluginManager::dispatch_route()
  │
  ├── [ASYNC] 获取 plugins RwLock 读锁
  │
  ├── 遍历所有已加载插件               ← O(N*M) 线性扫描
  │   │
  │   ├── 检查插件是否 enabled
  │   ├── 遍历 manifest.routes[]
  │   │   └── path_matches_route()
  │   │       逐段比较，:param 作为通配符
  │   │       例：/products/:id 匹配 /products/42
  │   │
  │   └── 找到匹配 → check_api_access()
  │
  ▼
call_plugin_json() — 按引擎类型分发
```

### 9.2 插件执行流程

以 `GET /api/v1/plugins/ecommerce/products` 匹配到 JS 插件为例：

```
call_plugin_json()
  │
  ▼
┌─────────────────────────────────────────────────────┐
│  JS 引擎（rquickjs）                                  │
│                                                       │
│  ├── 从实例池 round-robin 获取 JsInstance             │
│  │   （池大小 = 创建时固定）                           │
│  │                                                    │
│  ├── ctx.with(|ctx| { ... })                         │
│  │   ├── 查找 Plugin[handler] 函数                   │
│  │   ├── 传入 input JSON                              │
│  │   │   {path, method, body, headers, params}       │
│  │   ├── [SYNC] 执行 JS 函数                         │
│  │   │   插件可调用宿主函数：                          │
│  │   │   ├── vfsRead/vfsWrite/vfsDelete              │
│  │   │   ├── dbQuery/dbExecute                       │
│  │   │   ├── httpGet/httpPost                        │
│  │   │   ├── getConfig/getPost                       │
│  │   │   └── log/setData                             │
│  │   │   （宿主函数通过 HostContext 执行）             │
│  │   │                                                │
│  │   ├── 中断检查（wall-clock timeout）               │
│  │   └── 返回 JSON 结果                               │
│  │                                                    │
│  └── 清理中断 handler                                 │
└─────────────────────────────────────────────────────┘
```

**WASM 引擎**有所不同：通过 `block_in_place` 在 tokio blocking thread 上同步执行，有 fuel 限制（默认 10M）+ wall-clock timeout。

**Lua 引擎**：指令计数 hook（每 1000 指令检查一次），超出配额终止。

### 9.3 宿主函数调用

插件执行期间可通过宿主函数访问系统资源：

```
插件代码调用 vfsRead("data.json")
  │
  ▼
js_host::vfs_read() / lua_host::vfs_read()
  │
  ├── VFS 沙箱检查
  │   ├── 路径转义防护（拒绝 ..）
  │   ├── 权限检查（read/write/read-write）
  │   └── 大小限制（单文件 + 总配额）
  │
  └── [SYNC] std::fs::read()  ← 在 block_in_place 中执行
```

```
插件代码调用 dbQuery("SELECT * FROM tags")
  │
  ▼
host_common::db_query()
  │
  ├── 表权限检查（permissions.database 白名单）
  ├── 受保护表检查（users/roles 等禁止访问）
  ├── SQL 方言转换
  └── [SYNC] tokio::task::block_in_place(|| {
         handle.block_on(sqlx::query(...).fetch_all(&pool))
       })
```

### 9.4 关键节点

| # | 节点 | 耗时 | 阻塞 | 说明 |
|---|------|------|------|------|
| P1 | **body 全量读取** | μs~ms | 异步 | 即使 GET 请求也读取 body，上限 1MB |
| P2 | **插件线性扫描** | μs~ms | 非阻塞 | O(N*M)，N=插件数，M=路由数/插件 |
| P3 | RwLock 读锁获取 | μs | 非阻塞 | 阻塞插件热加载 |
| P4 | **JS 执行** | ms~s | ctx.with | 受 timeout 保护 |
| P5 | **WASM 执行** | ms~s | block_in_place | fuel + timeout 双重保护 |
| P6 | **Lua 执行** | ms~s | 同步 | 指令计数 hook |
| P7 | **VFS 文件操作** | μs~ms | block_in_place | std::fs 同步 IO |
| P8 | **DB 查询（宿主）** | ms | block_in_place | sqlx 连接池 + blocking |
| P9 | HTTP 调用（宿主） | ms~s | 异步 | reqwest async |

### 9.5 插件路由性能瓶颈

| # | 瓶颈 | 风险 | 优化建议 |
|---|------|------|---------|
| P1 | **404 也走插件扫描** | 🔴 高 | 所有未匹配路径都读取 body + 遍历插件，大量 404 时浪费严重 |
| P2 | **O(N*M) 路由匹配** | 🟡 中 | 构建路由前缀树（trie）替代线性扫描 |
| P3 | **GET 请求读 body** | 🟡 中 | GET/HEAD/DELETE 跳过 body 读取 |
| P4 | **无响应缓存** | 🟡 中 | 对幂等插件路由加缓存层 |
| P5 | **WASM 实例池固定** | 🟢 低 | 运行时动态扩展实例池 |
| P6 | **VFS 同步 IO** | 🟢 低 | 在 block_in_place 中已安全 |

## 10. 三种路由对比

| 维度 | 内置 API | Content Type | 插件路由 |
|------|---------|-------------|---------|
| 路由注册 | 编译时 axum route | 启动时动态 + catch-all | 运行时 fallback |
| 路由匹配 | O(1) trie | O(1) trie | O(N*M) 线性 |
| SQL 生成 | 静态 SQL（sqlx 宏检查） | 动态拼接（运行时） | 委托插件 |
| 缓存 | CachedPostRepository | cms_cache (DashMap TTL) | 无 |
| 校验 | validator crate | Schema 驱动（类型/必填/唯一/枚举/正则） | 委托插件 |
| 关联展开 | 手写 JOIN | 自动 resolve_relations | 委托插件 |
| 热加载 | 不支持 | 支持（ArcSwap registry） | 支持（文件监控 + RwLock） |
| 核心瓶颈 | 连接池大小 | COUNT+SELECT 双查询 | 404 也走插件扫描 |
