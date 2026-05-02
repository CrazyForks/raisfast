# Content Type Protocol 架构设计

> 版本：v5.0 · 日期：2026-05-02

## 一、目标

在 Content Type 系统中通过 AOP（Aspect-Oriented Programming）框架 + Protocol 机制实现：

1. **协议/trait 抽象** — 多个 Content Type 声明公共行为，系统自动注入
2. **默认能力** — 所有 CT 自动获得 `ownable` + `timestampable`，无需声明
3. **内置 Protocol** — 无争议的原子能力（`soft_deletable`、`versionable`、`cacheable`）
4. **插件自定义协议** — 第三方插件注册 Protocol，无限扩展（未实现）
5. **`__meta` 元数据** — 记录附加私有数据（已实现基础版：存储 `protocols` 列表）

### 架构决策：内置表不接入 Protocol

内置表（posts/pages/comments/categories/tags/media）是 typed struct + `sqlx::query().bind()`，
字段和 SQL 完全静态。Protocol 注入的值需要绕一圈再塞回 struct，是负收益。

**Content Type 表是动态 Record（`Map<String, Value>`），Aspect 修改 Record 直接成为 SQL 值源，路径最短。**

因此：
- 内置表 → Service 层直写 `created_by`/`updated_by`/`created_at`/`updated_at`
- Content Type 表 → 走 AspectEngine dispatch，Aspect 注入值直接写入 Record

## 二、三层分离

```
┌─────────────────────────────────────────────────────────┐
│ aspects/  — 纯框架层                                     │
│   Aspect trait + AspectEngine + Context 类型             │
│   不知道 Protocol 存在                                    │
├─────────────────────────────────────────────────────────┤
│ protocols/  — 业务 Protocol 实现（1:N 组合 Aspect）        │
│   Protocol trait + ProtocolRegistry                      │
│   每个 Protocol 包含 1 个 Aspect + columns() + name()     │
├─────────────────────────────────────────────────────────┤
│ services/aspect_dispatch.rs  — 预留（内置表已不使用）       │
│ handler.rs  — 调用 aspect_engine.dispatch_xxx()           │
└─────────────────────────────────────────────────────────┘
```

### AspectEngine 调度流程

```
handler::do_create()
  → aspect_engine.dispatch_data_before_create(table, &mut ctx)
    → get_aspects(JoinPointId{Data, Create, Before}, table)
      → 按 priority 升序排列（数字越小越先执行）
      → 只返回 enabled 且 pointcut 匹配 table 的 Aspect
    → 依次调用 aspect.on_data_before_create(&mut ctx)
      → Continue: 继续
      → Skip: 跳过后续 Aspect，但原始操作继续
      → Return(Value): 短路返回（用于缓存命中）
  → repo.create() 写入数据库
  → aspect_engine.dispatch_data_after_create(table, &mut ctx)
    → 同上，但 after 只处理 Continue
```

### Priority 排序

| Aspect | Priority | 说明 |
|--------|----------|------|
| OwnableAspect | -500 | 最先执行，注入 created_by/updated_by |
| TimestampableAspect | -400 | 注入 created_at/updated_at |
| SoftDeletableAspect | -300 | 删除时拦截，注入 deleted_at/deleted_by |
| VersionableAspect | 500 | after_update 异步写 revision |
| CacheableAspect | 1000 | 最后执行（目前占位） |

## 三、Protocol 分级

```
等级 1：默认内置（所有 CT 自动获得，不用声明）
  - ownable       → created_by + updated_by + 自动填当前用户
  - timestampable → created_at + updated_at

等级 2：内置 Protocol（implements 声明）
  - soft_deletable → 删除时标记 deleted_at 而非物理删除
  - versionable    → 更新时自动写 revision 到 content_revisions 表
  - cacheable      → 占位，当前仅日志（handler 层有独立 CMS 缓存实现）

等级 3：插件 Protocol（implements 声明 + 安装对应插件）（未实现）
  - slugable      → seo 插件
  - purchasable   → payment 插件
  - searchable    → search 插件
  - commentable   → comment 插件
  - ...无限扩展
```

### 筛选原则

Protocol 必须同时满足：

1. **原子性** — 不可拆分为更小的能力
2. **通用性** — 多个 CT 都需要，不绑定特定业务
3. **无争议** — 行为明确，不需要大量配置

| 能力 | 原子 | 通用 | 无争议 | 结论 |
|------|------|------|--------|------|
| ownable | 是 | 几乎所有 CT | 是 | **默认内置** |
| timestampable | 是 | 所有 CT | 是 | **默认内置** |
| soft_deletable | 是 | 需要数据恢复的 CT | 是 | **内置 Protocol** |
| versionable | 是 | 需要审计的 CT | 是 | **内置 Protocol** |
| cacheable | 是 | 高读低写 CT | 是 | **内置 Protocol**（占位） |
| publishable | **否** — 是 status + datetime 联动 | 只有内容型 CT | 否 | **不适合** |
| slugable | **否** — 依赖源字段 + 生成规则 | 大部分 CT | 否 | **插件 Protocol** |
| taggable | **否** — 依赖 relation 定义 | 部分 CT | 否 | **插件 Protocol** |
| searchable | 是 | 部分 CT | 是 | **插件 Protocol** |
| purchasable | 是 | 电商 CT | 是 | **插件 Protocol** |

### publishable 为什么不适合做内置 Protocol

- 绑定 `status` 字段语义（每个 CT 的 status 含义不同：draft/published vs pending/paid/shipped）
- 绑定"草稿→发布"业务流程（不是所有 CT 都有这个流程）
- 不是原子能力 — 是"datetime 字段 + status 联动逻辑"的组合

### slugable 为什么适合做插件 Protocol

slug 不是独立存在的 — 它依赖另一个字段，而且生成规则各不相同：

| CT | 源字段 | 生成规则 |
|----|--------|---------|
| Post | title | `rust-编程指南` |
| Product | name + category | `electronics/耳机-蓝牙` |
| Event | title + date | `2026-05-01-tech-summit` |

## 四、数据分层

```
┌──────────────────────────────────────────────────┐
│ 第 1 层：原生列（TOML 定义 + 默认内置 + Protocol 注入）│
│  → 核心业务数据，参与查询/过滤/排序/索引            │
│  → title, status, slug, price, created_by ...    │
├──────────────────────────────────────────────────┤
│ 第 2 层：__meta JSON（无需 ALTER TABLE）           │
│  → 不参与查询过滤，按需读取                         │
│  → protocols（已实现）、plugin_data、ui（未实现）    │
├──────────────────────────────────────────────────┤
│ 第 3 层：关联表                                   │
│  → 结构化关系、revision、tags                      │
└──────────────────────────────────────────────────┘
```

**核心原则：需要 WHERE / ORDER BY 的放原生列，其他的放 `__meta`。**

## 五、`__meta` JSON 结构

每条 Content Type 记录（`!ct.builtin && !ct.implements.is_empty()`）自动包含 `__meta` JSON 列。

### 已实现

创建时自动写入：
```json
{
  "protocols": ["versionable", "soft_deletable"]
}
```

### 未来扩展（未实现）

```json
{
  "protocols": ["versionable", "purchasable"],
  "ui": {
    "icon": "document",
    "color": "#3b82f6",
    "list_fields": ["title", "status", "published_at"],
    "detail_layout": "two-column"
  },
  "plugin_data": {
    "analytics": { "views_30d": 1234, "unique_visitors": 567 },
    "seo": { "meta_title": "...", "canonical_url": "..." }
  },
  "computed": {
    "reading_time": 5,
    "word_count": 1200
  }
}
```

| Key | 状态 | 谁写 | 谁读 | 说明 |
|-----|------|------|------|------|
| `protocols` | **已实现** | TOML 定义 | 系统引擎 | 声明的协议列表 |
| `ui` | 未实现 | Admin UI / TOML | Admin 前端 | 列表显示字段、图标、颜色 |
| `plugin_data.{plugin_id}` | 未实现 | 对应插件 | 对应插件 | 插件私有命名空间 |
| `computed` | 未实现 | 系统/插件 | API 响应 | 运行时计算缓存的衍生数据 |

### 跨数据库支持

| 数据库 | JSON 类型 | 索引 | 部分更新 |
|--------|----------|------|---------|
| SQLite | `TEXT` + `json_extract()` | 表达式索引（有限） | 无（全量替换） |
| PostgreSQL | `JSONB` | GIN 索引（优秀） | `jsonb_set()` |
| MySQL 8.0+ | `JSON` | 虚拟生成列 + 索引 | `JSON_SET()` |

## 六、5 个内置 Protocol 详解

### 6.1 ownable — 所有权（默认内置，无需声明）

- **Priority:** -500
- **Pointcuts:** `before_create` + `before_update`
- **注入字段:** `created_by` (text), `updated_by` (text)

自动行为：
- `before_create` → 从 `ctx.base.user_id` 写入 `created_by` + `updated_by`
- `before_update` → 写入 `updated_by`，不修改 `created_by`

### 6.2 timestampable — 时间戳（默认内置，无需声明）

- **Priority:** -400
- **Pointcuts:** `before_create` + `before_update`
- **注入字段:** `created_at` (text, ISO 8601), `updated_at` (text, ISO 8601)

自动行为：
- `before_create` → 写入 `created_at` + `updated_at`（覆盖已有值）
- `before_update` → 写入 `updated_at`，不修改 `created_at`

### 6.3 soft_deletable — 软删除（需声明）

- **Priority:** -300
- **Pointcuts:** `before_delete`
- **注入字段:** `deleted_at` (text), `deleted_by` (text)

声明方式（二选一）：
```toml
[content_type]
soft_delete = true
# 或者
implements = ["soft_deletable"]
```

自动行为：
- `before_delete` → 设置 `ctx.soft_delete = true`，注入 `deleted_at` + `deleted_by`
- Handler 根据 `soft_delete` 标志调用 `repo.soft_delete()`（UPDATE SET deleted_at/deleted_by）而非 `repo.delete()`
- `find()` (list) 和 `find_by_slug()` 自动过滤 `deleted_at IS NULL`
- `find_by_id()` **不过滤**（管理员可查看已删除记录，delete 流程需要先读取）

### 6.4 versionable — 版本控制（需声明）

- **Priority:** 500
- **Pointcuts:** `after_update`
- **注入字段:** `version` (integer, default=1)（仅声明，version 历史存 content_revisions 表）

```toml
[content_type]
versioning = true
```

自动行为：
- `after_update` → 异步 (`tokio::spawn`) 调用 `models::content_revision::create_revision()`
- 将 `old_record` 作为 snapshot 存入 `content_revisions` 表
- `revision_number` 自动递增（同一 content_type + record_id 下 MAX + 1）

`content_revisions` 表结构：

```sql
CREATE TABLE content_revisions (
    id TEXT PRIMARY KEY,              -- UUID v7
    content_type TEXT NOT NULL,       -- 表名
    record_id TEXT NOT NULL,          -- 记录 ID
    revision_number INTEGER NOT NULL, -- 自增版本号
    snapshot TEXT NOT NULL,           -- JSON 快照（old_record）
    created_by TEXT,                  -- 操作者
    created_at TEXT NOT NULL,         -- 操作时间
    UNIQUE(content_type, record_id, revision_number)
);
```

支持 API：
- `GET /cms/{plural}/{id}/revisions` — 列出版本历史
- `GET /cms/{plural}/{id}/revisions/{revision_id}` — 获取快照
- 版本间 diff 计算（`compute_diff`）

### 6.5 cacheable — 缓存（需声明，当前占位）

- **Priority:** 1000
- **Pointcuts:** `after_read` + `after_create` + `after_update` + `after_delete`
- **注入字段:** 无

当前状态：所有 handler 仅输出 `tracing::debug!` 日志。

CMS 缓存已在 handler 层独立实现（`DashMap` + TTL），不经过 AspectEngine：
- `do_list` / `do_get` → 检查 `cms_cache`，命中则直接返回
- `do_create` / `do_update` / `do_delete` → 调用 `invalidate_cms_cache()` 清除对应 CT 的所有缓存

未来可将缓存引用注入 `BaseContext.extensions`，让 CacheableAspect 接管。

## 七、Aspect 框架核心

### Aspect trait

```rust
#[async_trait]
pub trait Aspect: Send + Sync + 'static {
    fn name(&self) -> &str;
    fn priority(&self) -> i32 { 0 }
    fn pointcuts(&self) -> Vec<Pointcut>;
    fn columns(&self) -> Vec<ColumnDef> { vec![] }

    // Data Layer (8 hooks)
    async fn on_data_before_create(&self, ctx: &mut DataBeforeCreateContext) -> AspectResult;
    async fn on_data_after_create(&self, ctx: &mut DataAfterCreateContext) -> AspectResult;
    async fn on_data_before_read(&self, ctx: &mut DataBeforeReadContext) -> AspectResult;
    async fn on_data_after_read(&self, ctx: &mut DataAfterReadContext) -> AspectResult;
    async fn on_data_before_update(&self, ctx: &mut DataBeforeUpdateContext) -> AspectResult;
    async fn on_data_after_update(&self, ctx: &mut DataAfterUpdateContext) -> AspectResult;
    async fn on_data_before_delete(&self, ctx: &mut DataBeforeDeleteContext) -> AspectResult;
    async fn on_data_after_delete(&self, ctx: &mut DataAfterDeleteContext) -> AspectResult;

    // Access Layer, Event Layer, HTTP Layer (10 hooks)
    // ...
}
```

### Context 类型

```
BaseContext {
    user_id: Option<String>,
    user_role: Option<String>,
    tenant_id: String,
    now: String,                    // ISO 8601 时间戳
    request_id: String,
    extensions: Extensions,         // 类型安全的任意数据容器
    pool: Option<Pool>,             // 数据库连接池
}

DataBeforeCreateContext { base, table, record, schema }
DataAfterCreateContext  { base, table, record, schema }
DataBeforeReadContext   { base, table, query, schema }
DataAfterReadContext    { base, table, records: Vec<Record>, schema }
DataBeforeUpdateContext { base, table, old_record, new_record, schema }
DataAfterUpdateContext  { base, table, old_record, new_record, schema }
DataBeforeDeleteContext { base, table, record, soft_delete: bool, schema }
DataAfterDeleteContext  { base, table, record, schema }
```

其中 `Record = serde_json::Map<String, Value>`。

### Handler 层 Dispatch 点

| 函数 | Before Dispatch | After Dispatch | 说明 |
|------|----------------|----------------|------|
| `do_create` | `before_create` | `after_create` | 注入 created_by/at → 写 DB → 通知 |
| `do_update` | `before_update` | `after_update` | 传入 old_record → 写 DB → versionable 存快照 |
| `do_delete` | `before_delete` | `after_delete` | soft_delete 拦截 → 写 DB → 通知 |
| `do_list` | `before_read` | `after_read` | 缓存命中可短路 → 查询 → Aspect 可修改记录 |
| `do_get` | — | — | 不经过 AspectEngine（走独立 CMS 缓存） |

### Advice 枚举

```rust
pub enum Advice {
    Continue,         // 继续下一个 Aspect
    Skip,             // 跳过后续 Aspect，原始操作继续
    Return(Value),    // 短路返回（仅 before dispatch 支持）
}
```

## 八、注册流程

```rust
// src/lib.rs — build_app_state()
let mut protocol_registry = ProtocolRegistry::new();
protocol_registry.register(OwnableProtocol);      // priority -500
protocol_registry.register(TimestampableProtocol); // priority -400
protocol_registry.register(SoftDeletableProtocol); // priority -300
protocol_registry.register(VersionableProtocol);   // priority 500
protocol_registry.register(CacheableProtocol);     // priority 1000

let aspect_engine = AspectEngine::new();
protocol_registry.register_aspects_into(&aspect_engine);
// → 按 aspect name 去重，插入 dispatch_table，按 priority 排序
```

### 加载顺序

```
启动
 │
 ├── 1. 创建 ProtocolRegistry，注册 5 个内置 Protocol
 │
 ├── 2. 创建 AspectEngine，register_aspects_into()
 │     → 按 JoinPointId 预计算 dispatch table
 │     → priority 排序：ownable(-500) → timestampable(-400) →
 │       soft_deletable(-300) → versionable(500) → cacheable(1000)
 │
 ├── 3. 加载插件 → 注册插件 Protocol（未实现）
 │
 ├── 4. 加载 Content Type TOML
 │     ├── 解析 implements 列表
 │     ├── repo.migrate() → CREATE TABLE / ALTER TABLE
 │     │   ├── 默认列：id, tenant_id, status, slug...
 │     │   ├── ownable 列：created_by, updated_by
 │     │   ├── timestampable 列：created_at, updated_at
 │     │   ├── soft_deletable 列：deleted_at, deleted_by（如有声明）
 │     │   ├── versionable 列：version（如有声明）
 │     │   └── __meta 列（!builtin && !implements.is_empty()）
 │     └── 注册路由
 │
 └── 5. 启动 HTTP 服务器
```

## 九、TOML 定义方式

### 最小 CT（只有默认内置）

```toml
[content_type]
name = "Link"
singular = "link"
plural = "links"
table = "links"

[fields.title]
type = "text"

[fields.url]
type = "url"
required = true

# 自动获得：created_by, updated_by, created_at, updated_at
```

### CT + 软删除

```toml
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"
implements = ["soft_deletable"]

[fields.title]
type = "text"
required = true

[fields.content]
type = "richtext"

[fields.status]
type = "select"
options = ["draft", "published", "archived"]
default = "draft"

# 自动获得：created_by, updated_by, created_at, updated_at, deleted_at, deleted_by
```

### CT + 版本控制

```toml
[content_type]
name = "Document"
singular = "document"
plural = "documents"
table = "documents"
versioning = true

[fields.title]
type = "text"
required = true

[fields.body]
type = "richtext"

# 自动获得：created_by, updated_by, created_at, updated_at, version
# 更新时自动存 revision 到 content_revisions 表
```

### CT + 全部内置 Protocol

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["soft_deletable", "cacheable"]
versioning = true

[fields.title]
type = "text"
required = true

[fields.price]
type = "number"
required = true

# 自动获得：created_by, updated_by, created_at, updated_at
# soft_deletable: deleted_at, deleted_by
# versionable: version + content_revisions
# cacheable: 占位
```

### CT + 插件 Protocol（未实现）

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["soft_deletable", "versionable", "purchasable"]

[fields.title]
type = "text"
required = true

[fields.description]
type = "richtext"

# purchasable 由 payment 插件提供，自动注入 price, currency, inventory
```

### Single Type

```toml
[content_type]
name = "SiteSetting"
singular = "site_setting"
plural = "site_settings"
table = "site_settings"
kind = "single"
implements = ["cacheable"]

[fields.site_title]
type = "text"
default = "My Site"
```

## 十、运行时行为

### 创建记录

```
POST /api/v1/cms/articles
{ "title": "Rust 编程指南", "content": "..." }
```

Handler 执行链：
1. **before_create dispatch**（按 priority 顺序）：
   - `OwnableAspect.on_data_before_create` → `record["created_by"] = "user-123"`, `record["updated_by"] = "user-123"`
   - `TimestampableAspect.on_data_before_create` → `record["created_at"] = "2026-05-02T12:00:00Z"`, `record["updated_at"] = "2026-05-02T12:00:00Z"`
2. `repo.create()` → INSERT INTO
3. **after_create dispatch**：
   - （当前无 aspect 监听 after_create）

写入数据库：
```json
{
  "id": "0192abc...",
  "title": "Rust 编程指南",
  "content": "...",
  "created_by": "user-123",
  "updated_by": "user-123",
  "created_at": "2026-05-02T12:00:00Z",
  "updated_at": "2026-05-02T12:00:00Z",
  "__meta": { "protocols": ["soft_deletable"] }
}
```

### 更新记录

```
PUT /api/v1/cms/articles/{id}
{ "title": "Rust 编程指南（第2版）" }
```

Handler 执行链：
1. `repo.find_by_id()` → 获取 `old_record`
2. **before_update dispatch**：
   - `OwnableAspect.on_data_before_update` → `new_record["updated_by"] = "user-123"`
   - `TimestampableAspect.on_data_before_update` → `new_record["updated_at"] = "2026-05-02T13:00:00Z"`
3. `repo.update()` → UPDATE SET
4. **after_update dispatch**：
   - `VersionableAspect.on_data_after_update` → `tokio::spawn(create_revision(pool, table, id, old_record, user_id))`

### 删除记录（软删除）

```
DELETE /api/v1/cms/articles/{id}
```

Handler 执行链：
1. `repo.find_by_id()` → 获取 `record`
2. **before_delete dispatch**：
   - `SoftDeletableAspect.on_data_before_delete` → `ctx.soft_delete = true`, `record["deleted_at"] = "..."`, `record["deleted_by"] = "user-123"`
3. 检查 `before_ctx.soft_delete == true` → 调用 `repo.soft_delete()` (UPDATE SET deleted_at, deleted_by)
4. **after_delete dispatch**

### 列表查询（软删除过滤）

```
GET /api/v1/cms/articles
```

`ContentRepository::find()` 自动添加 `WHERE deleted_at IS NULL`（当 `ct.soft_delete || ct.implements.contains("soft_deletable")`）。

## 十一、数据库 Migration

### CREATE TABLE（自定义 CT）

```sql
CREATE TABLE IF NOT EXISTS articles (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    -- 用户定义的字段
    title TEXT,
    content TEXT,
    status TEXT DEFAULT 'draft',
    -- ownable（默认内置）
    created_by TEXT,
    updated_by TEXT,
    -- timestampable（默认内置）
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    -- soft_deletable（implements 声明）
    deleted_at TEXT,
    deleted_by TEXT,
    -- versionable（versioning = true）
    version INTEGER DEFAULT 1,
    -- 元数据
    __meta TEXT DEFAULT '{}'
);
```

### ALTER TABLE（增量同步）

`repo.migrate()` 对比 schema 与现有列，只补不删不改：

```
已有列: [id, title, created_at]
需要列: [id, title, content, created_at, updated_at, deleted_at]

ALTER TABLE articles ADD COLUMN content TEXT;
ALTER TABLE articles ADD COLUMN updated_at TEXT NOT NULL DEFAULT (datetime('now'));
ALTER TABLE articles ADD COLUMN deleted_at TEXT;
```

### 内置表

内置表（posts/pages/comments/categories/tags）的列在 `migrations/*.sql` 中显式定义，不依赖运行时注入。

## 十二、插件 Protocol（未实现）

### 概述

插件通过 `manifest.toml` 声明 Protocol，用插件代码实现行为。第三方开发者可以发布"协议包"，任何 Content Type 只要 `implements` 就能获得完整能力。

### 可能的插件 Protocol（示例）

| Protocol | 插件 | 注入字段 | 注入路由 |
|----------|------|---------|---------|
| `slugable` | seo | slug (text, unique) | 无 |
| `purchasable` | payment | price, currency, inventory | `POST /{id}/purchase` |
| `searchable` | search | 无 | 无（Hook 注入） |
| `commentable` | comment | 无 | `GET/POST /{id}/comments` |
| `reviewable` | review | rating (computed) | `POST /{id}/review` |
| `expirable` | scheduler | expires_at (datetime) | 无 |
| `subscribable` | subscription | plan, billing_cycle | `POST /{id}/subscribe` |

### 依赖检查

如果 Content Type 声明了 `implements = ["purchasable"]`，但 `payment` 插件未安装：

```
content type 'Product' registration failed:
  protocol 'purchasable' not found (provided by plugin 'payment')
```

## 十三、插件 meta 数据（未实现）

### 命名空间隔离

```json
{
  "plugin_data": {
    "analytics": { "views_30d": 1234 },
    "seo": { "meta_title": "..." }
  }
}
```

### SDK 接口

| 函数 | 说明 |
|------|------|
| `sdk.getMeta(recordId)` | 读取当前插件的 `plugin_data.{plugin_id}` |
| `sdk.storeMeta(recordId, data)` | 写入/合并当前插件的 `plugin_data.{plugin_id}` |
| `sdk.deleteMeta(recordId)` | 删除当前插件的 meta 数据 |

### 权限控制

- 插件只能读写自己的 `plugin_data.{plugin_id}` 命名空间
- 跨插件读取需要在 manifest.toml 声明：`meta_read = ["analytics"]`
- `plugin_data` 默认不返回公开 API

## 十四、`__meta` 返回规则

| 场景 | 返回内容 |
|------|---------|
| 公开 API（无认证） | `protocols` + `computed`（不返回 `plugin_data`） |
| Admin API（有认证） | 全部（含 `plugin_data`） |
| 插件 SDK 调用 | 只返回当前插件的 `plugin_data.{plugin_id}` |
| 列表 API | 不返回 `__meta` |
| 详情 API | 返回 `__meta` |

## 十五、与竞品对比

| 系统 | 类似机制 | 差异 |
|------|---------|------|
| **Strapi** | Component（可复用字段组） | 无协议/行为抽象，只是数据复用 |
| **Payload** | Field-level Hooks | 有行为但没有协议声明 |
| **WordPress** | Post Type Supports | 最接近，但硬编码 8 个选项，不可扩展 |
| **Directus** | None | 无类似机制 |
| **本系统** | Protocol（默认内置 + 声明式 + 插件可扩展） | 最灵活，第三方可注册自定义协议 |

核心优势：

1. **默认即有价值** — 所有 CT 自动获得 ownable + timestampable，零配置
2. **内置只保留原子能力** — soft_deletable、versionable、cacheable 无争议、无副作用
3. **复杂能力交给插件** — slugable、purchasable 等由各自领域的插件提供
4. **插件可扩展** — 第三方可以注册新 Protocol

## 十六、实现进度

### 已完成

- [x] Aspect 框架核心（Aspect trait + AspectEngine + Context 类型）
- [x] Protocol 层（Protocol trait + ProtocolRegistry）
- [x] 5 个内置 Protocol（ownable / timestampable / soft_deletable / versionable / cacheable 占位）
- [x] Content Type handler 完整 dispatch（before/after create/update/delete/read）
- [x] `__meta` 基础版（创建时写入 `protocols` 列表）
- [x] 内置表列名统一（migration 025: `created_by` / `updated_by` / `created_at` / `updated_at`）
- [x] AuthUser 统一身份系统
- [x] CMS 缓存（handler 层 DashMap + TTL）
- [x] 集成测试（920 tests passed）

### 未实现

- [ ] 插件 Protocol 注册（manifest.toml 解析 + 动态加载）
- [ ] `__meta` 扩展（ui / plugin_data / computed）
- [ ] 插件 meta 读写 SDK（getMeta / storeMeta / deleteMeta）
- [ ] 协议路由注入（`[[protocol.routes]]`）
- [ ] 字段自动注入（Protocol 默认字段合并到 CT）
- [ ] UI 配置（`[content_type.ui]`）
- [ ] CacheableAspect 接入 CMS 缓存
- [ ] 内置表接入 Protocol（已决定不做）

### "插件→原生晋升" 路径

```
1. JS 插件定义 purchasable Protocol + Product CT
2. 用户使用，验证需求
3. 用 Rust 重写 purchasable 为高性能 Protocol
4. Product CT 的 TOML 不用改，只是 Protocol 来源变了
5. 如果 Product 足够热门，升级为 builtin CT
   → 字段写入 migration SQL
   → 加 builtin = true
   → 走原生 handler（不走 Protocol Engine）
```
