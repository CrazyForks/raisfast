# Content Type 动态元数据架构设计

> 版本：v4.0 · 日期：2026-04-30

## 一、目标

在 Content Type 系统中引入 `__meta` JSON 字段 + 协议（Protocol）机制，实现：

1. **协议/ trait 抽象** — 多个 Content Type 声明公共行为，系统自动注入
2. **默认能力** — 所有 CT 自动获得 `ownable` + `timestampable`，无需声明
3. **内置 Protocol** — 极少量无争议的原子能力（`versionable`、`cacheable`）
4. **插件自定义协议** — 第三方插件注册 Protocol，无限扩展
5. **内置表复用 Protocol** — 内置 Blog/Pages 走同一套行为引擎
6. **插件扩展数据** — 插件通过 `__meta` 给记录附加私有数据

## 二、Protocol 分级

```
等级 1：默认内置（所有 CT 自动获得，不用声明）
  - ownable       → author_id + 自动填当前用户
  - timestampable → created_at + updated_at

等级 2：内置 Protocol（implements 声明，只有无争议的原子能力）
  - versionable   → 自动写 revision（纯行为，无字段依赖）
  - cacheable     → 自动缓存管理（纯行为，无字段依赖）

等级 3：插件 Protocol（implements 声明 + 安装对应插件）
  - slugable      → seo 插件（slug 生成规则因场景而异）
  - purchasable   → payment 插件
  - searchable    → search 插件
  - commentable   → comment 插件
  - reviewable    → review 插件
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
| versionable | 是 | 需要审计的 CT | 是 | **内置 Protocol** |
| cacheable | 是 | 高读低写 CT | 是 | **内置 Protocol** |
| publishable | **否** — 是 status + datetime 联动 | 只有内容型 CT | 否（status 语义各异） | **不适合** |
| slugable | **否** — 依赖源字段 + 生成规则 | 大部分 CT | 否（规则各不相同） | **插件 Protocol** |
| taggable | **否** — 依赖 relation 定义 | 部分 CT | 否 | **插件 Protocol** |
| searchable | 是 | 部分 CT | 是 | **插件 Protocol** |
| purchasable | 是 | 电商 CT | 是 | **插件 Protocol** |

### publishable 为什么不适合做内置 Protocol

`publishable` 看似通用，实际引入了过多复杂性：

- 绑定 `status` 字段语义（每个 CT 的 status 含义不同：draft/published vs pending/paid/shipped）
- 绑定"草稿→发布"业务流程（不是所有 CT 都有这个流程）
- 不是原子能力 — 是"datetime 字段 + status 联动逻辑"的组合

用户需要发布行为时，用 `published_at` 字段 + 插件 Hook 即可，或者用插件提供 `publishable` Protocol。

### slugable 为什么适合做插件 Protocol

slug 不是独立存在的 — 它依赖另一个字段，而且生成规则各不相同：

| CT | 源字段 | 生成规则 |
|----|--------|---------|
| Post | title | `rust-编程指南` |
| Product | name + category | `electronics/耳机-蓝牙` |
| Event | title + date | `2026-05-01-tech-summit` |

做成内置 Protocol 会限制灵活性，做成插件可以配置源字段和规则。

## 三、分层设计

```
┌──────────────────────────────────────────────────┐
│ 第 1 层：原生列（TOML 定义 + 默认内置 + Protocol 注入）│
│  → 核心业务数据，参与查询/过滤/排序/索引            │
│  → title, status, slug, price, author_id ...     │
├──────────────────────────────────────────────────┤
│ 第 2 层：__meta JSON（无需 ALTER TABLE）           │
│  → 不参与查询过滤，按需读取                         │
│  → protocols、plugin_data、ui、computed           │
├──────────────────────────────────────────────────┤
│ 第 3 层：关联表                                   │
│  → 结构化关系、revision、tags                      │
└──────────────────────────────────────────────────┘
```

**核心原则：需要 WHERE / ORDER BY 的放原生列，其他的放 `__meta`。**

## 四、`__meta` JSON 结构

每条 Content Type 记录自动包含一个 `__meta` JSON 列：

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

### 各 key 职责

| Key | 谁写 | 谁读 | 说明 |
|-----|------|------|------|
| `protocols` | TOML 定义 | 系统引擎 | 声明的协议（不含默认内置的 ownable/timestampable） |
| `ui` | Admin UI / TOML | Admin 前端 | 列表显示字段、图标、颜色、布局偏好 |
| `plugin_data.{plugin_id}` | 对应插件 | 对应插件 | 插件私有命名空间，互不干扰 |
| `computed` | 系统/插件 | API 响应 | 运行时计算缓存的衍生数据 |

### 跨数据库支持

| 数据库 | JSON 类型 | 索引 | 部分更新 |
|--------|----------|------|---------|
| SQLite | `TEXT` + `json_extract()` | 表达式索引（有限） | 无（全量替换） |
| PostgreSQL | `JSONB` | GIN 索引（优秀） | `jsonb_set()` |
| MySQL 8.0+ | `JSON` | 虚拟生成列 + 索引 | `JSON_SET()` |

**策略：** `__meta` 不参与 WHERE/ORDER BY，SQLite 的 JSON 劣势不影响性能。

## 五、默认内置能力

所有 Content Type 自动获得，不需要在 `implements` 中声明：

### ownable — 所有权

自动注入字段：
- `author_id` (text) — 创建时自动填充当前登录用户 ID

自动行为：
- `POST /cms/{plural}` → 从 JWT 提取用户 ID 写入 `author_id`
- Admin UI 显示 "只看我的" 过滤器
- API Rule 支持 `author_id = @request.auth.id` 条件

### timestampable — 时间戳

自动注入字段：
- `created_at` (text, ISO 8601) — 创建时间
- `updated_at` (text, ISO 8601) — 更新时间

自动行为：
- 创建时自动填充 `created_at` + `updated_at`
- 更新时自动更新 `updated_at`

注：`timestampable` 已经是当前系统的默认行为，Protocol 只是给它一个名字。

## 六、内置 Protocol

需要通过 `implements` 声明。只有无争议的原子能力才能成为内置 Protocol：

### versionable — 版本控制

自动注入字段：无

自动行为：
- 每次更新自动写入 `content_revisions` 表
- API 暴露 `GET /cms/{plural}/{id}/revisions`
- 支持 `POST /cms/{plural}/{id}/revert/{version}` 回滚

### cacheable — 缓存

自动注入字段：无

自动行为：
- GET 请求自动缓存（TTL 由 CT 配置决定）
- create/update/delete 自动清除缓存
- Admin UI 显示缓存状态

## 七、插件 Protocol

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

### 插件 manifest.toml

```toml
[plugin]
id = "payment"
name = "Payment"
version = "1.0.0"
runtime = "js"
entry = "main.js"

[[protocol]]
name = "purchasable"
description = "可购买的商品，自动注入价格/库存字段和购买 API"

[[protocol.fields]]
name = "price"
type = "number"
required = true

[[protocol.fields]]
name = "currency"
type = "select"
options = ["CNY", "USD"]
default = "CNY"

[[protocol.fields]]
name = "inventory"
type = "integer"
default = 0

[[protocol.hooks]]
on = "before_create"
handler = "onPurchasableCreate"

[[protocol.hooks]]
on = "before_update"
handler = "onPurchasableUpdate"

[[protocol.routes]]
method = "POST"
path = "/{id}/purchase"
handler = "handlePurchase"
```

### 插件 main.js

```javascript
export function onPurchasableCreate(ctx) {
  if (!ctx.data.price || ctx.data.price <= 0) {
    return sdk.fail(400, "price must be positive");
  }
  if (!ctx.data.inventory) {
    ctx.data.inventory = 0;
  }
}

export function onPurchasableUpdate(ctx) {
  if (ctx.existing.inventory > 0 && ctx.data.inventory === 0) {
    sdk.logInfo("product out of stock: " + ctx.record.id);
    sdk.eventEmit("product.out_of_stock", ctx.record.id);
  }
}

export async function handlePurchase(ctx) {
  const product = await sdk.dbQuery(
    `SELECT id, price, inventory FROM ${ctx.table} WHERE id = ?`,
    [sdk.extractJson(ctx.input, "params.id")]
  );

  if (!product || product.inventory <= 0) {
    return sdk.fail(400, "out of stock");
  }

  const quantity = sdk.extractJson(ctx.input, "body.quantity") || 1;
  const total = product.price * quantity;

  await sdk.dbExec(
    `UPDATE ${ctx.table} SET inventory = inventory - ? WHERE id = ?`,
    [quantity, product.id]
  );

  const orderId = sdk.newId();
  await sdk.dbExec(
    `INSERT INTO orders (id, product_id, quantity, total_price, status, user_id)
     VALUES (?, ?, ?, ?, 'pending', ?)`,
    [orderId, product.id, quantity, total, ctx.auth.id]
  );

  sdk.eventEmit("order.created", { orderId, total });
  return sdk.ok({ orderId, total, currency: product.currency });
}
```

### 依赖检查

如果 Content Type 声明了 `implements = ["purchasable"]`，但 `payment` 插件未安装：

```
content type 'Product' registration failed:
  protocol 'purchasable' not found (provided by plugin 'payment')
```

## 八、字段自动注入

### 合并规则

```
Protocol 默认字段 + 用户自定义字段 → 合并
用户显式定义了同名字段 → 用户覆盖（override）
Protocol 要求类型不匹配 → 报错
```

自动注入只适用于**自定义 CT**。内置 CT（`builtin=true`）字段全部显式定义。

### 示例：最小定义

```toml
# 自定义 CT — Protocol 字段可省略
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["versionable", "purchasable"]

[fields.title]
type = "text"

[fields.description]
type = "richtext"

# 自动获得（默认内置）：
#   ownable → author_id
#   timestampable → created_at, updated_at
# purchasable 自动注入：
#   price, currency, inventory
# versionable → 无字段，纯行为
```

### 示例：覆盖默认值

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["versionable", "purchasable"]

[fields.title]
type = "text"

# 覆盖 purchasable 的默认 currency 字段
[fields.currency]
type = "select"
options = ["CNY", "USD", "EUR", "JPY"]
default = "CNY"
```

## 九、TOML 定义方式

### 最小 CT（无 Protocol）

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

# 自动获得：author_id, created_at, updated_at
```

### CT + 内置 Protocol

```toml
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"
implements = ["versionable", "cacheable"]

[fields.title]
type = "text"

[fields.content]
type = "richtext"

[fields.status]
type = "select"
options = ["draft", "published", "archived"]
default = "draft"
```

### CT + 插件 Protocol

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["versionable", "purchasable"]

[fields.title]
type = "text"

[fields.description]
type = "richtext"

# purchasable 自动注入：price, currency, inventory
# versionable → 自动版本历史
# 默认内置：author_id, created_at, updated_at
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

### UI 配置

```toml
[content_type.ui]
icon = "shopping-bag"
color = "#8b5cf6"
list_fields = ["title", "price", "status"]
detail_layout = "two-column"
hidden_fields = ["internal_notes"]
```

## 十、注册时检查流程

```
1. 系统启动
2. 注入默认能力（ownable, timestampable）到所有 CT
3. 注册内置 Protocol（versionable, cacheable）
4. 加载插件 → 解析 manifest.toml → 注册插件 Protocol
5. 解析 Content Type TOML → 读取 implements 列表
6. 对每个协议：
   a. 查找协议定义（内置 + 插件注册表）
   b. 找不到 → 报错（未安装对应插件）
   c. 合并协议默认字段到 CT（用户显式定义的覆盖默认）
   d. 检查合并后字段类型是否匹配协议要求
7. 全部通过 → 注册路由（内置 CRUD + 协议注入的额外路由）
8. 任一失败 → 报错，拒绝注册
```

### 加载顺序图

```
启动
 │
 ├── 1. 注入默认能力
 │     所有 CT 自动获得 ownable + timestampable
 │
 ├── 2. 注册内置 Protocol
 │     versionable, cacheable
 │
 ├── 3. 加载插件
 │     ├── payment 插件 → 注册 purchasable
 │     ├── seo 插件 → 注册 slugable
 │     └── search 插件 → 注册 searchable
 │
 ├── 4. 加载 Content Type
 │     ├── Product.toml → implements ["versionable", "purchasable"]
 │     │   ├── 注入默认 → author_id, created_at, updated_at
 │     │   ├── purchasable → price, currency, inventory
 │     │   ├── 注册路由 + POST /cms/products/{id}/purchase
 │     │   └── versionable → 自动版本历史
 │     │
 │     └── Order.toml → implements []
 │         └── 只有默认：author_id, created_at, updated_at
 │
 └── 5. 启动 HTTP 服务器
```

## 十一、内置表复用 Protocol

### 设计原则

内置表（Blog/Pages/Media）字段全部**显式写全**（包括 Protocol 要求的字段），建表走静态 migration。**行为统一走 Protocol 引擎。**

| | 内置 CT (`builtin=true`) | 自定义 CT |
|---|---|---|
| 字段定义 | 显式写全（含默认内置和 Protocol 字段） | Protocol 字段可省略，自动注入 |
| 建表方式 | migration SQL 文件（静态） | `repo.migrate()` 动态 ALTER TABLE |
| Protocol 行为 | 统一走 Protocol 引擎 | 统一走 Protocol 引擎 |
| 可删除 | 否（可通过 `BUILTIN_xxx=false` 禁用） | 是 |

### 内置 Post

```toml
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
implements = ["versionable"]
builtin = true

[content_type.ui]
icon = "file-text"
color = "#3b82f6"
list_fields = ["title", "status", "created_at"]

[fields.title]
type = "text"
required = true

[fields.content]
type = "richtext"

[fields.excerpt]
type = "text"

[fields.cover_image]
type = "url"

[fields.is_pinned]
type = "boolean"
default = false

[fields.view_count]
type = "integer"
default = 0

[fields.status]
type = "select"
options = ["draft", "published", "archived"]
default = "draft"

[fields.slug]
type = "text"
unique = true

[fields.author_id]
type = "text"

# created_at, updated_at 由 migration 保证
```

### 内置 Page

```toml
[content_type]
name = "Page"
singular = "page"
plural = "pages"
table = "pages"
implements = ["versionable"]
builtin = true

[content_type.ui]
icon = "layout"
color = "#10b981"
list_fields = ["title", "status", "template"]

[fields.title]
type = "text"
required = true

[fields.slug]
type = "text"
unique = true

[fields.content]
type = "richtext"

[fields.blocks]
type = "json"

[fields.template]
type = "select"
options = ["default", "full", "landing", "contact"]
default = "default"

[fields.parent_id]
type = "relation"
target = "pages"

[fields.sort_order]
type = "integer"
default = 0

[fields.status]
type = "select"
options = ["draft", "published", "archived"]
default = "draft"

[fields.author_id]
type = "text"

[fields.meta_title]
type = "text"

[fields.meta_description]
type = "text"

[fields.og_image]
type = "url"

[fields.cover_image]
type = "url"
```

### 行为统一

```
之前：
  post_service::create_post()
    → 手动 slugify(title)
    → 手动 set author_id
    → 手动 emit event

  content_type handler::do_create()
    → 泛型创建，无上述行为

之后：
  Protocol Engine::before_create(ct, data)
    → ownable.on_create → 自动 author_id

  Protocol Engine::after_create(ct, record)
    → versionable.on_created → 自动写 revision
    → event.emit("content.created")

  # 内置 Post 和自定义 Product 走完全相同的代码路径
```

### 迁移策略

| 阶段 | 内容 | 风险 |
|------|------|------|
| **Phase 1** | Protocol 引擎做好，只用于自定义 CT | 低 |
| **Phase 2** | 内置 CT 加 `builtin=true` + `implements`，注册时验证字段 | 低 |
| **Phase 3** | 内置 CT 的 service 层逐步替换为 Protocol 行为 | 中 |
| **Phase 4** | 硬编码逻辑完全移除 | 高 |

### "插件→原生晋升" 路径

```
1. JS 插件定义 purchasable Protocol + Product CT
2. 用户使用，验证需求
3. 用 Rust 重写 purchasable 为高性能 Protocol
4. Product CT 的 TOML 不用改，只是 Protocol 来源变了
5. 如果 Product 足够热门，升级为 builtin CT
   → 字段写入 migration SQL
   → 加 builtin = true
   → 走原生 handler
```

## 十二、运行时行为

### 创建记录

```
POST /api/v1/cms/products
{
  "title": "Rust 编程指南",
  "price": 89.00,
  "currency": "CNY"
}
```

系统自动执行：
1. `ownable.on_create` → `author_id = current_user.id`
2. `timestampable.on_create` → `created_at = now()`, `updated_at = now()`
3. `purchasable.on_create` → `inventory = 0`

实际写入数据库：
```json
{
  "id": "0192abc...",
  "title": "Rust 编程指南",
  "price": 89.00,
  "currency": "CNY",
  "inventory": 0,
  "author_id": "user-123",
  "created_at": "2026-04-30T12:00:00Z",
  "updated_at": "2026-04-30T12:00:00Z",
  "__meta": {
    "protocols": ["versionable", "purchasable"]
  }
}
```

### 协议注入的额外路由

```
POST /api/v1/cms/products/0192abc.../purchase
{ "quantity": 2 }
```

→ 由 `purchasable` 协议注册，调用 payment 插件的 `handlePurchase`

## 十三、插件 meta 数据

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

### SDK 用法

```javascript
await sdk.storeMeta(recordId, { views_30d: 1234, bounce_rate: 0.32 });

const meta = await sdk.getMeta(recordId);
// { views_30d: 1234, bounce_rate: 0.32 }

await sdk.storeMeta(recordId, { unique_visitors: 567 });
// { views_30d: 1234, bounce_rate: 0.32, unique_visitors: 567 }
```

## 十四、API 影响

### 响应示例

```
GET /api/v1/cms/products/0192abc...
```

```json
{
  "code": 0,
  "data": {
    "id": "0192abc...",
    "title": "Rust 编程指南",
    "price": 89.00,
    "currency": "CNY",
    "inventory": 42,
    "author_id": "user-123",
    "created_at": "2026-04-30T12:00:00Z",
    "updated_at": "2026-04-30T12:00:00Z",
    "__meta": {
      "protocols": ["versionable", "purchasable"],
      "computed": { "revenue_total": 7890.00 }
    }
  }
}
```

### `__meta` 返回规则

| 场景 | 返回内容 |
|------|---------|
| 公开 API（无认证） | `protocols` + `computed`（不返回 `plugin_data`） |
| Admin API（有认证） | 全部（含 `plugin_data`） |
| 插件 SDK 调用 | 只返回当前插件的 `plugin_data.{plugin_id}` |
| 列表 API | 不返回 `__meta` |
| 详情 API | 返回 `__meta` |

## 十五、数据库 Migration

自定义 CT 自动建表时，默认添加 `__meta` 列 + 默认内置字段：

```sql
CREATE TABLE IF NOT EXISTS products (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    -- 用户定义 + Protocol 注入的字段...
    title TEXT,
    price REAL,
    currency TEXT DEFAULT 'CNY',
    inventory INTEGER DEFAULT 0,

    -- 默认内置
    author_id TEXT,
    __meta TEXT DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

内置 CT 的字段在 migration SQL 中显式定义，不依赖运行时注入。

## 十六、完整实例 — 电商系统

### 商品（Product）

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["versionable", "purchasable"]

[content_type.ui]
icon = "shopping-bag"
color = "#8b5cf6"
list_fields = ["title", "price", "inventory", "created_at"]

[fields.title]
type = "text"
required = true

[fields.description]
type = "richtext"

[fields.cover_image]
type = "url"

# 自动获得：
#   ownable → author_id
#   timestampable → created_at, updated_at
# purchasable 自动注入 → price, currency, inventory
# versionable → 自动版本历史
# purchasable 注册路由 → POST /cms/products/{id}/purchase
```

### 订单（Order）

```toml
[content_type]
name = "Order"
singular = "order"
plural = "orders"
table = "orders"

[fields.user_id]
type = "relation"
target = "users"
required = true

[fields.product_id]
type = "relation"
target = "products"
required = true

[fields.quantity]
type = "integer"
required = true
default = 1

[fields.total_price]
type = "number"

[fields.status]
type = "select"
options = ["pending", "paid", "shipped", "completed", "cancelled"]
default = "pending"

# 自动获得：author_id, created_at, updated_at
# 无 Protocol — status 是纯业务字段，不绑定任何行为
```

### API 路由汇总

```
Product:
  GET    /api/v1/cms/products              # 列表
  POST   /api/v1/cms/products              # 创建（自动 author_id + timestamps + inventory 默认值）
  GET    /api/v1/cms/products/{id}          # 详情
  PUT    /api/v1/cms/products/{id}          # 更新（自动 updated_at + versionable 快照）
  DELETE /api/v1/cms/products/{id}          # 删除
  POST   /api/v1/cms/products/{id}/purchase # 购买（purchasable 协议注入）

Order:
  GET    /api/v1/cms/orders                # 列表
  POST   /api/v1/cms/orders                # 创建（自动 author_id + timestamps）
  GET    /api/v1/cms/orders/{id}            # 详情
  PUT    /api/v1/cms/orders/{id}            # 更新
  DELETE /api/v1/cms/orders/{id}            # 删除
```

## 十七、实现路线

### Phase 1 — `__meta` 列 + 默认内置 + 内置 Protocol + 注册时检查

**改动范围：**
- `protocol.rs`（新文件）— Protocol 注册表 + 字段注入 + 合并检查
- `schema.rs` — `ContentTypeSchema` 新增 `implements` 字段 + `ui` 配置
- `content_type.rs` — `register()` 增加协议检查 + 默认内置字段注入
- `repository.rs` — 自动建表时添加 `__meta` 列 + `author_id` 列

**预计：** 2-3 天

### Phase 2 — 协议自动行为 + 插件协议注册

**改动范围：**
- `handler.rs` — create/update 时检查协议，自动触发 Hook
- `plugins/protocol.rs`（新文件）— 插件 manifest 解析 Protocol 定义
- 插件加载流程 — 先加载插件注册协议，再加载 Content Type
- API 响应附带 `__meta`

**预计：** 3-4 天

### Phase 3 — 插件 meta 读写 + 协议路由注入 + SDK

**改动范围：**
- `host_common.rs` — 新增 `get_meta()` / `store_meta()` Host API
- `sdk/js_plugin_v1.js` — 新增 `sdk.getMeta()` / `sdk.storeMeta()`
- `sdk/lua_plugin_v1.lua` — 同上
- 协议路由注册 — 解析 `[[protocol.routes]]` 注册到路由表

**预计：** 2-3 天

### Phase 4 — 内置表迁移 + UI hints

**改动范围：**
- 内置 Post/Page 的 service 层逐步替换为 Protocol 行为
- Admin 前端读取 `ui` 配置，个性化列表/详情页
- 协议字段在 Admin 中标记来源

**预计：** 3-4 天

## 十八、与竞品对比

| 系统 | 类似机制 | 差异 |
|------|---------|------|
| **Strapi** | Component（可复用字段组） | 无协议/行为抽象，只是数据复用 |
| **Payload** | Field-level Hooks | 有行为但没有协议声明 |
| **WordPress** | Post Type Supports | 最接近，但硬编码 8 个选项，不可扩展 |
| **Directus** | None | 无类似机制 |
| **本系统** | Protocol（默认内置 + 声明式 + 插件可扩展） | 最灵活，第三方可注册自定义协议 |

核心优势：

1. **默认即有价值** — 所有 CT 自动获得 ownable + timestampable，零配置
2. **内置只保留原子能力** — versionable 和 cacheable 无争议、无副作用
3. **复杂能力交给插件** — slugable、purchasable 等由各自领域的插件提供
4. **插件可扩展** — 第三方可以注册新 Protocol，WordPress `supports` 做不到
