# Laravel 设计借鉴路线图

> 从 Laravel 框架中提取适合 Rust/CMS 场景的设计模式，作为项目演进参考。
> 已完成：`STORAGE_ROOT_DIR` 统一存储目录（灵感来自 Laravel `storage/` + `.env` 约定）

---

## P0 — 高价值（解决实际痛点）

### 1. Migration 回滚

**现状**：只有 `db migrate`，不能回滚。生产环境出问题时无法快速恢复。

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

**现状**：测试中到处手写 `sqlx::query("INSERT INTO users ...")`，重复且易错。

**方案**：使用 derive 宏自动生成 Factory，而非 Laravel 式的手写 builder。Rust 的类型系统可以在编译期保证必填字段不为空，无需运行时校验。

```rust
// 在 proc-macro crate 中定义 derive 宏

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

// 自动生成:
// - UserFactory::new(pool) -> UserFactoryBuilder<MissingRequiredFields>
// - .email("x@t.com") -> UserFactoryBuilder<HasEmail>
// - .create() 编译通过（所有必填字段已设置）
```

核心设计点：
- `#[derive(Factory)]` 在 proc-macro crate 中实现，解析字段属性生成 builder
- 利用泛型状态机（phantom-typed builder）在**编译期**强制必填字段
- `#[factory(default = ...)]` 标记可选覆盖字段，不填则使用默认值
- 外键关联通过 `.associate("post", &post)` 自动解析
- 生成的代码直接执行 `sqlx::query("INSERT INTO ...")` ，不引入额外 ORM 层

**收益**：测试代码量砍半，可读性翻倍，新增测试不再是体力活。

**工作量**：3-5 天（含 proc-macro crate + 覆盖核心 8 个模型）

---

### 3. API Resource（响应转换层）

**现状**：handler 里直接 `json!({"code":0, "data": post})`，DB 字段（`password_hash`、`int_id`、内部状态）直接暴露给前端。

**方案**：显式的响应转换层，类似 Laravel Resource：

```rust
// src/dto/post.rs 已有 PostResponse，但没有强制使用
// 目标：handler 只返回 Resource，禁止直接序列化 Model

pub struct PostResource {
    pub id: String,
    pub title: String,
    pub slug: String,
    pub excerpt: Option<String>,
    pub author: UserBrief,
    pub tags: Vec<TagBrief>,
    pub created_at: String,
    // 不暴露：password_hash, int_id, tenant_id 等内部字段
}

impl PostResource {
    pub fn from_model(post: Post, author: User, tags: Vec<Tag>) -> Self { ... }
}
```

Handler 层强制约定：
- `list` → `Vec<PostResource>`
- `get` → `PostResource`
- `create` / `update` → `PostResource`
- 禁止在 handler 中直接 `serde_json::to_value(&model)`

**收益**：安全性（不泄露内部字段）、一致性（所有 API 响应格式统一）、可维护性（改字段只改 Resource）。

**工作量**：2-3 天（覆盖 6 个核心模型的 Resource）

---

### 4. Lifecycle Hook（数据生命周期钩子）

**现状**：`slug` 生成、`excerpt` 截取、时间戳填充散落在各 service 函数中，难以复用和扩展。

**方案**：基于现有 `EventBus`（`src/eventbus.rs`）构建 Lifecycle Hook 系统。与 Laravel Observer 不同，Rust 版本不使用独立观察者注册表，而是复用已有的 pub/sub 基础设施，以 sync callback 形式在 service 层拦截。

```rust
// src/lifecycle.rs — 基于 EventBus 的钩子注册

use crate::eventbus::Event;

pub struct LifecycleHooks;

impl LifecycleHooks {
    /// 在 startup 时调用，注册所有内置钩子
    pub fn register(bus: &EventBus) {
        // Slug 自动生成 — 写入前拦截
        bus.on_sync(Event::PostCreating, |ctx| {
            ctx.data["slug"] = generate_slug(&ctx.data["title"]);
        });

        // Excerpt 自动截取
        bus.on_sync(Event::PostCreating, |ctx| {
            ctx.data["excerpt"] = extract_excerpt(&ctx.data["content"], 200);
        });

        // 搜索索引更新 — 写入后异步触发
        bus.on_sync(Event::PostCreated, |ctx| {
            bus.emit(Event::SearchIndexUpdate { id: ctx.id });
        });
    }
}
```

设计要点：
- **复用 EventBus**：`PostCreating` / `PostCreated` 等事件已在 `eventbus.rs` 中定义（`#[non_exhaustive]` enum）
- **Sync hook**（`on_sync`）：在 service 写入 DB 前同步执行，可修改数据（slug、excerpt、password_hash）
- **Async hook**：写入后通过 `bus.emit()` 异步触发副作用（搜索索引、webhook、通知）
- **插件集成**：插件通过 `bus.on_sync(Event::Custom { ... }, handler)` 注册钩子
- 生命周期事件映射：`PostCreating` → before create, `PostCreated` → after create，无需新建事件体系

与 Laravel Observer 的区别：
| | Laravel Observer | Rust Lifecycle Hook |
|---|---|---|
| 注册方式 | 独立 `Observer` 类 + `observe()` | 复用 `EventBus::on_sync()` |
| 数据修改 | `$event->model->slug = ...` | `ctx.data["slug"] = ...` |
| 异步 | 无（同步框架） | Sync 拦截 + Async 副作用 |
| 插件扩展 | Service Provider 注册 | `bus.on_sync(Event::Custom)` |

**工作量**：2-3 天

---

## P1 — 中价值（提升工程质量）

### 5. Policy（资源级权限）

**现状**：RBAC 是全局角色（admin/author/member），没有"这篇文章只有作者能编辑"的资源级判断。`require_owner_or_admin` 已在做类似的事但没有体系化。

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
- 自动生成权限检查中间件

**工作量**：2-3 天

---

### 6. Transaction 封装

**现状**：多步写操作没有事务保护，部分失败会导致数据不一致。

**方案**：封装 sqlx 事务为 ergonomics API：

```rust
// src/db/transaction.rs

pub async fn transaction<F, T>(pool: &Pool, f: F) -> AppResult<T>
where
    F: FnOnce(&mut sqlx::Transaction<sqlx::Any>) -> BoxFuture<AppResult<T>>,
{
    let mut tx = pool.begin().await?;
    let result = f(&mut tx).await?;
    tx.commit().await?;
    Ok(result)
}

// 使用
transaction(&pool, |tx| Box::pin(async move {
    tx.execute("INSERT INTO posts ...").await?;
    tx.execute("INSERT INTO post_tags ...").await?;  // 失败自动 rollback
    Ok(post)
})).await?;
```

优先应用场景：
- `create_post`（post + tags 关联）
- `create_workflow_instance`（instance + step_logs）
- `delete_user`（用户 + 关联内容清理）

**工作量**：1-2 天

---

### 7. 维护模式

**现状**：部署时如果有正在进行的写操作，可能产生脏数据。

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

### 8. Query Scope（可复用查询片段）

**现状**：每个 handler 重复拼 SQL WHERE 条件（`status = 'published'`、`created_at DESC` 等）。当前使用 `format!()` + `ph()` 手动拼接。

**方案**：Laravel 使用 `&mut QueryBuilder` 传入 scope 函数，但这种 `&mut` 模式在 Rust 中组合性差（借用冲突）。改为**方法链 + 泛型状态机**实现类型安全的查询组合：

```rust
// src/repositories/scopes.rs

/// Phantom-typed state machine：编译期追踪已应用的 filter
pub struct Query<S: ScopeState = Init> {
    table: &'static str,
    clauses: Vec<String>,
    params: Vec<SqliteValue>,
    _state: PhantomData<S>,
}

// 状态标记
pub struct Init;
pub struct Published;
pub struct ByAuthor { author_id: i64 }
pub struct Paginated { page: u32, page_size: u32 }

impl Query<Init> {
    pub fn from(table: &'static str) -> Self { ... }
}

impl<S: ScopeState> Query<S> {
    /// 所有 scope 方法返回新类型，链式调用
    pub fn published(self) -> Query<Published> {
        // status = 'published' 已编译期嵌入
        ...
    }

    pub fn by_author(self, id: i64) -> Query<ByAuthor> {
        ...
    }

    pub fn paginate(self, page: u32, size: u32) -> Query<Paginated> {
        ...
    }
}

// 使用 — 编译期保证 published() 只调用一次
let posts = Query::from("posts")
    .published()
    .by_author(user.id)
    .paginate(1, 20)
    .fetch_all::<Post>(&pool)
    .await?;
```

设计要点：
- **Phantom-typed state machine**：每个 `scope` 方法消费 `self` 并返回新状态类型，防止重复调用（`published().published()` 编译报错）
- **与 `ph()` 兼容**：内部仍使用 `db::dialect::ph()` 生成跨库占位符
- **零运行时开销**：所有状态追踪在编译期完成
- **可组合**：不同 scope 状态可以自由组合，只需为最终状态实现 `fetch_all`

与 Laravel Scope 的区别：
| | Laravel Scope | Rust Query Scope |
|---|---|---|
| 机制 | `&mut QueryBuilder` 传入函数 | 方法链消费 self 返回新状态 |
| 安全性 | 运行时可能重复应用 filter | 编译期防止重复 |
| 组合性 | 自由组合，可能冲突 | 类型状态显式编码组合关系 |
| 跨库 | Eloquent 抽象 | 复用 `ph()` / `dialect` |

**工作量**：3-5 天

---

### 9. 数据库队列驱动

**现状**：Worker 队列在内存中，进程重启丢任务。

**方案**：SQLite 作为队列表：

```sql
CREATE TABLE queue_jobs (
    id INTEGER PRIMARY KEY,
    queue TEXT NOT NULL DEFAULT 'default',
    payload TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    reserved_at TEXT,
    available_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

Worker 从队列表拉取，处理后删除。重启不丢任务。

**工作量**：3-5 天

---

### 10. 其他可借鉴特性

| 特性 | 说明 | 工作量 |
|------|------|--------|
| **Rate Limit per User** | 按 user_id / IP 限流，当前只有全局/路由级 | 1-2 天 |
| **Scheduled Tasks 声明式** | Fluent API 替代 JSON cron 配置 | 2-3 天 |
| **Storage Symlink** | `storage:link` 命令创建 `public/storage → storage/uploads` 软链 | 0.5 天 |
| **Telescope 调试面板** | 记录 SQL 查询、请求日志、异常到 SQLite，管理后台查看 | 5-7 天 |
| **Horizon 队列监控** | Worker 任务状态、失败重试的可视化 | 5-7 天 |

> **注意**：Laravel 的 **Config Cache** 在 Rust 中无意义。`AppConfig::from_env()` 启动时一次性从 `.env` 读取所有配置，存入 `Arc<AppConfig>` 全生命周期共享，已经是"零成本缓存"。无需额外缓存层。

---

## 实施优先级

```
Transaction 封装 (P1-6)
    ↓
Factory / Seeder (P0-2)
    ↓
API Resource 转换层 (P0-3)
    ↓
Migration 回滚 (P0-1)
    ↓
Lifecycle Hook (P0-4)
    ↓
Policy (P1-5)
    ↓
维护模式 (P1-7)
    ↓
其余 P2 项按需排期
```

前两项改动小收益大，中间两项是安全和可维护性相关，Lifecycle Hook 和 Policy 是架构升级。

## Laravel → Rust 适配原则

| Laravel 模式 | Rust 适配 | 原因 |
|---|---|---|
| Factory（手写 builder） | `#[derive(Factory)]` proc-macro + phantom-typed builder | Rust 类型系统可在编译期保证必填字段，无需手写 |
| Observer（独立注册表） | Lifecycle Hook（复用 EventBus） | 项目已有完整 EventBus pub/sub，不需要第二套注册机制 |
| Scope（`&mut QueryBuilder`） | 方法链 + 泛型状态机 | `&mut` 模式在 Rust 中组合性差，phantom-typed state 更安全 |
| Config Cache | 不需要 | Rust 启动时一次读取 + `Arc` 共享，已等效缓存 |
| Migration / Policy / Transaction | 直接移植 | 这些模式与语言无关，Rust 实现几乎一致 |
