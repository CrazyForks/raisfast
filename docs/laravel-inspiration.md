# Laravel 设计借鉴路线图

> 从 Laravel 框架中提取适合 Rust/CMS 场景的设计模式，作为项目演进参考。

## 已完成

| 项目 | 完成时间 | 说明 |
|------|---------|------|
| `STORAGE_ROOT_DIR` 统一存储目录 | 早期 | 灵感来自 Laravel `storage/` + `.env` 约定 |
| **Transaction 封装** (`in_transaction!` 宏) | Phase 4 | 30+ 调用点覆盖 auth、payment、order、wallet 等核心服务 |
| **数据库队列驱动** (`SqliteJobQueue`) | Phase 5 | `src/worker/sqlite_queue.rs`，完整 SQLite 持久化队列 |
| **AOP Lifecycle Hook** (`AspectEngine`) | Phase 5 | `src/aspects/engine.rs`，before/after create/read/update/delete 全生命周期拦截 |
| **DTO 响应转换层**（核心模型） | Phase 8 | `PostResponse`、`PaymentOrderResponse` 等 DTO + `From<Model>` 过滤 |

---

## P0 — 高价值（解决实际痛点）

### 1. Migration 回滚

**现状**：只有 `db migrate`，不能回滚。`_migrations` 表无 `batch` 列，CLI 无 `rollback` 命令。生产环境出问题时无法快速恢复。

**方案**：每个迁移文件拆成 `up` / `down` 两个文件：

```
migrations/sqlite/
├── 001_schema.up.sqlite.sql          # 现有的 schema
├── 002_tenantable.up.sqlite.sql      # 现有的迁移
└── 002_tenantable.down.sqlite.sql    # 新增：回滚用
```

新增 CLI 子命令：

```bash
raisfast db migrate          # 执行所有 up
raisfast db rollback          # 回滚最后一批
raisfast db rollback --step=3 # 回滚最近 3 个
```

`_migrations` 表增加 `batch` 列，`rollback` 回滚最大 batch 的所有条目。

**工作量**：2-3 天

---

### 2. Factory / Seeder（测试数据工厂）

**现状**：测试中仍有 13 处原始 `sqlx::query("INSERT INTO users ...")`，重复且易错（`tests/api.rs` 4 处、`tests/api/api_token.rs` 5 处、`tests/api/tenant_e2e.rs` 3 处）。

**方案**：使用 derive 宏自动生成 Factory，而非 Laravel 式的手写 builder。Rust 的类型系统可以在编译期保证必填字段不为空，无需运行时校验。

```rust
#[derive(Factory)]
#[factory(table = "users")]
struct UserFactory {
    #[factory(default = "Uuid::now_v7().to_string()")]
    id: String,
    #[factory(default = "format!(\"user_{}@test.com\", random_suffix())")]
    email: String,
    #[factory(default = "format!(\"user_{}\", random_suffix())")]
    username: String,
    #[factory(default = "\"$2b$12$fake_hash\".to_string()")]
    password_hash: String,
    #[factory(default = "Role::Author")]
    role: Role,
    #[factory(default = "Utc::now().to_rfc3339()")]
    created_at: String,
}

// 自动生成 phantom-typed builder，编译期强制必填字段
```

核心设计点：
- `#[derive(Factory)]` 在 proc-macro crate 中实现，解析字段属性生成 builder
- 利用泛型状态机（phantom-typed builder）在**编译期**强制必填字段
- `#[factory(default = ...)]` 标记可选覆盖字段，不填则使用默认值
- 外键关联通过 `.associate("post", &post)` 自动解析
- 生成的代码直接执行 `sqlx::query("INSERT INTO ...")`，不引入额外 ORM 层

**收益**：测试代码量砍半，可读性翻倍，新增测试不再是体力活。

**工作量**：3-5 天（含 proc-macro crate + 覆盖核心 8 个模型）

---

### 3. API Resource（响应转换层 — 收尾）

**现状**：核心模型（post、order、payment、wallet、product）已有 `PostResponse`、`PaymentOrderResponse` 等 DTO 并配有 `From<Model>` 过滤。但存在两个遗留问题：

1. **16 处 `json!()` 调用**：`health.rs`、`options.rs`、`content_revision.rs`、`sse.rs`、`api_token.rs`、`tenant.rs`、`rbac.rs` 等轻量 handler 仍直接用 `json!()` 构造响应
2. **内部字段暴露**：部分 DTO 仍暴露内部 ID（如 `created_by: i64`）、`tenant_id` 等

**方案**：审计并收尾，非从零开始。

```rust
// 已有模式（保持）：
// src/dto/post.rs → PostResponse + impl From<Post>
// src/dto/payment.rs → PaymentOrderResponse + impl From<PaymentOrder>

// 待修复示例：
// PostResponse.created_by: i64 → 改为 author_name: String（已有但 created_by 未移除）
// 16 处 json!() → 改为 ApiResponse::success(data) + DTO
```

**工作量**：1-2 天（审计 + 修复，非新建）

---

### 4. 领域 Aspect 注册（slug 生成、excerpt 截取等）

**现状**：AOP Aspect Engine（`src/aspects/engine.rs`）已提供完整的 before/after create/read/update/delete 生命周期拦截。`TimestampableAspect`、`OwnableAspect` 等内置切面已实现。但**领域特定的钩子**（slug 自动生成、excerpt 自动截取、搜索索引同步）仍散落在各 service 函数中，未注册为 Aspect。

**方案**：将散落的领域逻辑注册为 Aspect，而非新建生命周期系统。

```rust
// src/aspects/slug_aspect.rs — 注册到现有 AspectEngine

pub struct SlugAspect;

impl SlugAspect {
    pub fn register(engine: &mut AspectEngine) {
        engine.on_before_create("posts", |data| {
            if data.get("slug").is_none_or_empty() {
                data["slug"] = generate_slug(&data["title"]);
            }
            Ok(())
        });
    }
}

// src/aspects/excerpt_aspect.rs

pub struct ExcerptAspect;

impl ExcerptAspect {
    pub fn register(engine: &mut AspectEngine) {
        engine.on_before_create("posts", |data| {
            data["excerpt"] = extract_excerpt(&data["content"], 200);
            Ok(())
        });
    }
}
```

与原文档的区别：
- ~~基于 EventBus 构建~~ → 基于 **已有的 AspectEngine** 注册
- ~~新增 `src/lifecycle.rs`~~ → 新增 `src/aspects/slug_aspect.rs` 等切面文件
- 基础设施已就绪，只需注册领域钩子

**工作量**：1-2 天

---

## P1 — 中价值（提升工程质量）

### 5. Policy（资源级权限）

**现状**：RBAC 已有 `PermissionGuard` + action/subject 检查 + AOP Access Layer（`AccessCheckContext`、`AccessFilterContext`）。但缺少 per-resource 的类型化 `Policy` trait（如"只有文章作者能编辑自己的文章"），这类判断仍散落在 handler 中。

**方案**：

```rust
// src/policy.rs

trait Policy {
    type Resource;
    fn view(user: &AuthUser, resource: &Self::Resource) -> bool;
    fn update(user: &AuthUser, resource: &Self::Resource) -> bool;
    fn delete(user: &AuthUser, resource: &Self::Resource) -> bool;
}

struct PostPolicy;
impl Policy for PostPolicy {
    type Resource = Post;
    fn update(user: &AuthUser, post: &Post) -> bool {
        user.role == "admin" || post.created_by == user.doc_id
    }
}

// handler 中使用
if !PostPolicy::update(&auth, &post) {
    return Err(AppError::Forbidden);
}
```

收益：
- 权限逻辑集中，不在 handler/service 中散落
- 可被插件覆盖或扩展
- 与现有 `PermissionGuard` 互补（Guard 管全局，Policy 管资源级）

**工作量**：2-3 天

---

### 6. 维护模式

**现状**：无维护模式。部署时如果有正在进行的写操作，可能产生脏数据。

**方案**：

```bash
raisfast down              # 启用维护模式
raisfast up                # 关闭维护模式
```

实现：在 storage 根目录创建 `maintenance` 标记文件，中间件检测到后所有请求返回 503。需要排除 `/api/v1/auth/login` 等关键路径。

```rust
// middleware
if storage_root.join("maintenance").exists() && !is_exempt_path(&req) {
    return Err(AppError::ServiceUnavailable("maintenance mode"));
}
```

**工作量**：0.5-1 天

---

## P2 — 值得但优先级低

### 7. Query Scope（可复用查询片段）

**现状**：Repository 层仍用 `format!()` + `ph()` 手动拼接 SQL WHERE 条件（`status = 'published'`、`created_at DESC` 等）。命令对象（`CreatePostCmd` 等）解决了参数传递，但查询构建无可复用抽象。

**方案**：Laravel 使用 `&mut QueryBuilder` 传入 scope 函数，但这种 `&mut` 模式在 Rust 中组合性差（借用冲突）。改为**方法链 + 泛型状态机**实现类型安全的查询组合：

```rust
// src/repositories/scopes.rs

pub struct Query<S: ScopeState = Init> {
    table: &'static str,
    clauses: Vec<String>,
    params: Vec<SqliteValue>,
    _state: PhantomData<S>,
}

pub struct Init;
pub struct Published;
pub struct ByAuthor { author_id: i64 }
pub struct Paginated { page: u32, page_size: u32 }

impl Query<Init> {
    pub fn from(table: &'static str) -> Self { ... }
}

impl<S: ScopeState> Query<S> {
    pub fn published(self) -> Query<Published> { ... }
    pub fn by_author(self, id: i64) -> Query<ByAuthor> { ... }
    pub fn paginate(self, page: u32, size: u32) -> Query<Paginated> { ... }
}

let posts = Query::from("posts")
    .published()
    .by_author(user.id)
    .paginate(1, 20)
    .fetch_all::<Post>(&pool)
    .await?;
```

设计要点：
- Phantom-typed state machine：编译期防止重复调用（`published().published()` 编译报错）
- 与 `ph()` 兼容：内部仍使用 `db::dialect::ph()` 生成跨库占位符
- 零运行时开销

**工作量**：3-5 天

---

### 8. 其他可借鉴特性

| 特性 | 说明 | 工作量 |
|------|------|--------|
| **Rate Limit per User** | 按 user_id 限流，当前只有 per-IP + per-API-token | 1-2 天 |
| **Scheduled Tasks 声明式** | Fluent API 替代 JSON cron 配置 | 2-3 天 |
| **Storage Symlink** | `storage:link` 命令创建 `public/storage → storage/uploads` 软链 | 0.5 天 |
| **Telescope 调试面板** | 记录 SQL 查询、请求日志、异常到 SQLite，管理后台查看 | 5-7 天 |
| **Horizon 队列监控** | Worker 任务状态、失败重试的可视化 | 5-7 天 |

> **注意**：Laravel 的 **Config Cache** 在 Rust 中无意义。`AppConfig::from_env()` 启动时一次性从 `.env` 读取所有配置，存入 `Arc<AppConfig>` 全生命周期共享，已经是"零成本缓存"。

---

## 实施优先级

```
Factory / Seeder (P0-2)
    ↓
API Resource 收尾 (P0-3)
    ↓
Migration 回滚 (P0-1)
    ↓
领域 Aspect 注册 (P0-4)
    ↓
Policy (P1-5)
    ↓
维护模式 (P1-6)
    ↓
其余 P2 项按需排期
```

Factory/Seeder 改动大收益高；API Resource 收尾工作量小但安全价值高；Migration 回滚是生产必备；领域 Aspect 和 Policy 是架构升级。

## Laravel → Rust 适配原则

| Laravel 模式 | Rust 适配 | 原因 |
|---|---|---|
| Factory（手写 builder） | `#[derive(Factory)]` proc-macro + phantom-typed builder | Rust 类型系统可在编译期保证必填字段，无需手写 |
| Observer（独立注册表） | Aspect 切面（注册到 `AspectEngine`） | 项目已有完整 AOP 引擎，直接注册领域钩子 |
| Scope（`&mut QueryBuilder`） | 方法链 + 泛型状态机 | `&mut` 模式在 Rust 中组合性差，phantom-typed state 更安全 |
| Config Cache | 不需要 | Rust 启动时一次读取 + `Arc` 共享，已等效缓存 |
| Migration / Policy | 直接移植 | 这些模式与语言无关，Rust 实现几乎一致 |
| Transaction | `in_transaction!` 宏 | ✅ 已完成 |
| Database Queue | `SqliteJobQueue` | ✅ 已完成 |
