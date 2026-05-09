# Spring Boot 设计借鉴路线图

> 从 Spring Boot 生态中提取适合 Rust/Axum 场景的设计模式，作为项目演进参考。
> 已有对应：`AppError` ≈ `@ControllerAdvice`、`EventBus` ≈ `ApplicationEventPublisher`、`Worker` ≈ `@Async` + `@Scheduled`。
> 与 `laravel-inspiration.md` 互补——本文聚焦 Spring Boot 独有的设计理念。

---

## 现有架构 ↔ Spring Boot 对照

| Spring Boot | 项目现状 | 差距 |
|---|---|---|
| IoC Container / `@Autowired` | `build_app_state()` 手动 `Arc::new()` | 60+ 组件手写构造 |
| `@ControllerAdvice` | `AppError` + `IntoResponse` | ✅ 已完善 |
| `@Validated` / Bean Validation | `validation::validate(&req)?` | ✅ 已完善 |
| `ApplicationEventPublisher` | `EventBus` (tokio broadcast) | ✅ 已完善 |
| `@Scheduled` | `CronScheduler` + DB cron 表 | ✅ 更灵活 |
| `@Transactional` | 手动 `pool.begin()` | ❌ 仅 4 处使用，多数多步操作无事务 |
| `@Profile` / 多环境配置 | 平铺 `.env` | ❌ 无环境分层 |
| Actuator 健康监控 | `/api/v1/health` 基础版 | ❌ 缺少详细探针 |
| Spring Security FilterChain | Axum `layer()` + `AuthUser` extractor | 部分覆盖 |
| AOP `@Around` | `AspectEngine` + `aop_http.rs` | 有基础，不够通用 |
| `@ConditionalOnProperty` | `#[cfg(feature = "...")]` | ✅ 编译期方案 |

---

## P0 — 高价值（解决实际痛点）

### 1. Service Locator / 模块化 App State

**现状**：`build_app_state()` 是一个 130 行的巨型函数（`src/lib.rs:117-231`），60+ 个 `Arc::new()` 手动排列。新增一个 service 要改 3 处（struct 定义 + 构造 + 传参）。

**Spring 借鉴**：Spring 的 `ApplicationContext` 容器自动管理 Bean 生命周期和依赖。Rust 不需要反射式 IoC，但可以借鉴 **Service Locator 模式** 简化组件注册。

**方案**：Builder + trait-object registry，编译期检查依赖类型：

```rust
// src/app/mod.rs

pub struct AppBuilder {
    pool: Pool,
    config: Arc<AppConfig>,
    services: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl AppBuilder {
    pub fn new(config: AppConfig) -> Self { ... }

    pub fn register<T: Send + Sync + 'static>(mut self, service: Arc<T>) -> Self {
        self.services.insert(TypeId::of::<T>(), service);
        self
    }

    pub fn resolve<T: 'static>(&self) -> Arc<T> {
        self.services
            .get(&TypeId::of::<T>())
            .and_then(|s| s.downcast_ref::<Arc<T>>())
            .cloned()
            .expect("service not registered")
    }

    pub fn build(self) -> AppState {
        AppState {
            inner: Arc::new(AppStateInner {
                pool: self.pool,
                config: self.config,
                services: self.services,
            }),
        }
    }
}

// 注册时链式调用
let state = AppBuilder::new(config)
    .register::<dyn PostRepository>(post_repo)
    .register::<dyn UserRepository>(user_repo)
    .register::<PostService>(post_service)
    .register::<EventBus>(bus)
    .build();
```

设计要点：
- **不引入反射**：用 `TypeId` + `Any` 实现类型安全的 service locator
- **可选编译期检查**：`resolve()` 可以用 macro 包装为 `resolve!<T>(state)` 在 debug 模式 panic，release 模式 unwrap
- **渐进迁移**：`AppState` 保留现有字段，新组件走 registry；逐步替换老字段
- **测试友好**：测试时只需 `register` mock 实现即可

与 Spring IoC 的区别：
| | Spring IoC | Rust Service Locator |
|---|---|---|
| 发现机制 | 反射 + `@ComponentScan` | 显式 `register::<T>()` |
| 依赖注入 | 自动 `@Autowired` | 手动 `resolve::<T>()` |
| 生命周期 | singleton/prototype/request | `Arc` 引用计数 |
| 循环依赖 | 自动检测 | 编译期 Arc 可循环，需人工避免 |

**收益**：新增 service 只需 1 行 register + 1 行 resolve，`build_app_state()` 从过程式变为声明式。

**工作量**：2-3 天

---

### 2. `#[transactional]` 过程宏

**现状**：只有 4 处使用 `pool.begin()` 手动事务（`sqlx_post.rs`、`auth.rs`、`sqlite_queue.rs`、`scheduler.rs`）。架构审计发现 media 删除、options 批量更新、content_type 字段写入等多步操作缺少事务保护。

**Spring 借鉴**：Spring 的 `@Transactional` 注解是声明式事务的典范——一个注解自动包裹 `begin` / `commit` / `rollback`。

**方案**：过程宏自动注入事务边界，service 层函数标注即可：

```rust
// proc-macro crate: raisfast-macros

#[transactional]
async fn create_post(
    pool: &Pool,
    cmd: CreatePostCmd,
    tenant_id: Option<&str>,
) -> AppResult<Post> {
    // 宏展开后自动变为:
    // let mut tx = pool.begin().await?;
    // let result = { ... original body with &mut tx ... }.await;
    // tx.commit().await?;
    // result

    sqlx::query("INSERT INTO posts ...")
        .bind(&cmd.title)
        .execute(pool)  // ← 宏会将 pool 替换为 &mut tx
        .await?;

    for tag_id in &cmd.tag_ids {
        sqlx::query("INSERT INTO post_tags ...")
            .execute(pool)  // ← 同上
            .await?;
    }

    Ok(post)
}
```

实现策略：
- 宏解析函数签名，找到 `pool: &Pool` 参数
- 在函数体前插入 `let mut tx = pool.begin().await?;`
- 将函数体中所有 `pool` 引用替换为 `&tx`
- 函数体后插入 `tx.commit().await?;`
- 错误自动 rollback（tx drop 时）

备选方案（如果过程宏改 pool 引用太侵入）：
```rust
// 更简单：wrapper function + 闭包
#[transactional(pool_arg = "pool")]
async fn create_post(pool: &Pool, cmd: CreatePostCmd) -> AppResult<Post> {
    // 宏生成:
    // pub async fn create_post(pool: &Pool, cmd: CreatePostCmd) -> AppResult<Post> {
    //     crate::db::transaction(pool, move |tx| Box::pin(async move {
    //         create_post_inner(tx, cmd).await
    //     })).await
    // }
    // async fn create_post_inner(tx: &Transaction, cmd: CreatePostCmd) -> AppResult<Post> { ... }
}
```

**收益**：一行注解代替 5 行事务样板代码，降低遗漏事务的风险。

**工作量**：2-3 天

---

### 3. Actuator 风格健康检查

**现状**：`/api/v1/health` 返回固定 `{ "status": "ok" }`，不检查 DB 连接、磁盘空间、Worker 状态等。生产环境无法通过 health endpoint 发现问题。

**Spring 借鉴**：Spring Boot Actuator 的 `/actuator/health` 汇总所有组件状态，返回 `UP` / `DOWN` / `DEGRADED`。

**方案**：

```rust
// src/handlers/health.rs — 扩展现有 health endpoint

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: HealthStatus,
    pub components: HashMap<String, ComponentHealth>,
    pub uptime_seconds: u64,
    pub version: String,
}

#[derive(Serialize)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    pub details: Option<Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HealthStatus {
    Up,
    Down,
    Degraded,
}

// 各组件实现 HealthIndicator trait
pub trait HealthIndicator: Send + Sync {
    fn name(&self) -> &str;
    async fn check(&self) -> ComponentHealth;
}

// 注册多个 indicator
struct DatabaseHealth { pool: Pool }
struct WorkerHealth { queue: Arc<dyn JobQueue> }
struct SearchHealth { engine: Arc<dyn SearchEngine> }
struct StorageHealth { config: Arc<AppConfig> }

// GET /api/v1/health
// → { "status": "UP", "components": { "database": { "status": "UP", ... }, ... } }
// → 任何 DOWN 则整体 status = DOWN，返回 503
```

关键检查项：
- **Database**：`SELECT 1` 测试连接
- **Worker**：dead jobs 数量 / 队列深度
- **Search**：索引是否可用
- **Storage**：磁盘剩余空间 / 可写
- **Migrations**：是否有 pending migration

管理端点（需 admin 权限）：
- `GET /admin/info` — 构建信息、Git SHA、运行时间
- `GET /admin/metrics` — 现有 Prometheus 指标（已有 `middleware/metrics.rs`）

**收益**：生产部署可对接 K8s liveness/readiness probe，故障快速定位。

**工作量**：2-3 天

---

### 4. Profile 风格多环境配置

**现状**：`AppConfig::from_env()` 读取 `.env`，`AppConfig::test_defaults()` 硬编码测试配置。没有开发/测试/预发布/生产环境分层。

**Spring 借鉴**：Spring 的 `application-{profile}.yml` 支持多环境配置覆盖：基础配置 + profile 特定覆盖。

**方案**：分层 `.env` 加载，后者覆盖前者：

```
.env                        # 基础默认（所有环境共享）
.env.local                  # 个人本地覆盖（gitignore）
.env.development            # 开发环境
.env.test                   # 测试环境
.env.production             # 生产环境
```

```rust
// src/config/app.rs — AppConfig::init() 改造

impl AppConfig {
    pub fn init() -> Self {
        let profile = env::var("APP_PROFILE")
            .unwrap_or_else(|_| "development".into());

        // 加载顺序：后者覆盖前者
        dotenvy::from_path(".env").ok();
        dotenvy::from_path(&format!(".env.{profile}")).ok();
        dotenvy::from_path(".env.local").ok();

        let config = Self::from_env();
        Self::validate(&config, &profile);
        config
    }
}
```

设计要点：
- **不引入 YAML/TOML 配置文件**：保持 `.env` 扁平结构（与 12-Factor App 一致）
- **`APP_PROFILE` 环境变量**选择当前环境（`development` / `test` / `staging` / `production`）
- **`.env.local`** 用于开发者个人覆盖（如本地 DB 路径），永远不入 git
- **生产安全**：`profile == "production"` 时强制校验 JWT_SECRET / CORS / HOST 等

与 Spring Profile 的区别：
| | Spring Profile | Rust Profile |
|---|---|---|
| 格式 | YAML 层叠 | `.env` 层叠 |
| 激活 | `spring.profiles.active` | `APP_PROFILE` |
| 覆盖 | 深度合并 | 简单 key 覆盖（dotenv 语义） |
| 类型安全 | `@Value` + `@ConfigurationProperties` | `from_env()` + 类型转换 |

**收益**：开发/测试/生产配置分离，不再手动改 `.env` 切换环境。

**工作量**：1-2 天

---

## P1 — 中价值（提升工程质量）

### 5. `@ConditionalOnProperty` 风格的 Feature Flag 运行时切换

**现状**：`#[cfg(feature = "db-sqlite")]` 是编译期特性开关，改一个 feature 要重新编译。`BuiltinsConfig`（`BUILTIN_BLOG` 等）是运行时开关但只在 `server.rs` 路由注册时检查，不能动态启停。

**Spring 借鉴**：`@ConditionalOnProperty` 允许 Bean 根据配置值有条件注册。

**方案**：在 `AppState` 中引入 Feature Flag 注册表，支持运行时读取和热更新：

```rust
// src/features.rs

pub struct FeatureFlags {
    flags: ArcSwap<HashMap<String, bool>>,
}

impl FeatureFlags {
    pub fn from_config(config: &AppConfig) -> Self { ... }
    pub fn is_enabled(&self, flag: &str) -> bool { ... }
    pub fn reload(&self, config: &AppConfig) { ... }
}

// handler 中使用
pub async fn list_posts(
    State(state): State<AppState>,
) -> AppResult<ApiResponse<...>> {
    if !state.features.is_enabled("blog") {
        return Err(AppError::NotFound("module disabled"));
    }
    // ...
}
```

设计要点：
- 使用 `ArcSwap`（项目已广泛使用）支持无锁热更新
- 与 `BuiltinsConfig` 合并，统一入口
- 可通过 admin API 动态开关（不重启）
- 编译期 feature（`db-sqlite`）保持不变——那是代码级开关，不应运行时切换

**工作量**：1-2 天

---

### 6. Handler Interceptor（统一请求前后处理）

**现状**：AOP HTTP layer（`aop_http.rs`）+ 各种 middleware。但请求计时、请求日志、响应 header 注入散落在多处 middleware 中，且无法按路由选择性地拦截。

**Spring 借鉴**：`HandlerInterceptor.preHandle()` / `postHandle()` / `afterCompletion()` 提供精细的请求生命周期拦截。

**方案**：在现有 `AspectEngine` 基础上扩展为结构化的 Interceptor 注册：

```rust
// src/middleware/interceptor.rs

pub trait HandlerInterceptor: Send + Sync {
    fn pre_handle(&self, req: &mut RequestParts, ctx: &mut InterceptorContext) -> Result<(), AppError> {
        Ok(())
    }
    fn post_handle(&self, req: &RequestParts, resp: &mut Response, ctx: &InterceptorContext) {
        // default no-op
    }
    fn after_completion(&self, req: &RequestParts, result: &Result<(), AppError>, ctx: &InterceptorContext) {
        // default no-op — for cleanup/logging
    }
}

pub struct InterceptorContext {
    pub start_time: Instant,
    pub extras: HashMap<String, Box<dyn Any + Send>>,
}

// 注册
let mut chain = InterceptorChain::new();
chain.add(LoggingInterceptor);           // 所有请求
chain.add(AuthLoggingInterceptor);       // 仅 auth 路由
chain.add(AuditInterceptor);             // 仅写操作
```

与现有 AOP 的关系：
- 现有 `AspectEngine` 侧重于插件系统的 `before_request` / `after_request` hook
- `HandlerInterceptor` 侧重于框架级的横切关注点（日志、审计、计时）
- 两者共存：Interceptor 先执行，AOP hook 后执行

**工作量**：2-3 天

---

### 7. 统一 `@RequestMapping` 风格路由注册

**现状**：`src/server.rs` 1700+ 行，所有路由集中在巨型 `build_router()` 函数中。路由注册、中间件应用、权限检查混在一起。

**Spring 借鉴**：`@RestController` + `@RequestMapping("/api/v1/posts")` 让路由声明和 handler 代码在一起，自动发现注册。

**方案**：用宏或约定将路由注册从 `server.rs` 中拆散到各 handler 模块：

```rust
// src/handlers/post.rs — 路由声明与 handler 在同一文件

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{slug}", get(get_by_slug).put(update).delete(delete))
        .route("/search", get(search))
        .layer(login_rate_limit())  // 路由级中间件
}

// src/server.rs — 只做组装
fn build_router(state: AppState) -> Router {
    let api = Router::new()
        .nest("/posts", post::routes())
        .nest("/users", user::routes())
        .nest("/media", media::routes())
        .nest("/workflow", workflow::routes())
        .nest("/content-type", content_type::routes())
        // ... 每个 handler 模块一行
        ;
    Router::new()
        .nest("/api/v1", api)
        .layer(/* global middleware */)
}
```

设计要点：
- 每个 handler 模块导出 `pub fn routes() -> Router<AppState>`
- `server.rs` 从 1700 行降到 ~100 行路由组装
- 路由级中间件（rate limit、auth guard）在模块内声明
- 全局中间件（CORS、security headers、tracing）仍在 `build_router` 中

**收益**：`server.rs` 可维护性大幅提升，新增模块只需加一行 `.nest()`。

**工作量**：1-2 天（纯重构，不改行为）

---

## P2 — 值得但优先级低

### 8. Spring Security 风格 FilterChain

**现状**：`AuthUser` 是 extractor（不拒绝请求），权限检查在 handler 中手动调用 `auth.ensure_admin()` 等。中间件 `permission.rs` 也是 extractor 模式。

**Spring 借鉴**：Spring Security 的 `SecurityFilterChain` 在请求到达 handler 之前完成所有认证/授权决策，handler 无需关心安全。

**方案**（远期）：

```rust
// src/middleware/security_chain.rs

pub struct SecurityChain {
    rules: Vec<SecurityRule>,
}

struct SecurityRule {
    path_pattern: Matcher,        // "/api/v1/admin/**"
    methods: Vec<Method>,
    required_role: Option<Role>,
    required_permission: Option<String>,
    public: bool,
}

// 在 router 层用 layer 应用
Router::new()
    .nest("/api/v1/admin", admin_routes())
    .layer(SecurityChain::new()
        .rule("/api/v1/admin/**", Require::Role("admin"))
        .rule("/api/v1/posts", Require::Any)       // 公开读
        .rule("/api/v1/posts/**", Require::Role("author"), Method::POST | Method::PUT)
    )
```

> 注意：当前 extractor 模式在 Rust/Axum 生态中更惯用，此方案为远期考虑。当前优先用 Laravel 路线图中的 Policy（P1-5）解决。

---

### 9. 其他可借鉴特性

| 特性 | 说明 | 工作量 |
|------|------|--------|
| **RetryTemplate** | 声明式重试策略（指数退避 + 可配置重试条件），用于插件调用、webhook 投递 | 2-3 天 |
| **RestTemplate 风格 HTTP Client** | 封装 `reqwest` 为类型安全的 HTTP client，插件 HTTP 调用统一走此 client | 1-2 天 |
| **Event Listener 条件过滤** | `@EventListener(condition = "#event.type == 'post'")` 风格，EventBus subscribe 支持条件过滤 | 1 天 |
| **ApplicationRunner** | 启动完成后执行一次性初始化任务（数据迁移检查、默认角色种子等） | 0.5 天 |
| **Graceful Shutdown** | Spring Boot 的 `ShutdownHook`，当前已有 SIGTERM 处理，可增强为等待所有 in-flight 请求完成 | 1 天 |

---

## 实施优先级

```
Service Locator / App State 重构 (P0-1)
    ↓
#[transactional] 过程宏 (P0-2)
    ↓
Actuator 健康检查 (P0-3)
    ↓
Profile 多环境配置 (P0-4)
    ↓
Feature Flag 运行时切换 (P1-5)
    ↓
Handler Interceptor (P1-6)
    ↓
路由注册拆分 (P1-7)
    ↓
其余 P2 项按需排期
```

前两项（Service Locator + Transactional）是架构级改进，中间两项是运维和开发体验提升，后两项是工程规范。

---

## Spring Boot → Rust 适配原则

| Spring Boot 模式 | Rust 适配 | 原因 |
|---|---|---|
| IoC Container | Service Locator（`TypeId` + `Arc<dyn Any>`） | Rust 无反射，显式注册 + 类型安全解析 |
| `@Transactional` | 过程宏注入 `tx` | Rust 没有运行时代理，编译期代码生成是唯一选择 |
| `@Profile` | `.env.{profile}` 层叠 | 保持 12-Factor App 风格，不引入 YAML |
| Actuator | `HealthIndicator` trait + 注册表 | Rust 不需要 JMX，HTTP endpoint 足够 |
| `@ConditionalOnProperty` | `ArcSwap<HashMap>` 运行时 flag | 编译期 feature 和运行时 flag 各有用途 |
| `@RequestMapping` | `fn routes() -> Router` | Axum 原生支持 Router 组合，无需注解发现 |
| Security FilterChain | 暂不实施 | Extractor 模式在 Axum 更惯用，Policy 层解决授权 |
| AOP | 编译期过程宏 / middleware | Rust 没有运行时字节码增强，所有"切面"在编译期展开 |

---

## 与 Laravel 路线图的关系

| 维度 | Laravel 借鉴 | Spring Boot 借鉴 |
|---|---|---|
| 数据层 | Factory、Migration 回滚、Query Scope | `@Transactional`、Service Locator |
| API 层 | API Resource 转换 | Handler Interceptor、路由拆分 |
| 权限 | Policy | Security Chain（远期） |
| 运维 | 维护模式 | Actuator、Profile、Feature Flag |
| 架构 | Lifecycle Hook（EventBus） | App State 重构 |
| 两者共有 | — | Factory ≈ `@Component`，EventBus ≈ `ApplicationEventPublisher` |

**建议实施顺序**：先完成 Laravel 路线图 P0（Migration 回滚、Factory、API Resource），再启动 Spring Boot 路线图 P0（Service Locator、Transactional、Actuator）。
