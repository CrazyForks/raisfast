# Loco & Zola 借鉴分析

> 分析 Loco（Rails 风格 Rust 全栈框架）和 Zola（静态站点生成器）中值得 raisfast 借鉴的设计模式。
> 聚焦于**我们尚未覆盖的、与已有 Laravel/Spring Boot 路线图互补**的特性。

---

## 项目概览

| | Loco | Zola |
|---|---|---|
| 定位 | Rust 全栈 Web 框架（Rails 风格） | 静态站点生成器（单二进制） |
| 技术栈 | Axum + SeaORM + Tera | Tera + pulldown-cmark |
| 核心理念 | Convention over Configuration | 零依赖、极简、内容优先 |
| 与 raisfast 关系 | 同为 Axum 生态，架构可直接参考 | 内容模型和 Markdown 处理可参考 |

---

## P0 — 高价值（直接解决痛点）

### 1. Scaffold 代码生成器

**来源**：Loco `cargo loco generate scaffold`

**现状**：新增一个 content_type 或 API 端点需要手写 handler + service + repository + migration + test，约 200-400 行重复代码。raisfast 已有 content_type 引擎可以自动建表 + CRUD，但**非 content_type 的自定义模型**（如电商 Order、Subscription）仍需全手工。

**方案**：CLI 驱动的代码生成器，一键生成完整 CRUD 模块：

```bash
# 生成新模型的完整 CRUD
raisfast generate model order \
    user:references \
    total:decimal \
    status:string \
    --api

# 输出：
#   added: migration/src/m20260510_order.rs
#   added: src/models/order.rs
#   added: src/repositories/sqlx_order.rs
#   added: src/handlers/order.rs
#   added: src/services/order.rs
#   added: src/dto/order.rs
#   added: tests/api/order.rs
#   injected: src/server.rs (路由注册)
```

生成内容：

| 文件 | 内容 |
|---|---|
| `migration/*.rs` | CREATE TABLE + `down` 回滚 |
| `models/order.rs` | Order struct + `impl_from_row_opt_tenant!` + 基础查询函数 |
| `repositories/sqlx_order.rs` | `define_sqlx_repo!(Order)` + trait 定义 + CRUD 实现 |
| `handlers/order.rs` | `pub fn routes() -> Router<AppState>` + list/get/create/update/delete |
| `services/order.rs` | 业务逻辑骨架 + EventBus emit |
| `dto/order.rs` | CreateOrderRequest / UpdateOrderRequest / OrderResponse |
| `tests/api/order.rs` | CRUD 集成测试骨架 |

设计要点：
- 使用 Tera 模板引擎（Loco 和 Zola 共用）渲染代码文件
- 字段语法：`name:type`，`!` 后缀 = NOT NULL，`^` 后缀 = UNIQUE，`:references` = 外键
- 支持 `--api`（纯 JSON）、`--html`（服务端渲染）模式
- 生成后 `cargo test` 直接可通过

与 Loco 的区别：
| | Loco | raisfast |
|---|---|---|
| ORM | SeaORM（Active Record） | sqlx raw SQL |
| 模板引擎 | Tera | Tera（代码生成） |
| 测试框架 | `boot_test` + insta | `test_app()` + 自定义 helper |

**工作量**：5-7 天（含 Tera 模板 + CLI 集成）

---

### 2. Initializer 模式（启动生命周期钩子）

**来源**：Loco `Initializer` trait + `Hooks`

**现状**：`build_app_state()` 是一个巨大的平铺函数，所有组件构造、EventBus 订阅、Worker 注册都混在一起。新增一个基础设施组件（如 Redis 缓存、邮件队列）需要改 `build_app_state()` + `server.rs` 多处。

**方案**：借鉴 Loco 的 `Initializer` trait，将启动逻辑拆分为独立的、可插拔的初始化单元：

```rust
// src/initializers/mod.rs

pub trait Initializer: Send + Sync {
    fn name(&self) -> &str;

    /// 在 AppState 构造前执行，返回需要注入的组件
    fn before_run(&self, ctx: &mut AppBuilder) -> Result<()> {
        Ok(())
    }

    /// 在路由注册后执行，可以给 Router 添加 layer
    fn after_routes(&self, router: Router<AppState>, ctx: &AppState) -> Result<Router<AppState>> {
        Ok(router)
    }
}
```

内置 Initializer 示例：

```rust
// src/initializers/eventbus_subscribers.rs
pub struct EventBusSubscribers;

impl Initializer for EventBusSubscribers {
    fn name(&self) -> &str { "eventbus_subscribers" }

    fn before_run(&self, ctx: &mut AppBuilder) -> Result<()> {
        let bus = ctx.resolve::<EventBus>();
        // 订阅 audit log
        let audit = AuditSubscriber::new(ctx.resolve::<dyn AuditRepository>());
        tokio::spawn(audit.listen(bus.subscribe()));
        // 订阅 webhook delivery
        // ...
        Ok(())
    }
}
```

注册：

```rust
// src/lib.rs
fn initializers() -> Vec<Box<dyn Initializer>> {
    vec![
        Box::new(initializers::Database::new()),
        Box::new(initializers::EventBusSubscribers),
        Box::new(initializers::WorkerRunner::new()),
        Box::new(initializers::PluginEngine::new()),
        Box::new(initializers::SearchIndex::new()),
    ]
}
```

设计要点：
- 每个 Initializer 是独立文件，职责单一
- `before_run` 接收 `&mut AppBuilder`（可注册组件到 service locator）
- `after_routes` 接收 Router（可添加中间件 layer）
- **插件可以注册自己的 Initializer**（通过 `plugin.toml` 声明）
- Loco 的教训：不需要 Rails 式的排序/覆盖机制——用户直接提供 Vec，顺序即优先级

与 Spring Boot 路线图中 Service Locator（P0-1）的关系：Initializer 是消费端，Service Locator 是基础设施。

**工作量**：2-3 天

---

### 3. Doctor 诊断命令

**来源**：Loco `cargo loco doctor`

**现状**：配置错误（DB 路径错误、JWT 缺失、存储目录不可写）只能在启动时报错，错误信息不友好。用户无法快速排查环境问题。

**方案**：

```bash
$ raisfast doctor

✅ Database connection: sqlite:./storage/db/raisfast.db
✅ Database schema: up to date (42 tables)
✅ Storage root: ./storage (writable, 2.3GB free)
✅ Uploads directory: ./storage/uploads (writable)
✅ Search index: ./storage/search_index (exists)
⚠️  JWT_SECRET: using default value (production unsafe)
✅ CORS origins: http://localhost:3000
⚠️  Redis: not configured (worker queue in-memory only)
✅ Plugins: 3 loaded (wasm, js, lua)
✅ Worker: 2 runners active, 0 dead jobs
```

检查项：

| 检查 | 内容 |
|---|---|
| Database connection | `SELECT 1` 测试 |
| Schema version | 对比 `_migrations` 表与期望版本 |
| Storage writable | 创建测试文件 + 删除 |
| Disk space | 剩余空间 |
| Search index | Tantivy 索引是否可打开 |
| JWT secret | 是否为默认值（生产环境警告） |
| CORS | 是否配置 |
| Plugins | 加载测试 |
| Worker | 队列状态 |

**工作量**：1-2 天

---

## P1 — 中价值（提升工程质量）

### 4. Worker Tag 过滤

**来源**：Loco Worker Tag Filtering

**现状**：Worker 的 `JobHandlerRegistry` 是一个扁平 HashMap，所有 worker 共享同一个队列。无法区分 "邮件 worker" 和 "搜索索引 worker"。

**方案**：

```rust
// 为 Job 类型添加 tags 字段
pub struct Job {
    pub id: i64,
    pub job_type: String,
    pub payload: String,
    pub tags: Vec<String>,  // 新增
    // ...
}

// Worker trait 添加 tags 方法
pub trait JobHandler: Send + Sync {
    fn job_type(&self) -> &str;
    fn tags(&self) -> Vec<String> { vec![] }  // 默认无 tag
    async fn handle(&self, ctx: &WorkerContext, payload: &str) -> Result<()>;
}

// 启动时指定 tag 过滤
// raisfast start --worker=email,notification
// raisfast start --worker=indexing
// raisfast start --server-and-worker  (处理无 tag 的 job)
```

设计要点：
- Job 入队时自动携带 handler 的 tags
- Worker 启动时指定 tags，只消费匹配的 job
- 无 tag 的 job 只被 `--server-and-worker` 模式的 worker 处理
- 支持分布式：不同机器跑不同 tag 的 worker

**工作量**：1-2 天

---

### 5. YAML Fixture Seeding

**来源**：Loco `db::seed` + YAML fixtures

**现状**：测试数据通过 `sqlx::query("INSERT INTO ...")` 手写，开发环境也需要手动创建测试用户。生产环境初始化（默认角色、管理员账户）通过 migration SQL 硬编码。

**方案**：

```yaml
# src/fixtures/users.yaml
---
- id: 1
  email: admin@raisfast.com
  username: admin
  password_hash: "$2b$12$..."  # pre-hashed "admin123"
  role: admin
  created_at: "2026-01-01T00:00:00Z"
- id: 2
  email: author@test.com
  username: test_author
  password_hash: "$2b$12$..."
  role: author
  created_at: "2026-01-01T00:00:00Z"
```

```bash
# CLI
raisfast db seed                    # 从 src/fixtures/ 加载所有 YAML
raisfast db seed --reset            # 先清空再加载
raisfast db seed --only=users,posts # 只加载指定表
raisfast db seed --dump             # 导出当前 DB 为 YAML fixtures
```

与 Laravel 路线图中 Factory 的关系：
- **Factory**（Laravel 路线图 P0-2）：运行时生成随机测试数据（`UserFactory::new().create()`）
- **Fixture**（本项）：声明式固定数据（YAML 文件），用于开发和集成测试的基线数据
- 两者互补：Factory 生成 → Fixture 导出 → 开发环境 seed

**工作量**：1-2 天

---

### 6. Storage Mirror/Backup 策略

**来源**：Loco Storage（Mirror Strategy + Backup Strategy + FailureMode）

**现状**：`src/storage/` 有基本的文件系统存储抽象，但只有一个后端。上传文件存本地磁盘，无冗余。

**方案**：多后端 Storage + 策略模式：

```rust
// src/storage/strategy.rs

pub trait StorageStrategy: Send + Sync {
    async fn upload(&self, path: &Path, data: &[u8]) -> Result<()>;
    async fn download(&self, path: &Path) -> Result<Vec<u8>>;
    async fn delete(&self, path: &Path) -> Result<()>;
}

/// 主 + 镜像：写入同步到所有后端，读取从主后端
pub struct MirrorStrategy {
    primary: Box<dyn StorageDriver>,
    secondaries: Vec<Box<dyn StorageDriver>>,
    failure_mode: MirrorFailureMode,
}

/// 主 + 备份：写入主 + 异步备份，读取从主
pub struct BackupStrategy {
    primary: Box<dyn StorageDriver>,
    backups: Vec<Box<dyn StorageDriver>>,
    failure_mode: BackupFailureMode,
}

pub enum MirrorFailureMode {
    AllMustSucceed,        // 任一副本失败 → 整体失败
    AllowMirrorFailure,    // 主成功即可，副本次要
}

pub enum BackupFailureMode {
    AllMustSucceed,
    AllowBackupFailure,
    AtLeastOneSuccess,
    CountSuccess(usize),
}
```

使用场景：
- **Mirror**：本地磁盘 + S3，读写都双写
- **Backup**：本地磁盘主存储 + 每晚备份到 S3
- **插件集成**：上传插件可以通过 Storage trait 接入任意后端

**工作量**：3-5 天（含 S3 driver）

---

### 7. Shortcode 系统（内容扩展语法）

**来源**：Zola Shortcodes

**现状**：raisfast 的 page_block 系统支持 JSON 定义的块类型（paragraph、heading、image、code、quote、callout），但用户无法自定义内容扩展语法。想要嵌入 YouTube、Gist、自定义组件需要在 markdown 中写 HTML。

**方案**：借鉴 Zola Shortcodes，在 Markdown 解析层注册可扩展的 shortcode：

```markdown
# 在文章内容中使用

这是一个视频：

{{ youtube(id="dQw4w9WgXcQ", autoplay=true) }}

引用外部数据：

{% book_list(path="/data/books.toml") %}
{% end %}

自定义图表：

{{ chart(type="bar", data=[10, 20, 30], labels=["A", "B", "C"]) }}
```

Shortcode 注册：

```rust
// 内置 shortcode
pub fn builtin_shortcodes() -> Vec<Box<dyn Shortcode>> {
    vec![
        Box::new(youtube::YoutubeShortcode),
        Box::new(gist::GistShortcode),
        Box::new(chart::ChartShortcode),
        Box::new(gallery::GalleryShortcode),
    ]
}

// 插件注册自定义 shortcode
pub trait Shortcode: Send + Sync {
    fn name(&self) -> &str;
    fn render(&self, args: &HashMap<String, Value>, body: Option<&str>) -> Result<String>;
}
```

设计要点：
- Shortcode 在 Markdown 解析**前**展开，生成标准 Markdown 或 HTML
- 支持**有 body**（`{% shortcode() %}...{% end %}`）和**无 body**（`{{ shortcode() }}`）两种形式
- 参数类型安全：string / bool / int / float / array
- 插件可通过 `plugin.toml` 注册自定义 shortcode

**工作量**：3-5 天

---

## P2 — 值得但优先级低

### 8. SharedStore（类型安全全局状态）

**来源**：Loco `AppContext.shared_store`

**现状**：`AppState` 是固定字段 struct，新增一个全局服务必须改 struct 定义 + 构造函数 + 所有引用点。

**方案**：`AppState` 内置 heterogeneous 容器，任意类型可注入：

```rust
// 类似 Loco 的 SharedStore
pub struct SharedStore {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl SharedStore {
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(val));
    }
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

// 使用
ctx.shared_store.insert(MyApiClient::new(api_key));
let client = ctx.shared_store.get::<MyApiClient>().unwrap();
```

> 注意：这与 Spring Boot 路线图中的 Service Locator（P0-1）功能重叠。建议二选一。SharedStore 更轻量但不支持 trait object；Service Locator 支持 `Arc<dyn Trait>` 但复杂度更高。考虑到 raisfast 已大量使用 `Arc<dyn XxxRepository>`，推荐 Service Locator 方案。

---

### 9. 其他可借鉴特性

| 特性 | 来源 | 说明 | 工作量 |
|---|---|---|---|
| **English-to-cron** | Loco Scheduler | `"every 15 seconds"` / `"at 10:00 am"` 自动转 cron 表达式 | 1 天 |
| **Job Queue 管理 CLI** | Loco `jobs` | `raisfast jobs cancel/tidy/purge/dump/import` | 2-3 天 |
| **Asset Colocation** | Zola | 上传文件与内容关联（`post/123/image.png`） | 1 天 |
| **Image Processing** | Zola `resize_image` | 内置图片裁剪/缩放/格式转换（已有 thumbnail worker 但是异步） | 2-3 天 |
| **Live Reload** | Zola `zola serve` | Admin 前端 dev 模式热更新（已有 Vite HMR，但后端数据变更无推送） | 1-2 天 |
| **`routes` 命令** | Loco | `raisfast routes` 列出所有注册路由，方便调试 | 0.5 天 |
| **`middleware` 命令** | Loco | `raisfast middleware` 列出所有中间件 | 0.5 天 |
| **Config Tera 模板** | Loco | YAML 配置文件中 `{{ get_env(name="KEY") }}` 引用环境变量 | 1 天 |

---

## 实施优先级

```
Scaffold 代码生成器 (P0-1)         ← 最高 ROI，开发体验飞跃
    ↓
Initializer 启动生命周期 (P0-2)    ← 架构级改进
    ↓
Doctor 诊断命令 (P0-3)            ← 运维体验
    ↓
Worker Tag 过滤 (P1-4)            ← 分布式部署必需
    ↓
YAML Fixture Seeding (P1-5)       ← 测试和开发体验
    ↓
Storage 策略 (P1-6)               ← 生产可靠性
    ↓
Shortcode 系统 (P1-7)             ← 内容扩展能力
    ↓
其余 P2 项按需排期
```

---

## 三份路线图汇总

| 路线图 | 聚焦 | 核心特性 |
|---|---|---|
| `laravel-inspiration.md` | **数据层** | Factory、Migration 回滚、API Resource、Query Scope、Lifecycle Hook、Policy |
| `springboot-inspiration.md` | **架构 & 运维** | Service Locator、Transactional 宏、Actuator、Profile、Feature Flag、路由拆分 |
| 本文档 | **DX & 内容** | Scaffold 生成器、Initializer、Doctor、Worker Tag、Fixture、Storage 策略、Shortcode |

### 建议实施顺序（跨路线图整合）

```
Phase 1 — 架构基础（2-3 周）
  ├── Service Locator / Initializer（Spring P0-1 + Loco P0-2）
  ├── #[transactional] 宏（Spring P0-2）
  └── 路由注册拆分（Spring P1-7）

Phase 2 — 开发体验（2-3 周）
  ├── Scaffold 代码生成器（Loco P0-1）
  ├── Factory derive 宏（Laravel P0-2）
  ├── YAML Fixture（Loco P1-5）
  └── Profile 多环境配置（Spring P0-4）

Phase 3 — 数据层完善（2-3 周）
  ├── Migration 回滚（Laravel P0-1）
  ├── API Resource（Laravel P0-3）
  ├── Lifecycle Hook（Laravel P0-4）
  └── Shortcode（Loco P1-7）

Phase 4 — 运维 & 生产化（2-3 周）
  ├── Actuator 健康检查（Spring P0-3）
  ├── Doctor 诊断（Loco P0-3）
  ├── Worker Tag（Loco P1-4）
  └── Storage 策略（Loco P1-6）
```
