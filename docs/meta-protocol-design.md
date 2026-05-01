# Content Type 动态元数据架构设计

> 版本：v3.0 · 日期：2026-04-30

## 一、目标

在 Content Type 系统中引入 `__meta` JSON 字段 + 协议（Protocol）机制，实现：

1. **协议/ trait 抽象** — 多个 Content Type 声明公共行为，系统自动注入行为
2. **字段自动注入** — 协议要求的字段自动合并，用户无需重复定义
3. **插件自定义协议** — 第三方插件可以注册自己的 Protocol，扩展系统行为
4. **内置表复用 Protocol** — 内置 Blog/Pages/Media 走同一套 Protocol 行为，统一架构
5. **插件扩展数据** — 插件可以给记录附加私有数据，无需 ALTER TABLE
6. **UI/行为配置** — Content Type 级别的个性化配置

## 二、分层设计

```
┌──────────────────────────────────────────────────┐
│ 第 1 层：原生列（TOML 定义 + Protocol 注入）        │
│  → 核心业务数据，参与查询/过滤/排序/索引            │
│  → title, status, slug, price ...                │
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

## 三、`__meta` JSON 结构

每条 Content Type 记录自动包含一个 `__meta` JSON 列：

```json
{
  "protocols": ["publishable", "slugable"],
  "ui": {
    "icon": "document",
    "color": "#3b82f6",
    "list_fields": ["title", "status", "published_at"],
    "detail_layout": "two-column",
    "hidden_fields": ["internal_notes"]
  },
  "plugin_data": {
    "analytics": { "views_30d": 1234, "unique_visitors": 567 },
    "seo": { "meta_title": "...", "canonical_url": "..." },
    "recommendations": { "score": 0.85, "related_ids": ["...", "..."] }
  },
  "computed": {
    "reading_time": 5,
    "word_count": 1200,
    "last_synced_at": "2026-04-30T12:00:00Z"
  }
}
```

### 各 key 职责

| Key | 谁写 | 谁读 | 说明 |
|-----|------|------|------|
| `protocols` | TOML 定义 | 系统引擎 | 协议声明，注册时检查字段是否满足 |
| `ui` | Admin UI / TOML | Admin 前端 | 列表显示字段、图标、颜色、布局偏好 |
| `plugin_data.{plugin_id}` | 对应插件 | 对应插件 | 插件私有命名空间，互不干扰 |
| `computed` | 系统/插件 | API 响应 | 运行时计算缓存的衍生数据 |

### 跨数据库支持

| 数据库 | JSON 类型 | 索引 | 部分更新 |
|--------|----------|------|---------|
| SQLite | `TEXT` + `json_extract()` | 表达式索引（有限） | 无（全量替换） |
| PostgreSQL | `JSONB` | GIN 索引（优秀） | `jsonb_set()` |
| MySQL 8.0+ | `JSON` | 虚拟生成列 + 索引 | `JSON_SET()` |

**策略：** `__meta` 不参与 WHERE/ORDER BY，所以 SQLite 的 JSON 劣势不影响性能。查询时总是按原生列过滤，`__meta` 只在返回结果时附带读取。

## 四、协议（Protocol）系统

### 设计思路

借鉴 Rust trait 的理念 — 多个 Content Type 通过声明 `implements` 获得公共行为，但用运行时检查代替编译时类型系统：

```
Rust trait:     编译时检查 → impl Trait for Type
CMS protocol:   注册时检查 → implements = ["publishable"]
```

### 核心创新点

1. **字段自动注入** — 协议要求的字段自动合并到 Content Type，用户不用重复写
2. **行为自动注入** — 协议定义的 Hook 在 create/update 时自动触发
3. **路由自动注入** — 协议可以注册额外的 API 路由
4. **插件可扩展** — 第三方插件可以注册自定义协议

### 内置协议定义

#### publishable — 可发布

自动注入字段：
- `status` (select, options: ["draft", "published", "archived"], default: "draft")
- `published_at` (datetime)

自动行为：
- `status` 变为 `published` 时，自动填充 `published_at`
- `status` 从 `published` 变为其他值时，自动清空 `published_at`
- API 自动过滤：公开 API 只返回 `status = 'published'` 的记录

#### slugable — 可读 URL

自动注入字段：
- `slug` (text, unique)

自动行为：
- 创建时根据 `title` 字段自动生成 slug（如果未提供）
- 更新时如果 `title` 变了且 `slug` 未手动修改，自动重新生成
- API 支持按 slug 查询：`GET /cms/posts/{slug}`

#### ownable — 所有权

自动注入字段：
- `author_id` (relation → users)

自动行为：
- 创建时自动填充当前登录用户 ID
- API Rule 自动注入 `author_id = @request.auth.id` 条件（如果配置）
- Admin UI 显示 "只看我的" 过滤器

#### taggable — 可标签

自动注入字段：
- `tags` (relation → 标签表)

自动行为：
- API 自动支持 `?tag_id=xxx` 查询参数
- Admin UI 显示标签筛选器

#### versionable — 版本控制

自动注入字段：
- 无额外字段

自动行为：
- 每次更新自动写入 `content_revisions` 表
- API 暴露 `GET /cms/{plural}/{id}/revisions`
- 支持 `POST /cms/{plural}/{id}/revert/{version}` 回滚

#### cacheable — 缓存

自动注入字段：
- 无额外字段

自动行为：
- GET 请求自动缓存（TTL 由 CT 配置决定）
- create/update/delete 自动清除缓存
- Admin UI 显示缓存状态

### 协议总表

| 协议 | 自动注入字段 | 自动行为 |
|------|-------------|----------|
| `publishable` | status (select) + published_at (datetime) | 自动填充发布时间，公开 API 过滤 |
| `slugable` | slug (text, unique) | 自动生成 slug，支持 slug 路由 |
| `ownable` | author_id (relation→users) | 自动填充作者，权限检查 |
| `taggable` | tags (relation) | 自动标签筛选 API |
| `versionable` | 无 | 自动写入版本历史，支持回滚 |
| `cacheable` | 无 | 自动缓存管理 |

## 五、字段自动注入

### 合并规则

```
Protocol 默认字段 + 用户自定义字段 → 合并
用户显式定义了同名字段 → 用户覆盖（override）
Protocol 要求类型不匹配 → 报错
```

### 示例：最小定义

```toml
# 只声明协议，不用手写协议要求的字段
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
implements = ["publishable", "slugable", "ownable"]

[fields.title]
type = "text"
required = true

[fields.content]
type = "richtext"

# status、published_at 由 publishable 自动注入
# slug 由 slugable 自动注入
# author_id 由 ownable 自动注入
```

等价于手写：

```toml
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"

[fields.title]
type = "text"
required = true

[fields.content]
type = "richtext"

[fields.status]                    # publishable 自动注入
type = "select"
options = ["draft", "published", "archived"]
default = "draft"

[fields.published_at]              # publishable 自动注入
type = "datetime"

[fields.slug]                      # slugable 自动注入
type = "text"
unique = true

[fields.author_id]                 # ownable 自动注入
type = "relation"
target = "users"
```

### 示例：覆盖默认值

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["publishable", "slugable"]

[fields.title]
type = "text"

[fields.price]
type = "number"

# 覆盖 publishable 的默认 status 字段（自定义选项）
[fields.status]
type = "select"
options = ["draft", "published", "archived", "out_of_stock"]

# published_at 仍由 publishable 自动注入（使用默认定义）
# slug 仍由 slugable 自动注入（使用默认定义）
```

## 六、插件自定义协议

### 概述

插件通过 `manifest.toml` 声明 Protocol，用插件代码实现行为。第三方开发者可以发布"协议包"，任何 Content Type 只要 `implements` 就能获得完整能力。

### 协议来源

| 来源 | 示例 | 注册时机 |
|------|------|---------|
| 内置 | publishable, slugable, ownable, versionable, cacheable | 编译时 |
| JS 插件 | purchasable, subscribable, rateable | 插件加载时 |
| Lua 插件 | 同上 | 插件加载时 |
| WASM 插件 | 同上 | 插件加载时 |

### 插件 manifest.toml

```toml
[plugin]
id = "payment"
name = "Payment"
version = "1.0.0"
runtime = "js"
entry = "main.js"

# 插件注册的协议
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

### Content Type 使用插件协议

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["publishable", "purchasable"]  # purchasable 来自 payment 插件

[fields.title]
type = "text"

[fields.description]
type = "richtext"

[fields.cover_image]
type = "url"

# 以下字段自动注入：
# publishable → status, published_at
# purchasable → price, currency, inventory

# 自动注册额外路由：
# POST /api/v1/cms/products/{id}/purchase → payment 插件 handlePurchase
```

### 依赖检查

如果 Content Type 声明了 `implements = ["purchasable"]`，但 `payment` 插件未安装：

```
content type 'Product' registration failed:
  protocol 'purchasable' not found (provided by plugin 'payment')
```

## 七、TOML 定义方式

### 最小 Content Type（无协议）

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
```

### Single Type + 协议

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

### 完整协议

```toml
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
implements = ["publishable", "slugable", "ownable", "taggable", "versionable"]

[content_type.ui]
icon = "document"
color = "#3b82f6"
list_fields = ["title", "status", "published_at", "author_id"]

[fields.title]
type = "text"
required = true

[fields.content]
type = "richtext"

[fields.excerpt]
type = "text"

[fields.cover_image]
type = "url"

# 以下自动注入：
# publishable → status (select), published_at (datetime)
# slugable    → slug (text, unique)
# ownable     → author_id (relation → users)
# taggable    → tags (relation)
# versionable → (无额外字段)
```

### UI 配置

```toml
[content_type.ui]
icon = "shopping-bag"                              # Admin 列表图标
color = "#8b5cf6"                                   # 品牌色
list_fields = ["title", "price", "status"]          # 列表显示的字段
detail_layout = "two-column"                        # 详情页布局
hidden_fields = ["internal_notes"]                  # 列表隐藏的字段
```

## 八、注册时检查流程

```
1. 系统启动
2. 注册内置 Protocol（publishable, slugable, ownable ...）
3. 加载插件 → 解析 manifest.toml → 注册插件自定义 Protocol
4. 解析 Content Type TOML → 读取 implements 列表
5. 对每个协议：
   a. 查找协议定义（内置 + 插件注册表）
   b. 找不到 → 报错（未安装对应插件）
   c. 合并协议默认字段到 CT（用户显式定义的覆盖默认）
   d. 检查合并后字段类型是否匹配协议要求
   e. 检查字段约束（unique, options, min, max）
6. 全部通过 → 注册路由（内置 CRUD + 协议注入的额外路由）
7. 任一失败 → 报错，拒绝注册
```

### 错误示例

```
content type 'Product' registration failed:
  protocol 'purchasable' requires field 'price' of type 'number', but found 'text'
  protocol 'slugable' requires field 'slug' with unique=true, but field has unique=false
```

### 加载顺序图

```
启动
 │
 ├── 1. 注册内置 Protocol
 │     publishable, slugable, ownable, taggable, versionable, cacheable
 │
 ├── 2. 加载插件
 │     ├── payment 插件 → 注册 purchasable Protocol
 │     ├── newsletter 插件 → 注册 subscribable Protocol
 │     └── analytics 插件 → 注册 trackable Protocol
 │
 ├── 3. 加载 Content Type
 │     ├── Product.toml → implements ["publishable", "purchasable"]
 │     │   ├── 检查 publishable（内置）→ 通过
 │     │   ├── 检查 purchasable（payment 插件）→ 通过
 │     │   ├── 合并字段（自动注入 status/published_at/price/currency/inventory）
 │     │   └── 注册路由 + POST /cms/products/{id}/purchase
 │     │
 │     └── Order.toml → implements []
 │         └── 纯自定义字段，无协议
 │
 └── 4. 启动 HTTP 服务器
```

## 九、运行时行为

### 创建记录

```
POST /api/v1/cms/products
{
  "title": "Rust 编程指南",
  "price": 89.00,
  "currency": "CNY",
  "status": "published"
}
```

系统自动执行：
1. `slugable.on_create` → `slug = "rust-编程指南"`
2. `publishable.on_create` → `published_at = "2026-04-30T12:00:00Z"`
3. `purchasable.on_create` → `inventory = 0`

实际写入数据库：
```json
{
  "id": "0192abc...",
  "title": "Rust 编程指南",
  "slug": "rust-编程指南",
  "price": 89.00,
  "currency": "CNY",
  "status": "published",
  "published_at": "2026-04-30T12:00:00Z",
  "inventory": 0,
  "__meta": {
    "protocols": ["publishable", "purchasable"]
  }
}
```

### 更新记录

```
PUT /api/v1/cms/products/0192abc...
{ "status": "archived" }
```

系统自动执行：
1. `publishable.on_update` → `published_at = null`

### 协议注入的额外路由

```
POST /api/v1/cms/products/0192abc.../purchase
{ "quantity": 2 }
```

→ 由 `purchasable` 协议注册，调用 payment 插件的 `handlePurchase`
→ 扣减 inventory，创建 Order 记录

## 十、插件 meta 数据

### 命名空间隔离

每个插件的 meta 数据存储在 `plugin_data.{plugin_id}` 下，互不干扰：

```json
{
  "plugin_data": {
    "analytics": { "views_30d": 1234 },
    "seo": { "meta_title": "..." },
    "recommendations": { "score": 0.85 }
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
- `plugin_data` 默认不返回公开 API（需 admin 权限或显式配置）

### SDK 用法

```javascript
// 写入 meta
await sdk.storeMeta(recordId, {
  views_30d: 1234,
  bounce_rate: 0.32,
});

// 读取 meta
const meta = await sdk.getMeta(recordId);
// { views_30d: 1234, bounce_rate: 0.32 }

// 更新（合并，不覆盖其他 key）
await sdk.storeMeta(recordId, {
  unique_visitors: 567,
});
// 现在 plugin_data.analytics = { views_30d: 1234, bounce_rate: 0.32, unique_visitors: 567 }
```

## 十一、API 影响

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
    "slug": "rust-编程指南",
    "price": 89.00,
    "currency": "CNY",
    "status": "published",
    "published_at": "2026-04-30T12:00:00Z",
    "inventory": 42,
    "__meta": {
      "protocols": ["publishable", "purchasable"],
      "computed": {
        "revenue_total": 7890.00
      }
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
| 列表 API | 不返回 `__meta`（减少响应体积） |
| 详情 API | 返回 `__meta` |

## 十二、数据库 Migration

Content Type 自动建表时，默认添加 `__meta` 列：

```sql
CREATE TABLE IF NOT EXISTS products (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL DEFAULT 'default',
    -- 用户定义 + 协议注入的字段...
    title TEXT,
    slug TEXT UNIQUE,
    status TEXT DEFAULT 'draft',
    published_at TEXT,
    price REAL,
    currency TEXT DEFAULT 'CNY',
    inventory INTEGER DEFAULT 0,

    -- 系统自动添加
    __meta TEXT DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

已有表通过 ALTER TABLE 添加：

```sql
ALTER TABLE products ADD COLUMN __meta TEXT DEFAULT '{}';
```

## 十三、完整实例 — 电商系统

### Content Type 定义

**商品（Product）：**

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
implements = ["publishable", "slugable", "purchasable"]

[content_type.ui]
icon = "shopping-bag"
color = "#8b5cf6"
list_fields = ["title", "price", "status", "inventory"]

[fields.title]
type = "text"
required = true

[fields.description]
type = "richtext"

[fields.cover_image]
type = "url"

# 自动注入：status, published_at, slug, price, currency, inventory
# 自动路由：POST /cms/products/{id}/purchase
```

**订单（Order）：**

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
```

**品牌介绍（Brand）：**

```toml
[content_type]
name = "Brand"
singular = "brand"
plural = "brands"
table = "brands"
kind = "single"
implements = ["publishable", "slugable"]

[content_type.ui]
icon = "heart"
color = "#ec4899"

[fields.title]
type = "text"

[fields.story]
type = "richtext"

[fields.logo]
type = "url"

# 自动注入：status, published_at, slug
```

### API 路由汇总

```
Product:
  GET    /api/v1/cms/products              # 列表（publishable 过滤只返回 published）
  POST   /api/v1/cms/products              # 创建（自动 slug + published_at + inventory 默认值）
  GET    /api/v1/cms/products/{slug}        # 详情
  PUT    /api/v1/cms/products/{slug}        # 更新（自动 published_at 联动）
  DELETE /api/v1/cms/products/{slug}        # 删除
  POST   /api/v1/cms/products/{id}/purchase # 购买（purchasable 协议注入）

Order:
  GET    /api/v1/cms/orders                # 列表
  POST   /api/v1/cms/orders                # 创建
  GET    /api/v1/cms/orders/{id}            # 详情
  PUT    /api/v1/cms/orders/{id}            # 更新
  DELETE /api/v1/cms/orders/{id}            # 删除

Brand:
  GET    /api/v1/cms/brand                  # 获取唯一记录（Single Type）
  PUT    /api/v1/cms/brand                  # 更新（自动 slug + published_at）
```

## 十四、内置表复用 Protocol

### 设计原则

内置表（Blog/Pages/Media）和自定义 Content Type 走**同一套 Protocol 行为引擎**，消除代码重复。

关键区别：

| | 内置 CT (`builtin=true`) | 自定义 CT |
|---|---|---|
| 字段定义 | **显式写全**（含 Protocol 字段） | Protocol 字段可省略，自动注入 |
| 建表方式 | migration SQL 文件（静态，版本控制） | `repo.migrate()` 动态 ALTER TABLE |
| 字段变更 | 新版本 migration | 运行时自动 |
| Protocol 行为 | 统一走 Protocol 引擎 | 统一走 Protocol 引擎 |
| 可删除 | 否（可通过 `BUILTIN_BLOG=false` 禁用） | 是 |
| Protocol 字段检查 | 注册时验证字段存在且类型匹配 | 注册时验证 + 自动注入缺失字段 |

### 内置表定义（编译嵌入）

内置 Content Type 的 TOML 由系统编译嵌入（`include_str!`），字段全部显式写出：

**Post：**

```toml
# 内置 Post — 字段写全，包括 Protocol 要求的字段
[content_type]
name = "Post"
singular = "post"
plural = "posts"
table = "posts"
implements = ["publishable", "slugable", "ownable", "taggable", "searchable"]
builtin = true

[content_type.ui]
icon = "file-text"
color = "#3b82f6"
list_fields = ["title", "status", "published_at", "author_id"]

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

# 以下字段满足 Protocol 要求，显式定义
[fields.status]
type = "select"
options = ["draft", "published", "archived"]
default = "draft"

[fields.published_at]
type = "datetime"

[fields.slug]
type = "text"
unique = true

[fields.author_id]
type = "relation"
target = "users"
```

**Page：**

```toml
[content_type]
name = "Page"
singular = "page"
plural = "pages"
table = "pages"
implements = ["publishable", "slugable", "ownable", "versionable"]
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

[fields.published_at]
type = "datetime"

[fields.author_id]
type = "relation"
target = "users"

[fields.meta_title]
type = "text"

[fields.meta_description]
type = "text"

[fields.og_image]
type = "url"

[fields.cover_image]
type = "url"
```

### 内置 Protocol 扩展

为内置表新增的 Protocol：

| Protocol | 说明 | 自动注入字段 | 适用 |
|----------|------|-------------|------|
| `searchable` | 全文搜索集成（Tantivy） | 无 | Post |
| `commentable` | 评论系统挂载 | 无 | Post |
| `blockable` | Block 编辑器支持 | 无 | Page |

### 行为统一

之前内置表的行为硬编码在 service 层：

```
之前：
  post_service::create_post()
    → 手动 slugify(title)           # slugable 行为
    → 手动 set published_at          # publishable 行为
    → 手动 set author_id             # ownable 行为
    → 手动 index to tantivy          # searchable 行为
    → 手动 emit event

  content_type handler::do_create()
    → 泛型创建，无上述行为
```

之后统一走 Protocol 引擎：

```
之后：
  Protocol Engine::before_create(ct, data)
    → slugable.on_create → 自动 slug
    → publishable.on_create → 自动 published_at
    → ownable.on_create → 自动 author_id
    → searchable.on_create → 自动索引

  Protocol Engine::after_create(ct, record)
    → commentable.on_created → 注册评论关联
    → event.emit("content.created")

  # 内置 Post 和自定义 Product 走完全相同的代码路径
```

### 架构统一全景

```
┌─────────────────────────────────────────────────────────┐
│                    Protocol Engine                       │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐    │
│  │ publishable │  │  slugable   │  │ searchable  │    │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘    │
│         │                │                │            │
│  ┌──────┴──────┐  ┌──────┴──────┐  ┌──────┴──────┐    │
│  │  ownable    │  │ commentable │  │ purchasable │    │
│  └─────────────┘  └─────────────┘  └─────────────┘    │
└──────────┬──────────────────────────────────┬──────────┘
           │                                  │
    ┌──────┴──────┐                    ┌──────┴──────┐
    │  内置 CT     │                    │  自定义 CT   │
    │ (builtin)   │                    │             │
    ├─────────────┤                    ├─────────────┤
    │ Post        │                    │ Product     │
    │ Page        │                    │ Order       │
    │ Media       │                    │ Course      │
    └─────────────┘                    └─────────────┘
```

### 迁移策略

不建议一开始就统一。分阶段迁移：

| 阶段 | 内容 | 风险 |
|------|------|------|
| **Phase 1** | Protocol 引擎做好，只用于自定义 CT | 低 |
| **Phase 2** | 内置 CT 加 `implements` 声明，注册时验证字段 | 低（只读检查） |
| **Phase 3** | 内置 CT 的 service 层逐步替换为 Protocol 行为 | 中（逐步替换） |
| **Phase 4** | `post_service` 等硬编码逻辑完全移除 | 高（全面验证） |

每个阶段独立验证，出问题可以回退。

### "插件→原生晋升" 路径

内置表复用 Protocol 后，插件到原生的晋升更顺滑：

```
1. JS 插件定义 purchasable Protocol + Product CT
2. 用户使用，验证需求
3. 用 Rust 重写 purchasable 为内置 Protocol
4. Product CT 的 TOML 不用改，只是 Protocol 来源变了
5. 未来如果 Product 足够热门，升级为 builtin CT
   → 字段写入 migration SQL
   → 加 builtin = true
   → 走原生 handler（更高性能）
```

## 十五、实现路线

### Phase 1 — `__meta` 列 + 协议声明 + 字段自动注入 + 注册时检查

**改动范围：**
- `protocol.rs`（新文件）— 协议定义注册表 + 字段注入 + 合并检查
- `schema.rs` — `ContentTypeSchema` 新增 `implements` 字段 + `ui` 配置
- `content_type.rs` — `register()` 增加协议检查 + 字段合并逻辑
- `repository.rs` — 自动建表时添加 `__meta` 列

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
- 权限检查：插件只能访问自己的命名空间

**预计：** 2-3 天

### Phase 4 — UI hints + Admin 前端消费

**改动范围：**
- Admin 前端读取 `ui` 配置，个性化列表/详情页
- 图标选择器、颜色选择器、字段排序配置
- 协议字段在 Admin 中标记来源（内置/插件/自定义）

**预计：** 2-3 天

## 十六、与竞品对比

| 系统 | 类似机制 | 差异 |
|------|---------|------|
| **Strapi** | Component（可复用字段组） | 无协议/行为抽象，只是数据复用 |
| **Payload** | Field-level Hooks | 有行为但没有协议声明 |
| **WordPress** | Post Type Supports（`supports => ['title','editor']`） | 最接近，但是写死的选项，不可扩展 |
| **Directus** | None | 无类似机制 |
| **Notion** | Block 嵌套组合 | 有组合但无行为抽象 |
| **本系统** | Protocol（声明式 + 字段自动注入 + 行为注入 + 插件可扩展） | 最灵活，第三方可注册自定义协议 |

WordPress 的 `supports` 是最接近的设计：

```php
register_post_type('book', [
    'supports' => ['title', 'editor', 'thumbnail', 'custom-fields'],
]);
```

我们的 `implements` 比 WordPress `supports` 强在：

1. **字段约束检查** — 不满足就报错，WordPress 不会
2. **字段自动注入** — 协议要求的字段不用手写，WordPress 需要手写
3. **自动行为注入** — publishable 自动填时间，WordPress 需要插件
4. **路由自动注入** — purchasable 自动注册购买 API
5. **插件可扩展** — 第三方插件可以注册自定义协议，WordPress `supports` 是硬编码
