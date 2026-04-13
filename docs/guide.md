# 博客系统 — 产品与技术指导手册

## 1. 项目概述

基于 Rust + Axum 构建的功能完整、高性能、安全的博客系统。面向个人开发者和小型团队，提供内容创作、发布、管理和互动的全流程支持。

### 1.1 项目目标

- 高性能：利用 Rust 零成本抽象和异步 I/O，单机支撑高并发访问
- 安全性：密码加盐哈希、JWT 鉴权、XSS/CSRF 防护、SQL 注入防护
- 易部署：Docker 容器化，单一二进制输出，资源占用低
- 可扩展：模块化架构，便于后续功能迭代

### 1.2 目标用户

| 角色 | 描述 |
|---|---|
| 管理员 | 系统配置、用户管理、内容审核 |
| 作者 | 创建和发布文章、管理个人内容 |
| 读者 | 浏览文章、评论互动、订阅 RSS |

---

## 2. 功能规格

### 2.1 用户系统

#### 2.1.1 注册与登录

- 邮箱 + 密码注册，邮箱唯一性校验
- 密码强度校验（最少 8 位，包含字母和数字）
- 密码使用 Argon2 加盐哈希存储，禁止明文存储
- 登录成功返回 JWT Access Token（15 分钟过期）+ Refresh Token（7 天过期）
- Refresh Token 用于无感刷新 Access Token

#### 2.1.2 用户资料

- 用户名、头像、个人简介、个人网站链接
- 头像上传支持 JPEG/PNG，最大 2MB，存储为 WebP 格式

#### 2.1.3 角色与权限

```
Admin  → 用户管理、内容审核、系统配置、所有 Author 权限
Author → 文章 CRUD、评论管理、标签管理
Reader → 浏览文章、发表评论、管理自己的评论
```

### 2.2 文章系统

#### 2.2.1 文章模型

| 字段 | 类型 | 说明 |
|---|---|---|
| id | UUID | 主键 |
| title | String | 标题，最长 200 字符 |
| slug | String | URL 友好标识，自动从标题生成，唯一 |
| content | Text | Markdown 格式正文 |
| excerpt | String | 摘要，最长 500 字符，自动从正文提取 |
| cover_image | String | 封面图片 URL |
| status | Enum | draft / published |
| author_id | UUID | 外键关联用户 |
| category_id | UUID | 外键关联分类 |
| created_at | Timestamp | 创建时间 |
| updated_at | Timestamp | 更新时间 |
| published_at | Timestamp | 发布时间 |

#### 2.2.2 文章功能

- Markdown 编辑，服务端渲染为 HTML
- 草稿自动保存
- 文章状态流转：`draft → published → archived`
- 发布时自动生成 slug（如 `my-first-post`），支持自定义
- 文章置顶、排序
- 浏览量统计（去重，基于 IP + User-Agent）

#### 2.2.3 分类与标签

- 分类：树形结构（支持一级子分类），每篇文章归属一个分类
- 标签：平铺结构，多对多关系，一篇文章可有多个标签
- 标签云展示

### 2.3 评论系统

- 评论关联文章，支持嵌套回复（最多 3 层）
- 评论者填写昵称 + 邮箱（未登录用户）或使用已登录身份
- 管理员可审核/删除评论
- 支持按时间正序/倒序排列
- 评论分页加载

### 2.4 搜索

- 按关键词搜索文章标题和正文
- 支持按分类、标签、作者筛选
- 搜索结果高亮关键词
- 后续可接入 Meilisearch 提升搜索体验

### 2.5 媒体管理

- 图片上传（JPEG、PNG、GIF、WebP）
- 单文件最大 5MB
- 支持本地存储和 S3 兼容对象存储
- 图片自动压缩和缩略图生成
- 上传后返回可访问的 URL

### 2.6 RSS 订阅

- 提供 `/feed.xml` 全局 RSS 订阅
- 包含最新 20 篇已发布文章
- 符合 RSS 2.0 规范

### 2.7 系统管理

- 站点基础配置（标题、描述、每页文章数）
- 友情链接管理
- 导航菜单配置

---

## 3. API 设计

### 3.1 通用约定

- 基础路径：`/api/v1`
- 请求/响应格式：JSON
- 认证方式：`Authorization: Bearer <token>`
- 分页参数：`?page=1&page_size=20`
- 排序参数：`?sort=created_at&order=desc`
- 时间格式：ISO 8601（`2026-04-10T12:00:00Z`）

### 3.2 统一响应结构

```json
{
  "code": 0,
  "message": "success",
  "data": { }
}
```

分页响应：

```json
{
  "code": 0,
  "message": "success",
  "data": {
    "items": [],
    "total": 100,
    "page": 1,
    "page_size": 20
  }
}
```

错误响应：

```json
{
  "code": 40100,
  "message": "invalid credentials",
  "data": null
}
```

### 3.3 错误码规范

| 范围 | 说明 |
|---|---|
| 0 | 成功 |
| 40000-40099 | 通用参数错误 |
| 40100-40199 | 认证与授权错误 |
| 40400-40499 | 资源不存在 |
| 40900-40999 | 资源冲突 |
| 50000-50099 | 服务端内部错误 |

### 3.4 接口列表

#### 认证

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| POST | /api/v1/auth/register | 用户注册 | 否 |
| POST | /api/v1/auth/login | 用户登录 | 否 |
| POST | /api/v1/auth/refresh | 刷新 Token | 否 |
| POST | /api/v1/auth/logout | 登出 | 是 |

#### 用户

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /api/v1/users/me | 获取当前用户信息 | 是 |
| PUT | /api/v1/users/me | 更新当前用户信息 | 是 |
| PUT | /api/v1/users/me/password | 修改密码 | 是 |
| GET | /api/v1/users/:id | 获取用户公开信息 | 否 |
| GET | /api/v1/users | 获取用户列表（管理员） | 是 |

#### 文章

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /api/v1/posts | 文章列表 | 否 |
| GET | /api/v1/posts/:slug | 文章详情 | 否 |
| POST | /api/v1/posts | 创建文章 | 是 |
| PUT | /api/v1/posts/:slug | 更新文章 | 是 |
| DELETE | /api/v1/posts/:slug | 删除文章 | 是 |

#### 分类

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /api/v1/categories | 分类列表 | 否 |
| POST | /api/v1/categories | 创建分类 | 是 |
| PUT | /api/v1/categories/:id | 更新分类 | 是 |
| DELETE | /api/v1/categories/:id | 删除分类 | 是 |

#### 标签

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /api/v1/tags | 标签列表 | 否 |
| POST | /api/v1/tags | 创建标签 | 是 |
| DELETE | /api/v1/tags/:id | 删除标签 | 是 |

#### 评论

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /api/v1/posts/:slug/comments | 文章评论列表 | 否 |
| POST | /api/v1/posts/:slug/comments | 发表评论 | 可选 |
| DELETE | /api/v1/comments/:id | 删除评论 | 是 |

#### 媒体

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| POST | /api/v1/media/upload | 上传文件 | 是 |
| GET | /api/v1/media | 文件列表 | 是 |
| DELETE | /api/v1/media/:id | 删除文件 | 是 |

#### RSS

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | /feed.xml | RSS 订阅 | 否 |

---

## 4. 数据库设计

### 4.1 ER 关系概览

```
users ──1:N── posts ──1:N── comments
  │            │
  │            ├── N:1 ── categories
  │            │
  │            └── M:N ── posts_tags ── M:N ── tags
  │
  └──1:N── media
```

### 4.2 表结构

#### users

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| email | VARCHAR(255) | UNIQUE, NOT NULL | 邮箱 |
| username | VARCHAR(50) | UNIQUE, NOT NULL | 用户名 |
| password_hash | VARCHAR(255) | NOT NULL | Argon2 哈希 |
| role | VARCHAR(20) | NOT NULL, DEFAULT 'reader' | 角色 |
| avatar | VARCHAR(500) | | 头像 URL |
| bio | TEXT | | 个人简介 |
| website | VARCHAR(500) | | 个人网站 |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |
| updated_at | TEXT | NOT NULL | 更新时间（ISO 8601） |

#### posts

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| title | VARCHAR(200) | NOT NULL | 标题 |
| slug | VARCHAR(250) | UNIQUE, NOT NULL | URL 标识 |
| content | TEXT | NOT NULL | Markdown 正文 |
| excerpt | VARCHAR(500) | | 摘要 |
| cover_image | VARCHAR(500) | | 封面图 URL |
| status | VARCHAR(20) | NOT NULL, DEFAULT 'draft' | draft/published/archived |
| author_id | UUID | FK → users.id | 作者 |
| category_id | UUID | FK → categories.id | 分类 |
| view_count | INTEGER | NOT NULL, DEFAULT 0 | 浏览量 |
| is_pinned | BOOLEAN | NOT NULL, DEFAULT 0 | 是否置顶 |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |
| updated_at | TEXT | NOT NULL | 更新时间（ISO 8601） |
| published_at | TEXT | | 发布时间（ISO 8601） |

#### categories

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| name | VARCHAR(100) | UNIQUE, NOT NULL | 分类名 |
| slug | VARCHAR(120) | UNIQUE, NOT NULL | URL 标识 |
| description | VARCHAR(500) | | 描述 |
| parent_id | UUID | FK → categories.id | 父分类 |
| sort_order | INTEGER | NOT NULL, DEFAULT 0 | 排序 |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |

#### tags

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| name | VARCHAR(50) | UNIQUE, NOT NULL | 标签名 |
| slug | VARCHAR(60) | UNIQUE, NOT NULL | URL 标识 |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |

#### posts_tags

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| post_id | UUID | FK → posts.id | 文章 ID |
| tag_id | UUID | FK → tags.id | 标签 ID |

主键：(post_id, tag_id)

#### comments

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| post_id | UUID | FK → posts.id | 所属文章 |
| author_id | UUID | FK → users.id, NULLABLE | 登录用户 |
| nickname | VARCHAR(50) | | 游客昵称 |
| email | VARCHAR(255) | | 游客邮箱 |
| content | TEXT | NOT NULL | 评论内容 |
| parent_id | UUID | FK → comments.id, NULLABLE | 父评论 |
| status | VARCHAR(20) | NOT NULL, DEFAULT 'pending' | pending/approved/spam |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |

#### media

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| user_id | UUID | FK → users.id | 上传者 |
| filename | VARCHAR(255) | NOT NULL | 原始文件名 |
| filepath | VARCHAR(500) | NOT NULL | 存储路径 |
| mimetype | VARCHAR(100) | NOT NULL | MIME 类型 |
| size | INTEGER | NOT NULL | 文件大小（字节） |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |

#### refresh_tokens

| 列名 | 类型 | 约束 | 说明 |
|---|---|---|---|
| id | UUID | PK | 主键 |
| user_id | UUID | FK → users.id | 所属用户 |
| token | VARCHAR(255) | UNIQUE, NOT NULL | Refresh Token |
| expires_at | TEXT | NOT NULL | 过期时间（ISO 8601） |
| created_at | TEXT | NOT NULL | 创建时间（ISO 8601） |

---

## 5. 技术架构

### 5.1 技术栈

#### 核心框架

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 语言 | Rust | Edition 2024 | 核心开发语言 |
| Web 框架 | axum | 0.8.x | HTTP 路由、中间件、提取器 |
| 异步运行时 | tokio | 1.x (full features) | 异步 I/O 运行时 |
| 中间件 | tower / tower-http | 0.5.x / 0.6.x | CORS、限流、请求日志、压缩、静态文件 |

#### 数据层

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 数据库 | SQLite | 3.x | 轻量级嵌入式数据存储（开发阶段） |
| SQL 工具 | sqlx | 0.8.x | 编译期 SQL 检查、异步查询 |
| 迁移 | sqlx-cli | 0.8.x | 数据库版本管理 |

#### 序列化与数据

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 序列化 | serde / serde_json | 1.x | JSON 序列化/反序列化 |
| UUID | uuid | 1.x (v7) | 主键生成，时间排序友好 |
| 时间 | chrono | 0.4.x | 时间处理，序列化配合 serde |

#### 安全与认证

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 密码哈希 | argon2 | 0.5.x | Argon2id 密码安全哈希（OWASP 推荐） |
| JWT | jsonwebtoken | 9.x | Token 签发与验证 |
| HTML 净化 | ammonia | 0.16.x | XSS 防护，清理用户提交的 HTML |
| CSRF | tokio (nonce) | — | CSRF Token 生成与校验 |

#### 业务功能

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| Markdown | comrak | 0.36.x | CommonMark/GFM 兼容 Markdown → HTML 渲染 |
| 代码高亮 | syntect | 5.x | Markdown 内代码块语法高亮 |
| Slug 生成 | slug | 0.1.x | URL 友好标题生成 |
| 校验 | validator | 0.19.x | 声明式请求参数校验（derive 宏） |
| 图片处理 | image | 0.25.x | 图片压缩、缩略图生成、格式转换 |
| RSS | rss | 2.x | RSS 2.0 订阅源生成 |

#### 错误处理与日志

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 应用错误 | anyhow | 1.x | 应用级通用错误传播（Service 层内部） |
| 业务错误 | thiserror | 2.x | 自定义错误类型 derive（定义 AppError） |
| 日志 | tracing | 0.1.x | 结构化日志，异步友好，支持 span |
| 日志格式 | tracing-subscriber | 0.3.x | 日志输出格式化和过滤 |

#### 配置与环境

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 环境变量 | dotenvy | 0.15.x | .env 文件加载 |
| 配置管理 | config | 0.14.x | 多格式配置文件 + 环境变量 + 类型安全 |

#### 测试与质量

| 层级 | 技术 | 版本 | 用途 |
|---|---|---|---|
| 单元测试 | #[test] + rstest | 0.23.x | 参数化测试、fixture 注入 |
| 快照测试 | insta | 1.x | API 响应快照测试，防回归 |
| HTTP 集成测试 | axum::test / reqwest | 0.12.x | Handler 级别集成测试 |
| Mock | mockall | 0.13.x | Service 层 Mock，隔离数据库依赖 |
| 基准测试 | criterion | 0.5.x | 性能基准测试，防性能退化 |

#### 开发工具链（CI 必装）

| 工具 | 用途 |
|---|---|
| cargo fmt | 代码格式化，统一风格 |
| cargo clippy | 代码 Lint，零警告要求 |
| cargo audit | 安全漏洞审计（检查依赖 CVE） |
| cargo deny | 许可证合规 + 安全策略检查 |
| cargo outdated | 依赖版本检查 |
| cargo nextest | 更快的测试运行器（比 cargo test 快 3x+） |
| cargo insta test --review | 快照测试审查流程 |

### 5.2 项目结构

```
hello-axum/
├── Cargo.toml
├── .env                        # 环境变量（不入库）
├── .env.example                # 环境变量示例
├── migrations/                 # 数据库迁移文件
│   └── 001_init.sql
├── docs/                       # 项目文档
│   └── guide.md
├── static/                     # 静态资源
├── uploads/                    # 上传文件目录
└── src/
    ├── main.rs                 # 入口：启动服务器
    ├── config/
    │   ├── mod.rs
    │   └── app.rs              # 应用配置（从环境变量加载）
    ├── db/
    │   ├── mod.rs
    │   └── connection.rs       # 数据库连接池初始化
    ├── models/
    │   ├── mod.rs
    │   ├── user.rs
    │   ├── post.rs
    │   ├── category.rs
    │   ├── tag.rs
    │   ├── comment.rs
    │   └── media.rs
    ├── handlers/
    │   ├── mod.rs
    │   ├── auth.rs             # 注册、登录、刷新、登出
    │   ├── user.rs             # 用户信息
    │   ├── post.rs             # 文章 CRUD
    │   ├── category.rs         # 分类管理
    │   ├── tag.rs              # 标签管理
    │   ├── comment.rs          # 评论
    │   ├── media.rs            # 文件上传
    │   └── rss.rs              # RSS 订阅
    ├── services/
    │   ├── mod.rs
    │   ├── auth.rs             # 认证业务逻辑
    │   ├── post.rs             # 文章业务逻辑
    │   ├── comment.rs          # 评论业务逻辑
    │   └── media.rs            # 媒体业务逻辑
    ├── middleware/
    │   ├── mod.rs
    │   ├── auth.rs             # JWT 认证中间件
    │   └── rate_limit.rs       # 限流中间件
    ├── errors/
    │   ├── mod.rs
    │   └── app_error.rs        # 统一错误类型（实现 IntoResponse）
    └── utils/
        ├── mod.rs
        ├── pagination.rs       # 分页提取器
        └── slug.rs             # Slug 生成工具
```

### 5.3 架构分层

```
┌──────────────────────────────────┐
│           HTTP 请求              │
└──────────────┬───────────────────┘
               ▼
┌──────────────────────────────────┐
│        路由层 (Router)           │  axum 路由定义、中间件挂载
└──────────────┬───────────────────┘
               ▼
┌──────────────────────────────────┐
│       处理层 (Handler)           │  参数提取、校验、调用 Service
└──────────────┬───────────────────┘
               ▼
┌──────────────────────────────────┐
│       业务层 (Service)           │  核心业务逻辑、事务管理
└──────────────┬───────────────────┘
               ▼
┌──────────────────────────────────┐
│       数据层 (Model / DB)        │  SQL 查询、数据映射
└──────────────────────────────────┘
```

- **Handler** 只负责提取请求参数、调用 Service、构造响应，不包含业务逻辑
- **Service** 包含所有业务逻辑，通过参数接收数据库连接，便于测试
- **Model** 定义数据结构和数据库操作，与数据库表一一对应
- **Middleware** 处理横切关注点（认证、日志、限流）

---

## 6. 安全设计

### 6.1 认证与授权

- JWT 签发使用 HS256 或 RS256 算法
- Access Token 短过期（15 分钟），Refresh Token 长过期（7 天）
- Refresh Token 存储在数据库中，支持吊销
- 密码使用 Argon2id 算法哈希，自动加盐

### 6.2 输入防护

- 所有用户输入通过 `validator` 进行服务端校验
- Markdown 渲染时过滤危险的 HTML 标签（XSS 防护）
- 使用 sqlx 参数化查询，杜绝 SQL 注入
- 请求体大小限制（默认 2MB）

### 6.3 传输安全

- 生产环境强制 HTTPS
- Cookie 设置 `HttpOnly`、`Secure`、`SameSite=Strict`
- CORS 白名单配置

### 6.4 限流

- 登录接口：同一 IP 每分钟最多 10 次
- 注册接口：同一 IP 每小时最多 5 次
- 评论接口：同一用户每分钟最多 3 次
- 通用接口：同一 IP 每分钟最多 60 次

---

## 7. 开发阶段规划

### Phase 1：项目骨架（1-2 天）

- [ ] 初始化项目结构，配置 Cargo.toml 依赖
- [ ] 搭建 axum 基础路由和服务器
- [ ] 配置 SQLite 连接池
- [ ] 实现统一错误处理
- [ ] 实现统一响应格式
- [ ] 配置日志和 .env 管理
- [ ] 编写初始数据库迁移

### Phase 2：用户认证（2-3 天）

- [ ] 用户注册（参数校验、密码哈希）
- [ ] 用户登录（JWT 签发）
- [ ] JWT 认证中间件
- [ ] Token 刷新机制
- [ ] 用户信息查询和更新

### Phase 3：文章核心（3-4 天）

- [ ] 文章 CRUD
- [ ] 分类 CRUD
- [ ] 标签 CRUD 及多对多关联
- [ ] Markdown 渲染
- [ ] Slug 自动生成
- [ ] 分页和排序

### Phase 4：评论与互动（2 天）

- [ ] 评论发表和列表
- [ ] 嵌套评论
- [ ] 评论审核

### Phase 5：媒体与辅助功能（2-3 天）

- [ ] 图片上传（本地存储）
- [ ] RSS 订阅
- [ ] 全文搜索（基础版）
- [ ] 浏览量统计

### Phase 6：安全与优化（1-2 天）

- [ ] Rate Limiting 中间件
- [ ] CORS 配置
- [ ] 输入校验完善
- [ ] 性能优化（数据库索引、查询优化）

### Phase 7：部署（1 天）

- [ ] Dockerfile 和 docker-compose
- [ ] Nginx 反向代理配置
- [ ] CI/CD 流程
- [ ] 生产环境配置

---

## 8. 环境变量

```env
# 应用配置
APP_HOST=0.0.0.0
APP_PORT=3000
APP_ENV=development

# 数据库
DATABASE_URL=sqlite:./data/blog.db?mode=rwc

# JWT
JWT_SECRET=your-secret-key-at-least-32-characters
JWT_ACCESS_EXPIRES=900          # 15 分钟，单位秒
JWT_REFRESH_EXPIRES=604800      # 7 天，单位秒

# 媒体上传
UPLOAD_DIR=./uploads
MAX_UPLOAD_SIZE=5242880         # 5MB，单位字节

# 日志
RUST_LOG=hello_axum=debug,tower_http=debug
```

---

## 9. 编码规范

### 9.1 Rust 风格

- 遵循 `rustfmt` 默认配置，使用 `cargo fmt` 格式化
- 使用 `cargo clippy` 检查代码质量，零警告
- 公开函数和类型必须添加文档注释 `///`
- **禁止使用 `unsafe`**，除非在极端性能热点且经过团队评审
- 使用 `#![deny(unsafe_code)]` 在 crate 级别禁止 unsafe
- 充分利用 Rust 类型系统：用 `Option<T>` 表示可能缺失，用 `Result<T, E>` 表示可能失败，用枚举表示有限状态
- 优先使用 `impl Into<String>` / `AsRef<str>` 而非固定 `String` 参数，减少不必要的分配
- 使用 `Cow<str>` 处理可能借用也可能拥有的字符串
- 优先使用迭代器方法链（`.map()` / `.filter()` / `.collect()`）替代命令式循环
- 使用 `#[non_exhaustive]` 标记公开枚举和结构体，保证 API 向后兼容

### 9.2 命名约定

- 文件名：snake_case（如 `auth_handler.rs`）
- 类型/结构体：PascalCase（如 `CreatePostRequest`）
- 函数/变量：snake_case（如 `create_post`）
- 常量：SCREAMING_SNAKE_CASE（如 `MAX_PAGE_SIZE`）
- 数据库列：snake_case
- API 路径：kebab-case（如 `/api/v1/refresh-tokens`）

### 9.3 错误处理

- **错误定义**：使用 `thiserror` derive 宏定义 `AppError` 枚举，每个变体对应一类业务错误
- **错误传播**：Service 内部使用 `anyhow::Result` 简化错误传播，在 Handler 边界转换为 `AppError`
- `AppError` 实现 `IntoResponse`，自动转换为 HTTP 响应
- 禁止使用 `unwrap()` 和 `expect()` 在非测试代码中，使用 `?` 或显式错误处理
- 数据库错误映射为业务语义错误（如唯一约束冲突 → 409 Conflict）
- 使用 `.ok_or(AppError::...)?` 将 Option 转为 Result
- 使用 `thiserror` 的 `#[source]` 属性保留错误链，配合 `tracing` 输出完整上下文

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("resource not found: {0}")]
    NotFound(String),

    #[error("unauthorized")]
    Unauthorized,

    #[error("forbidden")]
    Forbidden,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}
```

### 9.4 测试要求

- 每个 Service 函数编写单元测试
- 每个 Handler 编写集成测试
- 使用测试数据库，测试后清理
- 使用 `rstest` 编写参数化测试，减少重复
- 使用 `insta` 做 API 响应快照测试
- 使用 `mockall` Mock Service 层，隔离数据库依赖
- 目标测试覆盖率 > 70%
- CI 中使用 `cargo nextest run` 加速测试执行

### 9.5 性能要求

- 数据库连接使用连接池（sqlx 内置），避免频繁建连
- 热点查询使用 `sqlx::query_scalar!` 等编译期检查宏，减少运行时开销
- 分页查询必须使用 `LIMIT + OFFSET` 或游标分页，禁止全表扫描
- 使用 `Arc<T>` 共享状态，避免深层克隆（如 `Arc<Pool>`）
- 字符串处理优先使用 `&str` 借用，仅必要时转为 `String`
- 大量数据使用流式处理（`axum::body::BodyStream`），避免全量加载到内存
- 静态资源使用 `tower-http::services::ServeDir`，配合 `CompressionLayer` 压缩

### 9.6 Rust Idiom 清单

以下为必须遵循的 Rust 惯用法：

| 场景 | 推荐做法 | 反模式 |
|---|---|---|
| 错误处理 | `Result<T, E>` + `?` | `unwrap()` / `expect()` / `panic!` |
| 空值 | `Option<T>` | `null` / 占位默认值 |
| 字符串参数 | `&str` / `AsRef<str>` | 到处 `String` 克隆 |
| 共享所有权 | `Arc<T>` | 大量 `clone()` |
| 配置初始化 | `std::sync::LazyLock` / `once_cell` | 全局 `static mut` |
| 并发安全 | `Mutex<T>` / `RwLock<T>` / `tokio::sync` | 手动加锁 / `unsafe` |
| 类型转换 | `From<T> / Into<T>` / `TryFrom<T>` | 手动 `as` 强转 |
| 条件编译 | `#[cfg(test)]` / features | 运行时判断 |
| 资源管理 | RAII（Drop 守卫） | 手动 `close()` / `free()` |

---

## 10. 部署方案

### 10.1 Docker

```dockerfile
FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
COPY --from=builder /app/target/release/hello-axum /usr/local/bin/
COPY static/ /app/static/
EXPOSE 3000
CMD ["hello-axum"]
```

### 10.2 docker-compose

```yaml
services:
  app:
    build: .
    ports:
      - "3000:3000"
    env_file: .env
    volumes:
      - blog-data:/app/data
      - uploads:/app/uploads

  db:
    image: postgres:16
    profiles:
      - "postgres"
    environment:
      POSTGRES_DB: blog
      POSTGRES_USER: user
      POSTGRES_PASSWORD: password
    volumes:
      - pgdata:/var/lib/postgresql/data

volumes:
  blog-data:
  pgdata:
  uploads:
```

### 10.3 Nginx 反向代理

```nginx
server {
    listen 80;
    server_name blog.example.com;

    client_max_body_size 5M;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /uploads/ {
        alias /app/uploads/;
        expires 30d;
    }
}
```
