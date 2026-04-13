# 改进计划

> 基于项目审计，按优先级列出待修复和改进项。

## P0 — 关键问题（影响正确性/安全）

### 1. 特定限流器完全失效

**文件：** `src/middleware/rate_limit.rs`

**问题：** `rate_limit_fn!` 宏在每次请求中创建新的 `RateLimiter` 实例（空 HashMap），导致登录（10次/分）、注册（5次/时）、评论（3次/分）的限流永远不触发。仅全局限流器（通过 `Extension` 共享）有效。

**修复方案：** 将特定限流器也放入 `AppState`，或通过 `Extension` 共享多个命名限流器实例。

```rust
// 方案 A：AppState 中持有多个限流器
pub struct AppState {
    pub pool: SqlitePool,
    pub config: Arc<AppConfig>,
    pub rate_limiters: RateLimiterSet,
}

pub struct RateLimiterSet {
    pub global: RateLimiter,
    pub register: RateLimiter,
    pub login: RateLimiter,
    pub comment: RateLimiter,
}
```

**验收标准：** 连续发送 11 次登录请求，第 11 次应返回 429。

---

### 2. N+1 查询

**文件：** `src/services/post.rs` — `build_post_response()`

**问题：** 文章列表每篇文章执行 3 次额外查询（tags、author_name、category_name），N 篇文章共 3N+1 次查询。

**修复方案：** 改用 JOIN 一次查询，或批量查询后 HashMap 映射。

```sql
-- 方案 A：JOIN 查询
SELECT p.*, c.name AS category_name, u.username AS author_name
FROM posts p
LEFT JOIN categories c ON p.category_id = c.id
LEFT JOIN users u ON p.author_id = u.id
WHERE p.status = 'published'
ORDER BY p.created_at DESC
LIMIT ? OFFSET ?;

-- tags 批量查询
SELECT pt.post_id, t.id, t.name, t.slug
FROM posts_tags pt
JOIN tags t ON pt.tag_id = t.id
WHERE pt.post_id IN (...);
```

**验收标准：** `list_posts` 查询次数从 3N+1 降至 2~3 次。

---

### 3. 密码验证不完整

**文件：** `src/models/user.rs` — `RegisterRequest`

**问题：** 仅校验长度 ≥ 8，未检查必须包含字母和数字。`aaaaaaaa` 可通过验证。

**修复方案：** 增加 `validate` 自定义函数，或使用 `validator` 的 `regex` / `custom` 校验。

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 2, max = 50))]
    pub username: String,
    #[validate(length(min = 8, max = 128), custom(function = "validate_password"))]
    pub password: String,
}

fn validate_password(pwd: &str) -> Result<(), validator::ValidationError> {
    let has_letter = pwd.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = pwd.chars().any(|c| c.is_ascii_digit());
    if has_letter && has_digit {
        Ok(())
    } else {
        Err(ValidationError::new("password_strength"))
    }
}
```

**验收标准：** `aaaaaaaa`、`12345678` 被拒绝，`abc12345` 通过。

---

### 4. 全局请求体大小限制缺失

**文件：** `src/server/mod.rs`

**问题：** 仅 `/media/upload` 有 `RequestBodyLimitLayer`，其他 POST 端点无限制。恶意请求可发送超大 JSON body。

**修复方案：** 在 `api_v1` 路由组外层添加默认限制，upload 路由使用更大的限制。

```rust
let api_v1 = axum::Router::new()
    // ... 所有路由 ...
    .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024)) // 全局 2MB
    .layer(from_fn(global_rate_limit))
    .layer(Extension(RateLimiter::new(...)));
```

upload 路由单独覆盖为 5MB：

```rust
.route("/media/upload", http_post(media::upload)
    .layer(RequestBodyLimitLayer::new(5 * 1024 * 1024)))
```

**验收标准：** 发送 >2MB 的 JSON 到任意 POST 端点返回 413。

---

## P1 — 高优先级（影响功能/规范符合度）

### 5. CSRF 防护

**问题：** 无 CSRF token 生成和校验。规范 `guide.md` 6.3 要求实现。

**修复方案：** 使用 `axum-csrf` 或手动实现 Double Submit Cookie 模式。

由于 API 使用 Bearer Token（非 Cookie），CSRF 风险较低。若前端改为 Cookie 认证则必须实现。

**优先级评估：** 当前 Bearer Token 方案下可降为 P2。若未来改用 Cookie 认证则必须立即实现。

---

### 6. 代码语法高亮

**问题：** Markdown 渲染（`comrak`）未配置语法高亮。规范 `guide.md` 5.1 要求使用 `syntect`。

**修复方案：** 添加 `syntect` 依赖，在 `render_markdown` 中集成。

```rust
// src/utils/markdown.rs
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

pub fn render_markdown(input: &str) -> String {
    // comrak 渲染 markdown → HTML
    // 对 <code> 块使用 syntect 高亮
}
```

**注意：** `syntect` 增加约 10MB 编译体积和启动时语法集加载时间。可考虑在构建时预生成 HTML。

---

### 7. SQL 编译时校验

**问题：** 所有 SQL 查询使用 `sqlx::query()` / `sqlx::query_as()`（运行时检查），未使用 `sqlx::query!` / `sqlx::query_as!`（编译时校验）。

**修复方案：** 启用 `sqlx` 的 `offline` feature，运行 `cargo sqlx prepare` 生成查询元数据，逐步迁移为宏形式。

```toml
# Cargo.toml
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "offline"] }
```

**迁移优先级：** 从 `models/` 层开始（SQL 最集中），`services/` 层次之。

**注意：** 这是一项大规模重构，建议分批进行，每批一个 model 文件。

---

### 8. 评论列表分页

**文件：** `src/models/comment.rs` — `find_approved_by_post()`

**问题：** 一次查询文章所有已审核评论，无分页。热门文章评论数可达数千条。

**修复方案：** 增加分页参数，前端支持懒加载/无限滚动。

```rust
pub async fn find_approved_by_post_paginated(
    pool: &SqlitePool,
    post_id: &str,
    page: i64,
    page_size: i64,
) -> AppResult<(Vec<Comment>, i64)> {
    let offset = (page - 1) * page_size;
    // 查询 + COUNT
}
```

---

## P2 — 中优先级（影响可扩展性/代码质量）

### 9. 限流器内存无限增长

**文件：** `src/middleware/rate_limit.rs`

**问题：** 清理仅在 `check()` 调用时触发。低流量时过期条目长期驻留内存。

**修复方案：** 添加后台定时清理任务。

```rust
// server 启动时 spawn 一个清理任务
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        limiter.cleanup_expired().await;
    }
});
```

---

### 10. `#[non_exhaustive]` 缺失

**问题：** 公开 API 类型（`AppError`、`ApiResponse`、请求/响应结构体）缺少 `#[non_exhaustive]`，无法安全地添加字段/变体而不破坏向后兼容。

**修复方案：** 为所有公开类型添加 `#[non_exhaustive]`。

```rust
#[non_exhaustive]
pub enum AppError {
    BadRequest(String),
    Unauthorized,
    Forbidden,
    NotFound(String),
    Conflict(String),
    Internal(String),
}
```

---

### 11. sync_tags 批量插入

**文件：** `src/services/post.rs` — `sync_tags()`

**问题：** 逐条 INSERT + DELETE，N 个标签需要 N 次数据库操作。

**修复方案：** 使用事务 + 批量操作。

```rust
pub async fn sync_tags(pool: &SqlitePool, post_id: &str, tag_ids: &[String]) -> AppResult<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM posts_tags WHERE post_id = ?")
        .bind(post_id)
        .execute(&mut *tx)
        .await?;
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO posts_tags (post_id, tag_id) VALUES (?, ?)")
            .bind(post_id)
            .bind(tag_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
```

---

### 12. 限流器水平扩展

**问题：** 内存存储，多实例部署时限流状态不共享。

**修复方案：** 抽象 `RateLimitStore` trait，提供 `MemoryStore`（当前）和 `RedisStore`（未来）实现。

```rust
#[async_trait]
pub trait RateLimitStore: Send + Sync {
    async fn increment(&self, key: &str, window_secs: u64) -> u64;
    async fn get(&self, key: &str) -> u64;
}
```

当部署为多实例时，切换为 Redis 后端即可。

---

### 13. update_role 使用类型化请求

**文件：** `src/handlers/user.rs` — `update_role()`

**问题：** 手动解析 `serde_json::Value`，绕过了 `validator` 校验。

**修复方案：** 定义带验证的请求结构体。

```rust
#[derive(Debug, Deserialize, Validate)]
pub struct UpdateRoleRequest {
    #[validate(length(min = 1))]
    pub role: String,
}

pub async fn update_role(
    _admin: AdminUser,
    State(state): State<crate::AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRoleRequest>,
) -> AppResult<ApiResponse<UserResponse>> {
    if !["reader", "author", "admin"].contains(&req.role.as_str()) {
        return Err(AppError::BadRequest("invalid role".into()));
    }
    // ...
}
```

---

## P3 — 低优先级（锦上添花）

### 14. 单元测试 & 测试基础设施

**当前状态：** 60 个集成测试通过，但无单元测试、无 mock、无快照测试、无基准测试。`rstest` 已引入但未使用。

**改进项：**
- 为 `services/` 层添加单元测试（使用 `mockall` 隔离数据库）
- 为 `utils/markdown.rs` 添加快照测试（`insta`）
- 为热点路径（文章列表、JWT 验证）添加基准测试（`criterion`）
- CI 中使用 `cargo nextest` 替代 `cargo test`

### 15. 错误日志级别调整

**问题：** `BadRequest`、`NotFound` 等客户端错误记录为 `ERROR` 级别。

**修复：** 4xx 错误用 `tracing::warn!`，5xx 才用 `tracing::error!`。

### 16. view_count 并发安全

**文件：** `src/services/post.rs` — `get_post()`

**问题：** `find_published_by_slug` 和 `increment_view_count` 不是原子操作。高并发下可能出现计数偏差。

**修复方案：** 使用 SQL 单条语句 `UPDATE posts SET view_count = view_count + 1 WHERE slug = ? RETURNING *`，或在查询后用原子计数器。

### 17. 静态文件目录

**问题：** 规范中的 `static/` 目录未创建。

**修复：** 创建 `static/` 目录，用于 favicon、robots.txt 等静态资源，通过 `ServeDir` 挂载。

---

## 实施建议

按以下顺序分批推进：

1. **第一批（1-2 天）：** P0 全部 — #1 限流器、#2 N+1、#3 密码验证、#4 请求体限制
2. **第二批（2-3 天）：** P1 — #8 评论分页、#13 类型化请求、#5 CSRF 评估
3. **第三批（持续）：** P2/P3 — #6 语法高亮、#7 SQL 编译时校验、#9-12 扩展性改进、#14 测试
