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

**方案**：借鉴 Laravel Factory 模式，为每个模型提供 Builder：

```rust
// src/test_factory.rs 或 src/testing/mod.rs

let user = UserFactory::new(&pool)
    .email("admin@test.com")
    .role("admin")
    .create()
    .await;

let post = PostFactory::new(&pool)
    .author(&user)
    .status("published")
    .tags(&[&tag1, &tag2])
    .create()
    .await;
```

每个 Factory 封装：
- 必填字段的默认值生成（UUID、时间戳、随机字符串）
- 外键关联自动解析
- 可链式覆盖任意字段

**收益**：测试代码量砍半，可读性翻倍，新增测试不再是体力活。

**工作量**：3-5 天（覆盖核心 8 个模型）

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

### 4. Model Observer（数据生命周期钩子）

**现状**：`slug` 生成、`excerpt` 截取、时间戳填充散落在各 service 函数中，难以复用和扩展。

**方案**：统一的生命周期事件钩子，和现有 EventBus 互补：

```rust
// 观察者注册（在 startup 时）
observer::on("posts", Event::BeforeCreate, |ctx| {
    ctx.data["slug"] = generate_slug(&ctx.data["title"]);
    ctx.data["excerpt"] = extract_excerpt(&ctx.data["content"], 200);
});

observer::on("posts", Event::AfterCreate, |ctx| {
    ctx.eventbus.emit(Event::PostCreated { id: ctx.id });
});

observer::on("users", Event::BeforeCreate, |ctx| {
    ctx.data["password_hash"] = hash_password(ctx.data["password"]);
});
```

生命周期事件：
- `BeforeValidate` — 数据校验前
- `BeforeCreate` / `BeforeUpdate` — 写入前修改数据
- `AfterCreate` / `AfterUpdate` — 写入后触发副作用
- `BeforeDelete` / `AfterDelete` — 删除前后

插件系统可以 `register_observer` 接入，无需修改核心代码。

**工作量**：3-5 天

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

**现状**：每个 handler 重复拼 SQL WHERE 条件（`status = 'published'`、`created_at DESC` 等）。

**方案**：

```rust
// src/repositories/scopes.rs

pub fn published(query: &mut QueryBuilder) {
    query.where("status = 'published'");
}

pub fn by_author(query: &mut QueryBuilder, author_id: i64) {
    query.where("created_by = ?").bind(author_id);
}

// 使用
let mut q = QueryBuilder::select("posts");
published(&mut q);
by_author(&mut q, user.id);
q.order("created_at", "DESC");
let posts = q.fetch_all(&pool).await?;
```

**工作量**：2-3 天

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
| **Config Cache** | `.env` 解析结果序列化缓存，避免每次重启重读 | 1 天 |
| **Rate Limit per User** | 按 user_id / IP 限流，当前只有全局/路由级 | 1-2 天 |
| **Scheduled Tasks 声明式** | Fluent API 替代 JSON cron 配置 | 2-3 天 |
| **Storage Symlink** | `storage:link` 命令创建 `public/storage → storage/uploads` 软链 | 0.5 天 |
| **Telescope 调试面板** | 记录 SQL 查询、请求日志、异常到 SQLite，管理后台查看 | 5-7 天 |
| **Horizon 队列监控** | Worker 任务状态、失败重试的可视化 | 5-7 天 |

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
Observer (P0-4)
    ↓
Policy (P1-5)
    ↓
维护模式 (P1-7)
    ↓
其余 P2 项按需排期
```

前两项改动小收益大，中间两项是安全和可维护性相关，Observer 和 Policy 是架构升级。
