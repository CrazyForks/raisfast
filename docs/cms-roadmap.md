# CMS 化改造方案

> 将 raisfast 从博客系统改造为通用 CMS 平台，具备高度可扩展性，
> 可作为公司官网、新闻站、电商内容管理、SaaS 后端等系统的基础。

## 1. 设计理念

借鉴 Strapi 的 **Schema-Driven** 模式 + WordPress 的 **Hook 优先** 模式：

| 借鉴对象 | 核心思想 | 我们的做法 |
|----------|----------|------------|
| Strapi | Schema JSON → 自动建表 → 自动生成 API → 自动生成 Admin UI | Schema TOML → 自动 Migration → 泛型 API → 插件扩展 |
| WordPress | 注册 Content Type → 自动生成 Admin 菜单/URL → Hooks 扩展一切 | 插件注册 Content Type → 自动路由 → HookPoint 泛化 |

核心原则：

- **Schema 即代码**：内容类型用 TOML 定义，版本控制可追踪
- **约定优于配置**：注册一个 content type 自动获得 CRUD API + Admin 管理
- **插件优先**：新功能优先通过插件实现，而非硬编码
- **渐进迁移**：旧代码并行运行，逐步替换，不搞大爆炸重写
- **内置 + 扩展双轨制**：内置基础功能（blog）降低开箱门槛，Content Type 满足扩展需求
- **插件→原生晋升路径**：热门插件验证需求后，可用 Rust 原生重写为内置模块，API 不变用户无感迁移

---

## 2. 架构总览

```
┌──────────────────────────────────────────────────────────────┐
│  应用层                                                       │
│  Blog / 官网 / 新闻站 / Wiki / 电商商品管理                    │
│  （每个是不同的 content_type TOML + 插件组合）                  │
├──────────────────────────────────────────────────────────────┤
│  CMS 框架层                                                   │
│  ├── 内容类型引擎    (Phase 8)                                │
│  │   ├── Schema TOML 解析 / 校验 / 注册                       │
│  │   ├── 自动 Migration 生成                                  │
│  │   ├── 泛型 CRUD Repository                                │
│  │   └── 泛型 API Handler 自动注册                            │
│  ├── 动态 RBAC      (Phase 9)                                │
│  │   ├── 角色/权限数据表                                      │
│  │   ├── 细粒度权限矩阵 (per content type / per field)        │
│  │   └── 条件权限 (creator == $user.id)                       │
│  ├── 插件系统 v3    (Phase 10)                                │
│  │   ├── 插件注册 content type                                │
│  │   ├── 插件注册路由                                         │
│  │   ├── HookPoint 泛化 (ContentCreating::{type})             │
│  │   └── 生命周期扩展 (register/bootstrap/destroy)             │
│  ├── 站点配置       (Phase 11)                                │
│  │   ├── Options KV 表                                       │
│  │   ├── 自动加载机制                                         │
│  │   └── 公开/私有配置分离                                    │
│  ├── 多租户基础     (Phase 12)                                │
│  │   ├── tenant_id 隔离                                       │
│  │   ├── 域名 → 租户解析                                     │
│  │   └── 租户级配置覆盖                                       │
│  ├── 主题/模板      (Phase 13)                                │
│  │   ├── Tera 模板引擎                                        │
│  │   ├── 主题包结构                                           │
│  │   └── 服务端渲染 (SSR) 可选                                 │
│  └── Admin Dashboard API (Phase 14)                           │
│      ├── Content Type Builder API                             │
│      ├── 角色/权限管理 API                                    │
│      └── 仪表盘统计 API                                       │
├──────────────────────────────────────────────────────────────┤
│  基础设施层（已完成）                                          │
│  ├── Axum + Tokio                                             │
│  ├── SQLite / PostgreSQL / MySQL (多 DB 支持)                  │
│  ├── EventBus + Worker + Cron                                 │
│  ├── Tantivy 全文搜索                                         │
│  ├── WASM / JS / Lua 三引擎插件                               │
│  ├── JWT Auth + Rate Limiting                                 │
│  ├── SSE 实时推送                                             │
│  └── TLS / CORS / 密码策略                                    │
└──────────────────────────────────────────────────────────────┘
```

---

## 3. Phase 8：动态内容类型系统

这是整个 CMS 化的基石。完成后，现有硬编码的 `posts`、`categories`、`tags` 等
都将变为 content type 定义，CRUD API 由框架自动生成。

### 3.1 Content Type Schema 定义

每个内容类型一个 TOML 文件，放在 `content_types/` 目录下。

```toml
# content_types/post.toml

[content_type]
name = "Post"                          # 显示名称
singular = "post"                      # 单数 API 路径段
plural = "posts"                       # 复数 API 路径段
table = "posts"                        # 数据库表名
description = "博客文章"
draft_publish = true                   # 支持 draft/published/archived 状态
slug_field = "title"                   # 自动从 title 字段生成 slug
timestamps = true                      # 自动 created_at / updated_at
soft_delete = false                    # 是否软删除

# ── 字段定义 ──────────────────────────────────────────────

[fields.title]
type = "text"
required = true
max_length = 200
label = "标题"

[fields.slug]
type = "uid"
target_field = "title"
unique = true
label = "URL 标识"

[fields.content]
type = "richtext"
required = true
label = "正文"

[fields.excerpt]
type = "text"
max_length = 500
label = "摘要"

[fields.cover_image]
type = "media"
accept = ["image/*"]
max_count = 1
label = "封面图片"

[fields.status]
type = "enum"
enum_values = ["draft", "published", "archived"]
default = "draft"
label = "状态"

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "user"
label = "作者"

[fields.category]
type = "relation"
relation_type = "many_to_one"
target = "category"
label = "分类"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tag"
through = "posts_tags"
label = "标签"

[fields.view_count]
type = "integer"
default = 0
private = true
label = "浏览量"

[fields.is_pinned]
type = "boolean"
default = false
label = "是否置顶"

# ── 列表视图配置 ──────────────────────────────────────────

[list_view]
default_sort = "is_pinned:desc,created_at:desc"
columns = ["title", "status", "author", "category", "created_at"]

# ── 索引配置 ──────────────────────────────────────────────

[[indexes]]
fields = ["slug"]
unique = true

[[indexes]]
fields = ["status", "created_at"]
```

其他 content type 示例：

```toml
# content_types/product.toml — 电商商品
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
description = "商品"
draft_publish = true
slug_field = "name"
timestamps = true

[fields.name]
type = "text"
required = true
max_length = 200

[fields.price]
type = "decimal"
required = true
min = 0

[fields.inventory]
type = "integer"
default = 0
min = 0

[fields.images]
type = "media"
accept = ["image/*"]
max_count = 10

[fields.specs]
type = "json"
label = "商品规格 (JSON)"
```

```toml
# content_types/page.toml — 单页（公司介绍、关于我们等）
[content_type]
name = "Page"
singular = "page"
plural = "pages"
table = "pages"
description = "独立页面"
draft_publish = true
slug_field = "title"
timestamps = true

[fields.title]
type = "text"
required = true
max_length = 200

[fields.slug]
type = "uid"
target_field = "title"
unique = true

[fields.content]
type = "richtext"
required = true

[fields.layout]
type = "enum"
enum_values = ["full", "sidebar", "landing"]
default = "full"
```

### 3.2 字段类型系统

共 17 种内置字段类型，对齐 Strapi：

| 类型 | 存储映射 | 说明 | 校验规则 |
|------|----------|------|----------|
| `text` | VARCHAR / TEXT | 短文本 / 长文本 | `max_length`, `required`, `unique`, `pattern` |
| `richtext` | TEXT | Markdown 正文 | `required` |
| `integer` | INTEGER | 整数 | `min`, `max`, `required` |
| `bigint` | BIGINT | 大整数 | `min`, `max`, `required` |
| `decimal` | DECIMAL | 精确小数 | `precision`, `scale`, `min`, `max` |
| `float` | FLOAT | 浮点数 | `min`, `max`, `required` |
| `boolean` | BOOLEAN | 布尔 | `default` |
| `date` | TEXT (ISO 8601) | 日期 | `required`, `unique` |
| `datetime` | TEXT (ISO 8601) | 日期时间 | `required`, `unique` |
| `time` | TEXT | 时间 | `required` |
| `email` | VARCHAR | 邮箱 | `required`, `unique` |
| `password` | VARCHAR | 密码（Argon2 加密存储） | `min_length`, `max_length` |
| `enum` | VARCHAR | 枚举 | `values`, `default` |
| `uid` | VARCHAR | URL 安全标识 | `target_field`, `unique` |
| `json` | TEXT (JSON) | 任意 JSON | `schema` (可选 JSON Schema 校验) |
| `media` | VARCHAR (URL) | 媒体文件引用 | `accept`, `max_count` |
| `relation` | FK / junction | 关系 | `relation_type`, `target`, `through` |

字段通用属性：

| 属性 | 类型 | 说明 |
|------|------|------|
| `type` | string | 字段类型（必填） |
| `required` | bool | 是否必填 |
| `unique` | bool | 是否唯一 |
| `default` | value | 默认值 |
| `private` | bool | 私有字段，公开 API 隐藏，admin API 可见 |
| `label` | string | Admin UI 显示标签 |
| `description` | string | 字段说明 |
| `immutable` | bool | 创建后不可修改 |

六种关系类型：

| 关系 | 说明 | 示例 |
|------|------|------|
| `one_to_one` | 一对一 | 用户 ↔ 个人资料 |
| `one_to_many` | 一对多 | 用户 → 多篇文章 |
| `many_to_one` | 多对一 | 多篇文章 → 一个分类 |
| `many_to_many` | 多对多 | 文章 ↔ 标签（需 `through` 表） |
| `one_way` | 单向引用 | 文章 → 推荐文章 |
| `many_way` | 多向引用 | 无反向关系 |

### 3.3 项目结构

```
src/content_type/
├── mod.rs              # ContentTypeRegistry — 注册/查询所有 content type
├── schema.rs           # ContentTypeSchema / FieldSchema / FieldType / RelationConfig
├── parser.rs           # TOML → ContentTypeSchema 解析器
├── migration.rs        # schema → CREATE TABLE / ALTER TABLE SQL 生成器
├── repository.rs       # 泛型 ContentRepository — 动态 SQL 构建
├── handler.rs          # 泛型 CRUD API handler (list/get/create/update/delete)
├── validation.rs       # 根据 field type 校验输入数据
├── resolver.rs         # relation 字段解析（JOIN / 子查询 / 延迟加载）
├── slug.rs             # UID 字段自动生成
└── search.rs           # content type 与 Tantivy 搜索引擎集成
```

### 3.4 核心数据结构

```rust
/// 内容类型定义
struct ContentTypeSchema {
    name: String,
    singular: String,
    plural: String,
    table: String,
    description: String,
    fields: Vec<FieldSchema>,
    draft_publish: bool,
    slug_field: Option<String>,
    timestamps: bool,
    soft_delete: bool,
    indexes: Vec<IndexDef>,
    list_view: Option<ListViewConfig>,
}

/// 字段定义
struct FieldSchema {
    name: String,
    field_type: FieldType,
    required: bool,
    unique: bool,
    default: Option<serde_json::Value>,
    private: bool,
    immutable: bool,
    label: Option<String>,
    description: Option<String>,
    max_length: Option<usize>,
    min: Option<f64>,
    max: Option<f64>,
    pattern: Option<String>,
    relation: Option<RelationConfig>,
    media_config: Option<MediaConfig>,
    enum_values: Option<Vec<String>>,
}

/// 字段类型枚举
enum FieldType {
    Text,
    RichText,
    Integer,
    BigInt,
    Decimal,
    Float,
    Boolean,
    Date,
    DateTime,
    Time,
    Email,
    Password,
    Enum,
    Uid,
    Json,
    Media,
    Relation,
}

/// 关系配置
struct RelationConfig {
    relation_type: RelationType,
    target: String,            // 目标 content type 名称
    through: Option<String>,   // 多对多中间表
    foreign_key: Option<String>,
}

/// 内容类型注册表
struct ContentTypeRegistry {
    types: HashMap<String, ContentTypeSchema>,
}

impl ContentTypeRegistry {
    /// 从 content_types/ 目录加载所有 TOML 定义
    fn load_from_dir(dir: &Path) -> AppResult<Self>;

    /// 注册单个 content type
    fn register(&mut self, schema: ContentTypeSchema);

    /// 按名称查询
    fn get(&self, name: &str) -> Option<&ContentTypeSchema>;

    /// 按表名查询
    fn get_by_table(&self, table: &str) -> Option<&ContentTypeSchema>;

    /// 获取所有已注册 content type
    fn all(&self) -> Vec<&ContentTypeSchema>;
}
```

### 3.5 泛型 Content Repository

替代现有的 `PostRepository`、`CategoryRepository` 等硬编码实现：

```rust
struct ContentRepository {
    pool: Pool,
    registry: Arc<ContentTypeRegistry>,
}

/// 通用查询参数
struct ContentQuery {
    page: i64,
    page_size: i64,
    sort: Option<String>,                    // "created_at:desc,title:asc"
    filters: Vec<Filter>,                    // 字段级过滤
    status: Option<String>,                  // draft / published / archived
    search: Option<String>,                  // 全文搜索关键词
    fields: Option<Vec<String>>,             // 选择字段
    include_relations: Vec<String>,          // 要解析的 relation 字段
}

struct Filter {
    field: String,
    operator: FilterOp,                      // eq, ne, gt, gte, lt, lte, in, like, is_null
    value: serde_json::Value,
}

impl ContentRepository {
    /// 动态构建 SELECT 查询
    async fn find(
        &self,
        ct: &ContentTypeSchema,
        query: ContentQuery,
    ) -> AppResult<(Vec<serde_json::Value>, i64)> {
        // 1. 构建 SELECT 子句：过滤 private 字段
        // 2. 构建 WHERE 子句：filters + status
        // 3. 构建 ORDER BY 子句
        // 4. 构建 LIMIT/OFFSET
        // 5. COUNT(*) 总数
        // 6. 执行查询，映射为 JSON
    }

    /// 按 ID 查找
    async fn find_by_id(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
    ) -> AppResult<Option<serde_json::Value>>;

    /// 按 slug 查找
    async fn find_by_slug(
        &self,
        ct: &ContentTypeSchema,
        slug: &str,
    ) -> AppResult<Option<serde_json::Value>>;

    /// 创建
    async fn create(
        &self,
        ct: &ContentTypeSchema,
        data: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        // 1. 校验 required 字段
        // 2. 生成 slug (如果 slug_field 存在)
        // 3. 设置 timestamps
        // 4. 设置默认值
        // 5. 构建 INSERT SQL
        // 6. 处理 relation (junction table 写入)
    }

    /// 更新
    async fn update(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
        data: serde_json::Value,
    ) -> AppResult<serde_json::Value>;

    /// 删除
    async fn delete(
        &self,
        ct: &ContentTypeSchema,
        id: &str,
    ) -> AppResult<()>;

    /// 批量操作
    async fn batch_delete(&self, ct: &ContentTypeSchema, ids: &[String]) -> AppResult<u64>;
    async fn batch_update_status(&self, ct: &ContentTypeSchema, ids: &[String], status: &str) -> AppResult<u64>;
}
```

### 3.6 泛型 API Handler

注册一个 content type 后自动生成完整 REST API：

```rust
/// 为所有 content type 自动注册路由
fn register_content_routes(router: Router, registry: &ContentTypeRegistry, state: AppState) -> Router {
    let mut api = router;

    for ct in registry.all() {
        let plural = ct.plural.clone();

        // 公开 API
        api = api
            .route(
                &format!("/api/v1/{plural}"),
                get(generic_list).post(generic_create),
            )
            .route(
                &format!("/api/v1/{plural}/:slug"),
                get(generic_get).put(generic_update).delete(generic_delete),
            );

        // Admin API（含所有状态）
        api = api
            .route(
                &format!("/api/v1/admin/{plural}"),
                get(generic_admin_list),
            )
            .route(
                &format!("/api/v1/admin/{plural}/:slug"),
                get(generic_admin_get),
            );
    }

    api
}
```

自动生成的 API 端点示例（以 `posts` 为例）：

| Method | Path | 说明 | 认证 |
|--------|------|------|------|
| GET | `/api/v1/posts` | 文章列表（仅 published） | 否 |
| GET | `/api/v1/posts/:slug` | 文章详情 | 否 |
| POST | `/api/v1/posts` | 创建文章 | 是 |
| PUT | `/api/v1/posts/:slug` | 更新文章 | 是 |
| DELETE | `/api/v1/posts/:slug` | 删除文章 | 是 |
| GET | `/api/v1/admin/posts` | 全状态列表 | 是 (admin) |
| GET | `/api/v1/admin/posts/:slug` | 全状态详情 | 是 (admin) |

查询参数（所有 content type 通用）：

```
GET /api/v1/posts
  ?page=1
  &page_size=20
  &sort=created_at:desc
  &filters[status]=published
  &filters[category]=uuid-xxx
  &filters[tags][in]=uuid-1,uuid-2
  &search=关键词
  &fields=title,slug,excerpt
  &include=author,category,tags
```

### 3.7 自动 Migration 生成

```rust
struct SchemaMigrator {
    pool: Pool,
    registry: Arc<ContentTypeRegistry>,
}

impl SchemaMigrator {
    /// 根据 content type 定义生成 CREATE TABLE SQL
    fn generate_create_table(ct: &ContentTypeSchema) -> String {
        let mut sql = format!("CREATE TABLE IF NOT EXISTS {} (\n", ct.table);

        // id TEXT PRIMARY KEY
        sql.push_str("    id TEXT PRIMARY KEY,\n");

        // 遍历字段，生成列定义
        for field in &ct.fields {
            let col_type = match field.field_type {
                FieldType::Text | FieldType::RichText | FieldType::Email => "TEXT",
                FieldType::Integer | FieldType::BigInt => "INTEGER",
                // ...
            };
            sql.push_str(&format!("    {} {}", field.name, col_type));
            if field.required { sql.push_str(" NOT NULL"); }
            if let Some(ref default) = field.default { sql.push_str(&format!(" DEFAULT {}", default)); }
            sql.push_str(",\n");
        }

        // 自动字段
        if ct.timestamps {
            sql.push_str("    created_at TEXT NOT NULL,\n");
            sql.push_str("    updated_at TEXT NOT NULL,\n");
        }
        if ct.draft_publish {
            sql.push_str("    published_at TEXT,\n");
        }
        if ct.soft_delete {
            sql.push_str("    deleted_at TEXT,\n");
        }

        sql.push_str(")");
        sql
    }

    /// 对比 schema 与现有表结构，生成 ALTER TABLE
    fn generate_alter_table(ct: &ContentTypeSchema, existing_columns: &[String]) -> Vec<String>;

    /// 执行 migration
    async fn migrate(&self) -> AppResult<()> {
        for ct in self.registry.all() {
            // 检查表是否存在
            // 不存在 → CREATE TABLE
            // 已存在 → ALTER TABLE ADD COLUMN (仅添加，不删不改)
            // 创建索引
            // 创建 junction 表 (many_to_many)
        }
        Ok(())
    }
}
```

### 3.8 现有代码迁移策略

**渐进迁移，不搞大爆炸：**

```
阶段 A：并行运行
  ├── content_types/post.toml 定义 Post schema
  ├── 新的 ContentRepository + 泛型 API 注册
  └── 旧的 PostRepository + 旧 handler 保留不变
  → 新 API 走 /api/v2/posts，旧 API 走 /api/v1/posts

阶段 B：功能对齐
  ├── 确保泛型 API 覆盖旧 API 所有功能
  ├── 迁移测试
  └── 旧 handler 标记 deprecated

阶段 C：切换
  ├── /api/v1/posts 内部转发到泛型 handler
  ├── 删除旧 PostRepository impl
  └── 保留 PostRepository trait 作为别名
```

需要迁移的现有实体：

| 实体 | 现有 Repository | Content Type | 迁移难度 |
|------|-----------------|--------------|----------|
| post | PostRepository (13 methods) | `post.toml` | 中（relation 多） |
| category | CategoryRepository (5 methods) | `category.toml` | 低 |
| tag | TagRepository (3 methods) | `tag.toml` | 低 |
| comment | CommentRepository (7 methods) | `comment.toml` | 中（嵌套关系） |
| media | MediaRepository (4 methods) | `media.toml` | 低 |
| user | UserRepository (7 methods) | 系统内置，不迁移 | — |
| refresh_token | RefreshTokenRepository | 系统内置，不迁移 | — |
| plugin_storage | 无 Repository | 系统内置，不迁移 | — |

---

## 4. Phase 9：动态 RBAC 权限系统

### 4.1 数据库表

```sql
-- 角色
CREATE TABLE roles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    description TEXT,
    is_system BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 权限
CREATE TABLE permissions (
    id TEXT PRIMARY KEY,
    role_id TEXT NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    subject TEXT NOT NULL,
    fields TEXT,                          -- JSON: ["title","content"] 或 ["*"]
    conditions TEXT,                      -- JSON: {"author_id": "$user.id"}
    created_at TEXT NOT NULL
);

-- 预置系统角色
INSERT INTO roles (id, name, description, is_system, created_at, updated_at) VALUES
    ('role-admin', 'admin', '超级管理员', 1, datetime('now'), datetime('now')),
    ('role-editor', 'editor', '编辑', 0, datetime('now'), datetime('now')),
    ('role-author', 'author', '作者', 0, datetime('now'), datetime('now')),
    ('role-reader', 'reader', '读者', 1, datetime('now'), datetime('now'));
```

### 4.2 权限格式

权限由三部分组成：`action` + `subject` + `fields` + `conditions`

**action 格式：**

```
content-type::{type}.{operation}    -- 内容操作
plugin::{name}.{action}             -- 插件操作
admin::{area}.{action}              -- 管理后台操作
```

**operation 类型：**

| operation | 说明 |
|-----------|------|
| `create` | 创建 |
| `read` | 读取 |
| `update` | 更新 |
| `delete` | 删除 |
| `publish` | 发布（draft → published） |
| `archive` | 归档（published → archived） |

**subject 格式：**

```
content-type::{type}                -- 特定内容类型
content-type::*                     -- 所有内容类型
plugin::{name}                      -- 特定插件
admin::{area}                       -- 管理区域
*                                   -- 所有
```

**通配符：**

| 格式 | 含义 |
|------|------|
| `content-type::post.*` | post 类型的所有操作 |
| `content-type::*.read` | 所有类型的读取操作 |
| `*` | 所有权限 |

**conditions（条件权限）：**

```json
{"author_id": "$user.id"}                    // 只能操作自己创建的内容
{"status": "published"}                      // 只能操作已发布的内容
{"organization_id": "$user.organization_id"} // 只能操作本组织的内容（多租户）
```

### 4.3 权限矩阵示例

| role | action | subject | fields | conditions |
|------|--------|---------|--------|------------|
| admin | `*` | `*` | `*` | |
| editor | `content-type::*.create` | `content-type::*` | `*` | |
| editor | `content-type::*.read` | `content-type::*` | `*` | |
| editor | `content-type::*.update` | `content-type::*` | `*` | |
| editor | `content-type::*.publish` | `content-type::*` | `*` | |
| editor | `content-type::*.delete` | `content-type::*` | `["*"]` | |
| author | `content-type::post.create` | `content-type::post` | `*` | |
| author | `content-type::post.read` | `content-type::post` | `*` | |
| author | `content-type::post.update` | `content-type::post` | `["*"]` | `{"author_id": "$user.id"}` |
| author | `content-type::post.delete` | `content-type::post` | `["*"]` | `{"author_id": "$user.id"}` |
| reader | `content-type::post.read` | `content-type::post` | `["title","slug","content","excerpt"]` | |
| reader | `content-type::comment.create` | `content-type::comment` | `["content","nickname","email"]` | |

### 4.4 权限检查中间件

```rust
/// 替代现有的 AuthUser / AdminUser / AuthorUser 硬编码提取器
struct PermissionGuard {
    pub user_id: String,
    pub role_id: String,
}

impl PermissionGuard {
    /// 检查当前用户是否有权限执行操作
    async fn check(
        &self,
        pool: &Pool,
        action: &str,
        subject: &str,
        context: Option<&serde_json::Value>,
    ) -> AppResult<()> {
        // 1. 查询 role 对应的所有 permissions
        // 2. 匹配 action + subject（支持通配符）
        // 3. 如果有 conditions，检查 context 是否满足
        // 4. 匹配成功 → Ok(()), 否则 → Err(AppError::Forbidden)
    }
}

/// 使用示例
async fn generic_create(
    State(state): State<AppState>,
    Path(type_name): Path<String>,
    guard: PermissionGuard,
    Json(data): Json<serde_json::Value>,
) -> AppResult<ApiResponse<serde_json::Value>> {
    guard.check(
        &state.pool,
        &format!("content-type::{type_name}.create"),
        &format!("content-type::{type_name}"),
        None,
    ).await?;

    // ... 执行创建
}
```

### 4.5 与现有 Auth 的兼容

```
现有：JWT → Claims { sub, role }
                        ↓
               role == "admin" ?  OK : Forbidden (硬编码)

改造：JWT → Claims { sub, role_id }
                        ↓
               查询 permissions 表 → 匹配 action/subject → OK/Forbidden
```

JWT Claims 结构扩展：

```rust
struct Claims {
    sub: String,         // 用户 ID
    role_id: String,     // 角色 ID（替代原来的 role 字符串）
    exp: usize,
    iat: usize,
}
```

迁移策略：先兼容旧 token（`role` 字段映射到 `role_id`），逐步切换。

---

## 5. Phase 10：插件系统升级 (v3)

### 5.1 现有插件系统能力

| 能力 | 状态 |
|------|------|
| WASM / JS / Lua 三引擎 | ✅ |
| Hook: Filter / Action / StringFilter / HandleRoute | ✅ |
| 权限控制 (http / db / fs / config) | ✅ |
| VFS 虚拟文件系统 | ✅ |
| Cron 定时任务 | ✅ |
| 热重载 | ✅ |
| 自动禁用（连续错误） | ✅ |
| 指标收集 | ✅ |

### 5.2 需要新增的能力

| 能力 | 说明 | 优先级 |
|------|------|--------|
| 插件注册 content type | 插件通过 manifest 声明自己的数据模型 | P0 |
| 插件注册路由 | 插件声明自定义 REST 端点 | P0 |
| HookPoint 泛化 | 从 Post-specific → Content-Type-generic | P0 |
| 生命周期钩子 | register → bootstrap → destroy | P1 |
| 自定义字段类型 | 插件提供新的 field type 实现 | P1 |
| 插件间通信 | 插件调用其他插件的能力 | P2 |
| Admin UI 扩展点 | 插件注册 Admin 页面/组件 | P2 |

### 5.3 Manifest 扩展

```toml
# plugins/ecommerce/plugin.toml

[plugin]
id = "com.example.ecommerce"
name = "E-Commerce"
version = "1.0.0"
description = "电商功能插件"
runtime = "lua"
entry = "init.lua"

[permissions]
max_memory_mb = 32
database = ["read:products", "write:products", "read:orders", "write:orders"]
http = ["payment-gateway.example.com/*"]

# ── 新增：插件声明 content types ──────────────────────────

[[content_types]]
file = "schemas/product.toml"

[[content_types]]
file = "schemas/order.toml"

# ── 新增：插件注册自定义路由 ──────────────────────────────

[[routes]]
method = "GET"
path = "/products/featured"
handler = "get_featured_products"
auth = false

[[routes]]
method = "POST"
path = "/orders"
handler = "create_order"
auth = true
permission = "plugin::ecommerce.order.create"

[[routes]]
method = "GET"
path = "/orders/:id"
handler = "get_order"
auth = true
permission = "plugin::ecommerce.order.read"

# ── 新增：插件注册 Admin 页面 ────────────────────────────

[[admin_pages]]
path = "/ecommerce/products"
label = "商品管理"
icon = "shopping-bag"
component = "admin/products"

[[admin_pages]]
path = "/ecommerce/orders"
label = "订单管理"
icon = "receipt"
component = "admin/orders"

# ── 新增：生命周期钩子 ────────────────────────────────────

[hooks.on_register]
handler = "on_register"         # 插件加载时调用，注册 content type / 路由

[hooks.on_bootstrap]
handler = "on_bootstrap"        # 所有插件加载完毕后调用

[hooks.on_destroy]
handler = "on_destroy"          # 插件卸载时调用
```

### 5.4 HookPoint 泛化

从博客专属 hook → 通用 CMS hook：

| 现有 HookPoint | 泛化后 | 说明 |
|----------------|--------|------|
| `PostCreating` | `ContentCreating::post` | 创建前拦截/修改 |
| `PostCreated` | `ContentCreated::post` | 创建后通知 |
| `PostUpdating` | `ContentUpdating::post` | 更新前拦截 |
| `PostUpdated` | `ContentUpdated::post` | 更新后通知 |
| `PostDeleted` | `ContentDeleted::post` | 删除后通知 |
| `CommentCreating` | `ContentCreating::comment` | 评论创建前 |
| `CommentCreated` | `ContentCreated::comment` | 评论创建后 |
| `RenderMarkdown` | `RenderField::post.content` | 字段渲染 |
| `FilterHtml` | `FilterField::post.content` | 字段过滤 |
| `HandleRoute` | `HandleRoute` | 保留 |
| `OnLogin` | `OnLogin` | 保留 |
| `CronTick` | `CronTick` | 保留 |
| — | `ContentQuery::{type}` | 查询前拦截/改写（新增） |
| — | `FieldValidate::{type}.{field}` | 自定义字段校验（新增） |
| — | `ContentTransform::{type}` | 输出前转换（新增） |
| — | `UserRegistering` | 用户注册前拦截（新增） |
| — | `MediaUploading` | 媒体上传前拦截（新增） |

HookPoint 实现：

```rust
enum HookPoint {
    // ── 内容生命周期 ──
    ContentCreating { content_type: String },
    ContentCreated { content_type: String },
    ContentUpdating { content_type: String },
    ContentUpdated { content_type: String },
    ContentDeleted { content_type: String },
    ContentQuery { content_type: String },
    ContentTransform { content_type: String },

    // ── 字段级 ──
    RenderField { content_type: String, field: String },
    FilterField { content_type: String, field: String },
    FieldValidate { content_type: String, field: String },

    // ── 用户/认证 ──
    OnLogin,
    UserRegistering,

    // ── 媒体 ──
    MediaUploading,

    // ── 路由 ──
    HandleRoute,

    // ── 定时任务 ──
    CronTick,
}
```

插件 manifest hooks 配置兼容旧格式：

```toml
# 旧格式（兼容）
[hooks.on-cron-tick]
priority = 10

# 新格式（推荐）
[[hooks]]
point = "ContentCreating::post"
priority = 10
handler = "validate_post"

[[hooks]]
point = "FieldValidate::product.price"
priority = 5
handler = "validate_price"
```

### 5.5 插件注册路由实现

```rust
impl PluginManager {
    /// 收集所有插件声明的路由，合并到主 Router
    pub async fn collect_routes(&self) -> Vec<PluginRoute> {
        let mut routes = Vec::new();
        for plugin in self.plugins.read().await.values() {
            if let Some(manifest) = &plugin.manifest {
                for route_def in &manifest.routes {
                    routes.push(PluginRoute {
                        plugin_id: plugin.id.clone(),
                        method: route_def.method.clone(),
                        path: route_def.path.clone(),
                        handler: route_def.handler.clone(),
                        auth: route_def.auth,
                        permission: route_def.permission.clone(),
                    });
                }
            }
        }
        routes
    }
}

/// 动态路由 handler — 分发到对应插件的 handler 函数
async fn plugin_route_dispatcher(
    State(state): State<AppState>,
    Path(params): Path<HashMap<String, String>>,
    req: Request,
) -> AppResult<Response> {
    let route_info = state.plugin_routes.get(req.uri().path());
    // 调用对应插件的 handler 函数
    state.plugins.dispatch_route(route_info, req).await
}
```

---

## 6. Phase 11：站点配置系统

### 6.1 Options 表

参考 WordPress `wp_options` 设计：

```sql
CREATE TABLE options (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,                  -- JSON value
    autoload BOOLEAN NOT NULL DEFAULT 1,  -- 启动时预加载到内存
    updated_at TEXT NOT NULL
);
```

### 6.2 内置配置项

| key | 类型 | 默认值 | 说明 |
|-----|------|--------|------|
| `site_title` | string | `"My Site"` | 站点标题 |
| `site_description` | string | `""` | 站点描述 |
| `site_url` | string | 自动 | 站点 URL |
| `posts_per_page` | integer | `10` | 每页条目数 |
| `default_role` | string | `"reader"` | 新用户默认角色 |
| `comment_moderation` | boolean | `true` | 评论需审核 |
| `comment_order` | string | `"asc"` | 评论排序 |
| `allowed_origins` | array | `[]` | CORS 白名单 |
| `theme` | string | `"default"` | 当前主题 |
| `admin_email` | string | `""` | 管理员邮箱 |
| `timezone` | string | `"UTC"` | 时区 |
| `date_format` | string | `"%Y-%m-%d"` | 日期格式 |
| `permalink_structure` | string | `"/:year/:month/:slug"` | URL 结构 |
| `rss_items` | integer | `20` | RSS 条目数 |
| `maintenance_mode` | boolean | `false` | 维护模式 |

### 6.3 Options Service

```rust
struct OptionsService {
    cache: RwLock<HashMap<String, serde_json::Value>>,
    pool: Pool,
}

impl OptionsService {
    /// 启动时加载所有 autoload = true 的配置
    async fn load_autoload(&self) -> AppResult<()>;

    /// 获取配置（先查缓存，再查 DB）
    async fn get(&self, key: &str) -> Option<serde_json::Value>;

    /// 设置配置
    async fn set(&self, key: &str, value: serde_json::Value) -> AppResult<()>;

    /// 批量设置
    async fn set_batch(&self, pairs: HashMap<String, serde_json::Value>) -> AppResult<()>;

    /// 删除配置
    async fn delete(&self, key: &str) -> AppResult<()>;

    /// 获取所有公开配置（前端可见）
    async fn get_public(&self) -> HashMap<String, serde_json::Value>;
}
```

### 6.4 API

| Method | Path | 说明 | 认证 |
|--------|------|------|------|
| GET | `/api/v1/options/public` | 公开配置（站点标题等） | 否 |
| GET | `/api/v1/admin/options` | 所有配置 | 是 (admin) |
| PUT | `/api/v1/admin/options` | 批量更新 | 是 (admin) |
| GET | `/api/v1/admin/options/:key` | 获取单个配置 | 是 (admin) |
| PUT | `/api/v1/admin/options/:key` | 设置单个配置 | 是 (admin) |
| DELETE | `/api/v1/admin/options/:key` | 删除配置 | 是 (admin) |

---

## 7. Phase 12：多租户基础

### 7.1 数据库表

```sql
CREATE TABLE tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT UNIQUE,                   -- blog.example.com
    config TEXT NOT NULL DEFAULT '{}',    -- 租户级 JSON 配置覆盖
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### 7.2 租户隔离策略

**方案 A（推荐初期）：共享数据库 + tenant_id 列**

```sql
-- content_entries 统一表（替代每个 content type 一张表）
CREATE TABLE content_entries (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES tenants(id),
    content_type TEXT NOT NULL,           -- "post", "product", "page" ...
    data TEXT NOT NULL,                   -- JSON (所有字段值)
    status TEXT NOT NULL DEFAULT 'draft',
    slug TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    published_at TEXT,
    UNIQUE(tenant_id, content_type, slug)
);

CREATE INDEX idx_entries_tenant_type ON content_entries(tenant_id, content_type);
CREATE INDEX idx_entries_tenant_status ON content_entries(tenant_id, content_type, status);
```

**方案 B（后期可升级）：独立 schema（PostgreSQL）/ 独立数据库**

### 7.3 请求级租户解析

```rust
/// 中间件：根据请求解析当前租户
async fn tenant_middleware(
    mut req: Request,
    next: Next,
) -> Response {
    let host = req.headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("default");

    // 从 AppState 的 tenant_cache 查找
    if let Some(tenant) = resolve_tenant(host).await {
        req.extensions_mut().insert(tenant);
    }

    next.run(req).await
}
```

### 7.4 租户级配置覆盖

```json
// tenant.config 覆盖全局 options
{
    "site_title": "Company Blog",
    "theme": "corporate",
    "posts_per_page": 15
}
```

配置优先级：`租户配置 > 全局 options > 默认值`

---

## 8. Phase 13：主题/模板系统

### 8.1 主题包结构

```
themes/
├── default/
│   ├── theme.toml                   # 主题元数据
│   ├── templates/
│   │   ├── layout.html              # 基础布局
│   │   ├── index.html               # 首页（内容列表）
│   │   ├── detail.html              # 内容详情
│   │   ├── category.html            # 分类页
│   │   ├── tag.html                 # 标签页
│   │   ├── archive.html             # 归档页
│   │   ├── page.html                # 独立页面
│   │   ├── search.html              # 搜索结果页
│   │   └── 404.html                 # 404 页面
│   ├── partials/
│   │   ├── header.html              # 页头
│   │   ├── footer.html              # 页脚
│   │   ├── sidebar.html             # 侧边栏
│   │   ├── pagination.html          # 分页
│   │   └── comment.html             # 评论区
│   ├── assets/
│   │   ├── css/
│   │   │   └── style.css
│   │   ├── js/
│   │   │   └── main.js
│   │   └── images/
│   └── functions.lua                # 主题级逻辑（可选）
└── corporate/
    ├── theme.toml
    └── ...
```

### 8.2 主题元数据

```toml
# themes/default/theme.toml

[theme]
name = "Default"
version = "1.0.0"
description = "默认博客主题"
author = "raisfast"
license = "MIT"

[theme.supports]
content_types = ["post", "page", "category", "tag"]
features = ["search", "rss", "comments", "pagination"]

[theme.settings]
[theme.settings.color_scheme]
type = "enum"
enum_values = ["light", "dark", "auto"]
default = "auto"
label = "配色方案"

[theme.settings.accent_color]
type = "text"
default = "#3b82f6"
label = "主题色"
```

### 8.3 模板引擎

选用 **Tera**（Rust 的 Jinja2 模板引擎）：

- 运行时动态加载模板（CMS 必需）
- 继承 / include / macro 支持
- 内置过滤器丰富
- 性能优秀

```rust
use tera::Tera;

struct ThemeEngine {
    tera: Tera,
    theme_name: String,
}

impl ThemeEngine {
    /// 加载主题模板
    fn load(theme_dir: &Path) -> AppResult<Self>;

    /// 渲染模板
    fn render(&self, template: &str, context: &tera::Context) -> AppResult<String>;
}
```

### 8.4 SSR 渲染端点

```
GET /                          → themes/default/templates/index.html
GET /posts/:slug               → themes/default/templates/detail.html
GET /category/:slug            → themes/default/templates/category.html
GET /page/:slug                → themes/default/templates/page.html
GET /search?q=keyword          → themes/default/templates/search.html
```

当请求 `Accept: text/html` 时走 SSR，`Accept: application/json` 时走 API。

### 8.5 Headless 模式

主题系统可选。不配置 theme 时，系统为纯 Headless CMS，
所有交互通过 REST API，前端完全由 Next.js 或其他 SPA 驱动。

---

## 9. Phase 14：Admin Dashboard API

为管理后台前端提供完整的 CMS 管理 API。

### 9.1 Content Type Builder API

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/admin/content-types` | 列出所有 content type 定义 |
| GET | `/api/v1/admin/content-types/:name` | 获取单个 content type schema |
| POST | `/api/v1/admin/content-types` | 创建新 content type |
| PUT | `/api/v1/admin/content-types/:name` | 修改 content type 字段 |
| DELETE | `/api/v1/admin/content-types/:name` | 删除 content type |

创建 content type 示例：

```json
POST /api/v1/admin/content-types
{
    "name": "Product",
    "singular": "product",
    "plural": "products",
    "table": "products",
    "description": "商品",
    "draft_publish": true,
    "slug_field": "name",
    "timestamps": true,
    "fields": [
        {"name": "name", "type": "text", "required": true, "max_length": 200},
        {"name": "price", "type": "decimal", "required": true, "min": 0},
        {"name": "description", "type": "richtext"},
        {"name": "images", "type": "media", "accept": ["image/*"], "max_count": 10}
    ]
}
```

操作结果：
1. 生成 `content_types/product.toml`
2. 自动执行 `CREATE TABLE products (...)` migration
3. 自动注册 `/api/v1/products` CRUD 路由
4. 自动注册 Admin 管理页面

### 9.2 角色/权限管理 API

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/admin/roles` | 列出所有角色 |
| POST | `/api/v1/admin/roles` | 创建角色 |
| PUT | `/api/v1/admin/roles/:id` | 更新角色 |
| DELETE | `/api/v1/admin/roles/:id` | 删除角色 |
| GET | `/api/v1/admin/roles/:id/permissions` | 获取角色权限 |
| PUT | `/api/v1/admin/roles/:id/permissions` | 设置角色权限 |

### 9.3 仪表盘统计 API

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/admin/stats` | 总览统计 |
| GET | `/api/v1/admin/stats/content/:type` | 内容类型统计 |
| GET | `/api/v1/admin/stats/trends` | 趋势数据（近 N 天） |

```json
GET /api/v1/admin/stats
{
    "total_posts": 156,
    "total_comments": 423,
    "total_users": 89,
    "total_media": 234,
    "content_by_type": {
        "post": 156,
        "page": 12,
        "product": 45
    },
    "recent_activity": [
        {"type": "post.created", "title": "...", "at": "..."},
        {"type": "comment.created", "content": "...", "at": "..."}
    ]
}
```

---

## 10. 新增依赖

| crate | 版本 | 用途 | Phase |
|-------|------|------|-------|
| `tera` | 1.x | 模板引擎（Jinja2 语法） | 13 |
| `toml` | 0.8 | Content Type schema 解析（已有） | 8 |
| `jsonschema` | 可选 | JSON 字段校验 | 8 |

无需引入重量级新依赖，现有 `serde_json` + `sqlx` 足以支撑大部分功能。

---

## 11. 实施路线图

```
Phase 8   ██████████████████████████  动态内容类型系统     2-3 周
Phase 9   ████████████████            动态 RBAC           1-2 周
Phase 10  ██████████████              插件系统 v3          1-2 周
Phase 11  ████████                    站点配置系统         3-5 天
Phase 12  ██████                      多租户基础           1 周
Phase 13  ██████████                  主题/模板系统        1-2 周
Phase 14  ████████                    Admin Dashboard API  1 周

总计约 8-12 周
```

依赖关系：

```
Phase 8 (内容类型)
  ├── Phase 9  (RBAC 依赖 content type 名称作为 subject)
  ├── Phase 10 (插件注册 content type / 泛化 hook)
  ├── Phase 12 (多租户依赖 content_entries 表)
  ├── Phase 13 (主题渲染依赖 content type 数据)
  └── Phase 14 (Admin API 依赖 content type CRUD)

Phase 11 (站点配置) — 独立，可随时插入
```

建议顺序：

```
Week 1-3:   Phase 8  (内容类型系统)     ← 最高优先级
Week 3-4:   Phase 11 (站点配置)         ← 独立，穿插做
Week 4-5:   Phase 9  (动态 RBAC)        ← 依赖 Phase 8
Week 5-6:   Phase 10 (插件 v3)          ← 依赖 Phase 8
Week 7:     Phase 14 (Admin API)        ← 依赖 8+9
Week 8:     Phase 12 (多租户)           ← 可选
Week 9-10:  Phase 13 (主题系统)         ← 可选
```

---

## 12. 关键决策记录

| 决策 | 选项 A | 选项 B | 结论 | 理由 |
|------|--------|--------|------|------|
| Schema 存储 | TOML 文件 | 数据库 JSON | TOML 文件为主，DB 缓存 | 版本控制可追踪，可 code review |
| 通用查询 | 动态拼 SQL | 预编译每种类型 | 动态拼 SQL | 已有 `db::dialect::translate` 跨 DB 层 |
| 主题引擎 | Tera | Askama | Tera | 运行时动态加载，CMS 必需 |
| 前端 Admin | 扩展现有 Next.js | 独立 Admin SPA | 扩展现有 Next.js | 减少维护成本 |
| 现有代码 | 全部重写 | 逐步迁移 | 逐步迁移，新旧并行 | 降低风险 |
| 多租户 | 共享 DB + tenant_id | 独立 DB | 初期共享 DB | 部署简单 |
| 模板渲染 | 可选 SSR | 纯 Headless | 都支持 | Headless 模式是默认，SSR 可选 |

---

## 13. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 动态 SQL 性能低于硬编码 | 中 | 提供预编译选项，热点 content type 可退回硬编码 |
| 泛型 API 无法覆盖特殊需求 | 中 | HookPoint 机制允许插件拦截和改写任何阶段 |
| TOML schema 变更频繁导致 migration 复杂 | 中 | 只支持 ADD COLUMN，不支持 DROP/RENAME，需要手动处理 |
| 多租户查询性能 | 低 | 初期不做多租户，预留 tenant_id 字段即可 |
| 主题安全（模板注入） | 中 | Tera 默认自动转义，禁用危险过滤器 |
| Phase 8 工作量大导致阻塞后续 | 高 | 先实现核心 5 种字段类型，其余渐进添加 |

---

## 14. 验收标准

### Phase 8 验收

- [ ] `content_types/post.toml` 定义完整，可解析
- [ ] 启动时自动创建/更新 `posts` 表
- [ ] `/api/v1/posts` CRUD 全部通过泛型 handler 工作
- [ ] 现有 439 个测试不回归
- [ ] 新增 content type 只需创建 TOML 文件 + 重启

### Phase 9 验收

- [ ] `roles` / `permissions` 表创建
- [ ] 管理员可通过 API 创建角色并分配权限
- [ ] `PermissionGuard` 替代硬编码角色检查
- [ ] 条件权限（`author_id == $user.id`）生效

### Phase 10 验收

- [ ] 插件可通过 manifest 声明 content type
- [ ] 插件可通过 manifest 声明路由
- [ ] HookPoint 泛化后旧插件兼容运行
- [ ] `site-maintenance` 插件无需修改即可运行

### Phase 11 验收

- [ ] `options` 表 + autoload 机制工作
- [ ] `/api/v1/options/public` 返回站点标题等公开配置
- [ ] Admin 可通过 API 修改所有配置

### Phase 12 验收

- [ ] `tenants` 表 + `content_entries` 表创建
- [ ] 请求级租户解析中间件工作
- [ ] 数据按 tenant_id 隔离，跨租户不可见

### Phase 13 验收

- [ ] 默认主题 `themes/default/` 渲染正常
- [ ] 切换主题只需改 `options.theme` 配置
- [ ] Headless 模式不受影响

### Phase 14 验收

- [ ] Content Type Builder API 可创建新 content type
- [ ] 角色/权限管理 API 完整
- [ ] 仪表盘统计 API 返回正确数据
