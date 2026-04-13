# 博客系统 — 详细实现计划

> 验收方式：每一步完成后，执行 `cargo fmt --check && cargo clippy -- -D warnings && cargo build && cargo test` 全部通过，即可标记 `[x]`。
>
> 文档约定：`[ ]` 未完成，`[x]` 已完成。

---

## Phase 1：项目骨架 ✅

### 1.1 配置 Cargo.toml 依赖

- [x] 在 `[dependencies]` 中添加以下依赖：
  - `axum`、`tokio`（features = full）、`tower`、`tower-http`（features = cors, trace, limit）
  - `sqlx`（features = runtime-tokio, sqlite）、`serde`、`serde_json`
  - `uuid`（features = v7, serde）、`chrono`（features = serde）
  - `argon2`、`jsonwebtoken`
  - `thiserror`、`anyhow`
  - `tracing`、`tracing-subscriber`（features = env-filter）
  - `dotenvy`、`validator`（features = derive）
  - `comrak`、`slug`、`ammonia`
  - `image`、`rss`
- [x] 在 `src/main.rs` 顶部添加 `#![deny(unsafe_code)]`
- [x] `cargo build` 编译通过

验收：`cargo build` 无报错

### 1.2 创建目录结构与模块声明

- [x] 创建以下目录和文件：
  ```
  src/
  ├── main.rs
  ├── lib.rs
  ├── config/
  │   └── mod.rs
  ├── db/
  │   └── mod.rs
  ├── models/
  │   └── mod.rs
  ├── handlers/
  │   └── mod.rs
  ├── services/
  │   └── mod.rs
  ├── middleware/
  │   └── mod.rs
  ├── errors/
  │   └── mod.rs
  └── utils/
      └── mod.rs
  ```
- [x] 每个模块文件包含基本内容（可为空 `pub mod xxx;` 或占位）
- [x] `src/lib.rs` 声明所有子模块：`pub mod config; pub mod db; ...`
- [x] `cargo build` 编译通过

验收：`cargo build` 无报错，目录结构完整

### 1.3 环境变量与配置加载

- [x] 创建 `.env.example`，包含 `APP_HOST`、`APP_PORT`、`APP_ENV`、`DATABASE_URL`、`JWT_SECRET`、`JWT_ACCESS_EXPIRES`、`JWT_REFRESH_EXPIRES`、`UPLOAD_DIR`、`MAX_UPLOAD_SIZE`、`RUST_LOG`
- [x] 创建 `.env`（从 `.env.example` 复制，填入开发用默认值）
- [x] 将 `.env` 加入 `.gitignore`
- [x] 在 `src/config/mod.rs` 中定义 `AppConfig` 结构体，从环境变量加载：
  ```rust
  pub struct AppConfig {
      pub host: String,
      pub port: u16,
      pub env: String,
      pub database_url: String,
      pub jwt_secret: String,
      pub jwt_access_expires: u64,
      pub jwt_refresh_expires: u64,
      pub upload_dir: String,
      pub max_upload_size: usize,
  }
  ```
- [x] 实现 `AppConfig::from_env()` 方法，使用 `dotenvy::dotenv()` 加载 `.env`
- [x] `cargo build` 编译通过

验收：单元测试验证 `AppConfig::from_env()` 正确读取 `.env` 中的值

### 1.4 SQLite 连接池初始化

- [x] 在 `src/db/mod.rs` 中实现 `init_pool(database_url: &str) -> SqlitePool`
- [x] 使用 `SqlitePoolOptions` 配置连接池（max_connections = 5）
- [x] 启用 SQLite `PRAGMA journal_mode = WAL`、`PRAGMA foreign_keys = ON`
- [x] 在 `src/main.rs` 中调用 `init_pool` 并将 `SqlitePool` 放入 axum State
- [x] `cargo build` 编译通过

验收：运行程序不报数据库连接错误；可在测试中执行简单查询 `SELECT 1`

### 1.5 统一错误处理 AppError

- [x] 在 `src/errors/mod.rs` 中定义 `AppError` 枚举：
  ```rust
  #[derive(Debug, thiserror::Error)]
  pub enum AppError {
      #[error("bad request: {0}")]
      BadRequest(String),       // 400
      #[error("unauthorized")]
      Unauthorized,             // 401
      #[error("forbidden")]
      Forbidden,                // 403
      #[error("not found: {0}")]
      NotFound(String),         // 404
      #[error("conflict: {0}")]
      Conflict(String),         // 409
      #[error("internal server error")]
      Internal(#[from] anyhow::Error), // 500
  }
  ```
- [x] 为 `AppError` 实现 `IntoResponse`，返回 JSON 格式 `{ "code": xxx, "message": "xxx", "data": null }`
- [x] 定义 `AppResult<T> = Result<T, AppError>` 类型别名
- [x] 为 sqlx::Error 实现到 AppError 的转换（唯一约束冲突 → Conflict）
- [x] `cargo build` 编译通过

验收：Handler 返回 `AppError::NotFound("post".into())` 时 HTTP 响应 404 + 正确 JSON body

### 1.6 统一响应格式 ApiResponse

- [x] 在 `src/errors/mod.rs`（或独立 `src/utils/response.rs`）中定义：
  ```rust
  #[derive(Serialize)]
  pub struct ApiResponse<T: Serialize> {
      pub code: i32,
      pub message: String,
      pub data: Option<T>,
  }
  ```
- [x] 实现 `ApiResponse::success(data)` 和 `ApiResponse::error(code, message)` 构造方法
- [x] 定义分页结构 `PaginatedData<T>`：
  ```rust
  pub struct PaginatedData<T> {
      pub items: Vec<T>,
      pub total: i64,
      pub page: i64,
      pub page_size: i64,
  }
  ```
- [x] `cargo build` 编译通过

验收：`ApiResponse::success(Some("hello"))` 序列化为 `{"code":0,"message":"success","data":"hello"}`

### 1.7 tracing 日志初始化

- [x] 在 `src/main.rs` 中初始化 tracing：
  ```rust
  tracing_subscriber::fmt()
      .with_env_filter(EnvFilter::from_default_env())
      .init();
  ```
- [x] 在关键位置添加 `tracing::info!` 日志（服务器启动、数据库连接等）
- [x] `cargo build` 编译通过

验收：设置 `RUST_LOG=hello_axum=debug` 运行程序，终端可见结构化日志输出

### 1.8 axum 路由与服务器启动 + 健康检查

- [x] 在 `src/handlers/mod.rs` 实现健康检查 handler：
  ```
  GET /api/v1/health → { "code": 0, "message": "success", "data": { "status": "ok" } }
  ```
- [x] 在 `src/main.rs` 中组装 axum Router：
  - 挂载 `/api/v1/health` 路由
  - 使用 `tower::ServiceBuilder` 添加 `TraceLayer`（请求日志）
  - 使用 `axum::extract::State` 共享 `SqlitePool` 和 `AppConfig`
- [x] 使用 `tokio::net::TcpListener` 绑定地址，`axum::serve` 启动
- [x] 添加 graceful shutdown（监听 ctrl+c）
- [x] `cargo build` 编译通过

验收：`cargo run` 启动后 `curl http://localhost:3000/api/v1/health` 返回 200 + JSON

### 1.9 初始数据库迁移

- [x] 安装 `cargo install sqlx-cli --no-default-features --features sqlite`
- [x] 创建 `migrations/` 目录
- [x] 编写 `migrations/001_init.sql`，包含所有表：
  - `users`
  - `categories`
  - `tags`
  - `posts`
  - `posts_tags`
  - `comments`
  - `media`
  - `refresh_tokens`
- [x] 每张表的字段类型、约束、外键、索引与 `docs/guide.md` 第 4 节一致
- [x] 为 `posts` 添加索引：`slug`、`status`、`author_id`、`category_id`、`created_at`
- [x] 为 `comments` 添加索引：`post_id`、`status`
- [x] 为 `users` 添加索引：`email`、`username`
- [x] 运行迁移成功执行
- [x] 验证表结构：`sqlite3 data/blog.db ".tables"` 和 `.schema users`

验收：`sqlite3 data/blog.db ".tables"` 输出 8 张表，schema 与文档一致

### 1.10 Phase 1 最终验证

- [x] `cargo fmt --check` 通过（无格式问题）
- [x] `cargo clippy -- -D warnings` 通过（零警告）
- [x] `cargo build` 通过
- [x] `cargo test` 通过
- [x] `cargo run` 启动后健康检查返回 200
- [x] 数据库迁移正常

---

## Phase 2：用户认证 ✅

### 2.1 用户数据模型

- [x] 在 `src/models/user.rs` 中定义数据库模型
- [x] 实现查询函数：`find_by_email`、`find_by_id`、`create`、`update_profile`
- [x] 在 `src/models/mod.rs` 中声明 `pub mod user;`
- [x] `cargo build` 通过

验收：模型和查询函数编译通过

### 2.2 用户注册

- [x] 定义请求结构体 `RegisterRequest`（email, username, password）并使用 `validator` derive 校验
- [x] 定义响应结构体 `UserResponse`（不含 password_hash）
- [x] 在 `src/services/auth.rs` 实现 `register`（校验唯一性、argon2 哈希、UUID v7）
- [x] 在 `src/handlers/auth.rs` 实现 `POST /api/v1/auth/register` handler
- [x] 在 Router 中注册路由
- [x] `cargo build` 通过

验收：
- 注册返回 200 + 用户信息（无密码）
- 重复注册返回 409

### 2.3 用户登录与 JWT 签发

- [x] 定义 `LoginRequest` 和 `LoginResponse`
- [x] 实现 JWT 工具函数（generate_access_token, verify_token）
- [x] 在 `src/services/auth.rs` 实现 `login`
- [x] 在 `src/handlers/auth.rs` 实现 `POST /api/v1/auth/login` handler
- [x] `cargo build` 通过

验收：
- 正确邮箱+密码登录返回 200 + tokens
- 错误密码返回 401
- 数据库中 `refresh_tokens` 表有记录

### 2.4 JWT 认证中间件

- [x] 在 `src/middleware/auth.rs` 实现 `AuthUser`、`AdminUser`、`AuthorUser` 提取器
- [x] `cargo build` 通过

验收：
- 带 Bearer Token 请求受保护接口 → 200
- 不带 Token → 401

### 2.5 Token 刷新

- [x] 定义 `RefreshRequest`
- [x] 实现 `refresh`（token rotation）
- [x] 实现 `POST /api/v1/auth/refresh` handler
- [x] `cargo build` 通过

验收：
- 有效 refresh_token 刷新返回新 token 对
- 旧 refresh_token 不可重复使用

### 2.6 登出

- [x] 实现 `POST /api/v1/auth/logout`（需认证）
- [x] `cargo build` 通过

验收：登出后 refresh_token 不可用

### 2.7 用户信息接口

- [x] 定义 `UpdateUserRequest` 和 `UpdatePasswordRequest` 并校验
- [x] 实现 `GET /api/v1/users/me`（需认证）
- [x] 实现 `PUT /api/v1/users/me`（需认证）
- [x] 实现 `PUT /api/v1/users/me/password`（需认证）
- [x] 实现 `GET /api/v1/users/{id}`（公开）
- [x] 实现 `GET /api/v1/users`（需 Admin）
- [x] `cargo build` 通过

验收：
- 各接口返回正确状态码和数据
- 修改密码后旧密码登录失败，新密码登录成功
- 非 Admin 访问用户列表返回 403

### 2.8 Phase 2 最终验证

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy -- -D warnings` 通过
- [x] `cargo test` 通过
- [x] 手动测试完整认证流程：注册 → 登录 → 访问受保护接口 → 刷新 Token → 登出
- [x] 数据库中数据正确（用户密码已哈希、refresh_token 可查）

---

## Phase 3：文章核心 ✅

### 3.1 分类 CRUD

- [x] 在 `src/models/category.rs` 定义 `Category` 模型和请求结构体
- [x] 实现 model 层：create, find_all, find_by_id, update, delete
- [x] 实现 `src/handlers/category.rs`：GET/POST/PUT/DELETE
- [x] 注册路由
- [x] `cargo build` 通过

验收：分类 CRUD 正常，Reader 角色无法创建（403）

### 3.2 标签 CRUD

- [x] 在 `src/models/tag.rs` 定义 `Tag` 模型和 `CreateTagRequest`
- [x] 实现 model 层：create, find_all, delete
- [x] 实现 `src/handlers/tag.rs`：GET/POST/DELETE
- [x] 注册路由
- [x] `cargo build` 通过

验收：标签 CRUD 正常，重复 name 返回 409

### 3.3 文章 CRUD

- [x] 在 `src/models/post.rs` 定义完整模型和请求/响应结构体
- [x] 实现 service 层：create, update, delete, get, list（含 slug 自动生成、excerpt 提取）
- [x] 实现 `src/handlers/post.rs`：完整 CRUD + 权限校验
- [x] 注册路由
- [x] `cargo build` 通过

验收：文章 CRUD 正常，权限控制正确，slug 自动生成

### 3.4 文章-标签关联

- [x] 创建/更新时处理 `tag_ids`（sync_tags）
- [x] 查询详情时返回标签列表
- [x] `cargo build` 通过

验收：文章详情返回对应标签

### 3.5 Markdown 渲染

- [x] 在 `src/utils/markdown.rs` 实现 `render_markdown`（comrak + ammonia）
- [x] `PostResponse` 包含 `html_content` 字段
- [x] `cargo build` 通过

验收：Markdown 正确渲染为 HTML（h1 + strong 等）

### 3.6 分页工具

- [x] `PaginationParams` 使用 axum Query 提取
- [x] page 默认 1，page_size 默认 20，最大 100
- [x] 所有列表接口统一使用
- [x] `cargo build` 通过

验收：分页参数正确

### 3.7 Phase 3 最终验证

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy -- -D warnings` 通过
- [x] `cargo test` 通过
- [x] 完整流程：创建分类 → 创建标签 → 创建文章 → 列表 → 详情 → 更新 → 删除

---

## Phase 4：评论系统 ✅

### 4.1 评论数据模型与 CRUD

- [x] 在 `src/models/comment.rs` 定义 `Comment` 模型、`CreateCommentRequest`
- [x] 实现 `src/services/comment.rs`：create, find_approved_by_post, delete
- [x] 实现 `src/handlers/comment.rs`：游客和登录用户分开的评论接口
- [x] 注册路由
- [x] `cargo build` 通过

验收：游客评论关联 nickname，登录用户关联 author_id

### 4.2 嵌套评论

- [x] `CreateCommentRequest` 支持 `parent_id` 字段
- [x] 验证 `parent_id` 对应评论存在且属于同一文章
- [x] 限制嵌套深度不超过 3 层
- [x] 查询时构建树形结构返回（`build_tree`）
- [x] `cargo build` 通过

验收：嵌套回复正常显示在 `replies` 字段中

### 4.3 评论审核

- [x] 评论默认 status = "pending"
- [x] 实现 `PUT /api/v1/comments/{id}/status`（需 Admin）
- [x] 公开评论列表仅返回 status = "approved" 的评论
- [x] `cargo build` 通过

验收：pending 不可见 → approve 后可见

### 4.4 Phase 4 最终验证

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy -- -D warnings` 通过
- [x] `cargo test` 通过

---

## Phase 5：媒体与辅助功能 ✅

### 5.1 图片上传

- [x] 在 `src/models/media.rs` 定义 `Media` 模型
- [x] 实现 `src/services/media.rs`：
  - `save_file` — 验证文件类型和大小，保存到 UPLOAD_DIR，生成唯一文件名，写数据库
  - `find_all` — 分页列表
  - `delete_media` — 删除文件和数据库记录
- [x] 实现 `src/handlers/media.rs`：
  - `POST /api/v1/media/upload`（需认证）— multipart 上传
  - `GET /api/v1/media`（需认证）
  - `DELETE /api/v1/media/{id}`（需认证，仅本人或 Admin）
- [x] 挂载 `tower_http::services::ServeDir` 提供 `/uploads/` 静态文件访问
- [x] 注册路由
- [x] `cargo build` 通过

验收：
- 上传图片返回 URL，文件存在于 uploads 目录
- 超过 5MB 返回 400
- 非图片类型返回 400

### 5.2 RSS 订阅

- [x] 在 `src/handlers/rss.rs` 实现 `GET /feed.xml`：
  - 查询最新 20 篇 published 文章
  - 使用 `rss` crate 生成 RSS 2.0 XML
  - 返回 `Content-Type: application/xml`
- [x] 注册路由
- [x] `cargo build` 通过

验收：`curl /feed.xml` 返回合法 RSS 2.0 XML，包含最新文章

### 5.3 全文搜索（基础版）

- [x] `GET /api/v1/posts` 增加 `q` 查询参数
- [x] 使用 SQLite `LIKE '%keyword%'` 搜索 title 和 content
- [x] `cargo build` 通过

验收：`GET /api/v1/posts?q=rust` 返回标题或正文包含 "rust" 的文章

### 5.4 浏览量统计

- [x] 在 `src/services/post.rs` 的 `find_by_slug` 中增加浏览量 +1
- [x] 基于简化方案：每次访问 detail 接口 view_count += 1（不做去重）
- [x] `PostResponse` 包含 `view_count` 字段
- [x] `cargo build` 通过

验收：多次访问文章详情，view_count 递增

### 5.5 Phase 5 最终验证

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy -- -D warnings` 通过
- [x] `cargo test` 通过

---

## Phase 6：安全与优化 ✅

### 6.1 Rate Limiting 中间件

- [x] 在 `src/middleware/rate_limit.rs` 实现基于 IP 的限流：
  - 自定义 `RateLimiter`（tokio::sync::Mutex + HashMap）作为 axum Extension + middleware
  - 登录接口：10 次/分钟/IP
  - 注册接口：5 次/小时/IP
  - 评论接口：3 次/分钟/IP
  - 通用：60 次/分钟/IP
- [x] 挂载到对应路由组
- [x] `cargo build` 通过

验收：超过限制后返回 429 Too Many Requests

### 6.2 CORS 配置

- [x] 使用 `tower_http::cors::CorsLayer` 配置：
  - 开发环境允许 Any origin（生产应改为白名单）
  - 允许的方法：Any
  - 允许的头部：Any
- [x] `cargo build` 通过

验收：跨域请求返回正确的 CORS 头

### 6.3 输入校验完善

- [x] 所有 POST/PUT 接口的请求体使用 `validator` derive 校验（含 `UpdateCommentStatusRequest`）
- [x] Handler 中统一调用 `validate(&req)?`，校验失败返回 400 + 具体错误信息
- [x] `cargo build` 通过

验收：发送空 body 或非法字段返回 400 + 错误描述

### 6.4 数据库索引与查询优化

- [x] 在 `migrations/002_add_indexes.sql` 中添加复合索引：
  - `idx_posts_status_created` — 覆盖文章列表查询
  - `idx_posts_status_category` — 覆盖按分类筛选
  - `idx_posts_status_author` — 覆盖按作者筛选
  - `idx_comments_post_status` — 覆盖评论列表查询
  - `idx_media_user_created` — 覆盖媒体文件列表查询
- [x] 所有分页查询使用单独 count 查询
- [x] `cargo build` 通过

验收：索引覆盖常用查询路径

### 6.5 Phase 6 最终验证

- [x] `cargo fmt --check` 通过
- [x] `cargo clippy -- -D warnings` 通过
- [x] `cargo test` 通过
- [x] 限流正常工作
- [x] CORS 正常工作

---

## Phase 7：部署 ✅

### 7.1 Dockerfile

- [x] 编写多阶段 Dockerfile：
  - Stage 1：`rust:1.85` 编译 release
  - Stage 2：`debian:bookworm-slim` 运行
  - 复制二进制 + migrations 目录
  - 暴露 3000 端口
- [x] `docker build -t hello-axum .` 成功
- [x] `docker run -p 3000:3000 hello-axum` 启动正常

验收：`curl http://localhost:3000/api/v1/health` 返回 200

### 7.2 docker-compose

- [x] 编写 `docker-compose.yml`：
  - app 服务（构建镜像、挂载 data + uploads）
- [x] `docker compose up -d` 正常启动

验收：`docker compose up -d` 后健康检查通过

### 7.3 Nginx 配置

- [x] 编写 Nginx 反向代理配置（`deploy/nginx.conf`）
- [x] 配置 gzip、静态文件缓存、client_max_body_size
- [x] 文档中包含配置说明

验收：配置文件语法正确（`nginx -t`）

### 7.4 CI/CD

- [x] 编写 `.github/workflows/ci.yml`：
  - 触发条件：push / PR to main
  - 步骤：`cargo fmt --check` → `cargo clippy -- -D warnings` → `cargo test`
  - 含 cargo cache 加速

验收：CI workflow 语法正确，本地 act 可执行（或推送到 GitHub 后验证）

### 7.5 Phase 7 最终验证

- [x] Docker 构建和运行正常
- [x] docker-compose 启动正常
- [x] 所有 Phase 1-6 的验收项仍然通过
- [x] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` 全通过
