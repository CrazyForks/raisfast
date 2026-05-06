# Content Type 开发指南

## 概述

Content Type（内容类型）是 raisfast 的核心 CMS 机制，借鉴了 Strapi v5 Content Type Builder 和 WordPress Custom Post Types 的设计理念。每个 Content Type 由一个 TOML 文件定义，系统启动时自动加载、建表、注册 REST API。

Content Type 与 Plugin 完全独立，互不依赖。

## 架构

```
TOML 文件 (content_types/*.toml)
       ↓ 启动加载
ContentTypeRegistry (ArcSwap 无锁热加载)
       ↓ 注册时缓存
  ├─ cache_select_columns()  — 预计算 SELECT 列
  └─ cache_rules()           — 预编译 API Rule 为 AST
       ↓ 请求到来
Handler (动态路由分发)
       ↓ Rule SQL 编译
Repository (动态 SQL 构建)
       ↓ 参数化查询
SQLite / PostgreSQL
```

### 核心模块

| 模块 | 文件 | 职责 |
|------|------|------|
| Registry | `src/content_type.rs` | ArcSwap 无锁热加载注册表 |
| Schema | `src/content_type/schema.rs` | 数据结构 + TOML 解析 |
| Handler | `src/content_type/handler.rs` | HTTP 处理 + 路由注册 + 缓存 |
| Repository | `src/content_type/repository.rs` | 动态 SQL CRUD |
| Rule Engine | `src/content_type/rule_engine.rs` | 表达式解析 → SQL 编译 + 运行时求值 |
| Migration | `src/content_type/migration.rs` | 自动建表 / ALTER TABLE |
| Validation | `src/content_type/validation.rs` | 字段类型/必填/唯一/枚举/范围/正则校验 |
| Resolver | `src/content_type/resolver.rs` | 批量关联字段填充（populate） |
| CLI | `src/cli/ct_cmd.rs` | `ct new` / `ct check` |

## TOML Schema 定义

### 最小示例

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
description = "商品"
draft_publish = false
implements = ["ownable", "timestampable"]

[fields.name]
type = "text"
required = true
max_length = 200
label = "商品名称"

[api.list]
access = "public"

[api.get]
access = "public"

[api.create]
access = "admin"

[api.update]
access = "admin"

[api.delete]
access = "admin"
```

### `[content_type]` 顶层字段

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `name` | string | 必填 | 显示名称（如 "Product"） |
| `singular` | string | 必填 | 单数标识，用于 API 路径和注册 key |
| `plural` | string | 必填 | 复数标识，用于 API 路径 |
| `table` | string | 必填 | 数据库表名 |
| `description` | string | `""` | 描述 |
| `draft_publish` | bool | `false` | 启用 draft/published/archived 状态流 |
| `slug_field` | string | — | 从哪个字段自动生成 slug |
| `implements` | string[] | `[]` | 声明实现的 Protocol 列表（见 Protocols 章节） |
| `indexes` | IndexDef[] | `[]` | 索引定义（见下方） |
| `list_view` | object | — | 管理列表配置 |
| `api` | object | — | API 访问控制配置 |

### 字段类型（17 种）

| 类型 | SQL 类型 | 说明 |
|------|----------|------|
| `text` | TEXT | 纯文本 |
| `richtext` | TEXT | 富文本 / HTML |
| `integer` | INTEGER | 整数 |
| `bigint` | INTEGER | 大整数 |
| `decimal` | REAL | 精确小数 |
| `float` | REAL | 浮点数 |
| `boolean` | BOOLEAN | 布尔（存储 0/1） |
| `date` | TEXT | ISO 8601 日期 |
| `datetime` | TEXT | ISO 8601 日期时间 |
| `time` | TEXT | ISO 8601 时间 |
| `email` | TEXT | 邮箱地址 |
| `password` | TEXT | 密码（哈希） |
| `enum` | TEXT | 枚举值 |
| `uid` | TEXT | 自动生成 slug/UUID |
| `json` | TEXT | 任意 JSON |
| `media` | TEXT | 文件附件（URL） |
| `relation` | TEXT | 关联关系（FK ID） |

### `[fields.XXX]` 字段配置

| 字段 | 类型 | 说明 |
|------|------|------|
| `type` | string | 必填，上面 17 种之一 |
| `required` | bool | 必填且非空 |
| `unique` | bool | 唯一约束 |
| `default` | any | 默认值 |
| `private` | bool | 私有字段，公开 API (`/cms/`) 隐藏，admin API (`/admin/cms/`) 可见 |
| `immutable` | bool | 创建后不可修改 |
| `label` | string | Admin UI 显示标签 |
| `description` | string | 字段描述 |
| `max_length` | int | 最大字符串长度 |
| `min` / `max` | float | 数值范围 |
| `pattern` | string | 正则校验 |
| `enum_values` | string[] | enum 类型的可选值 |
| `relation_type` | string | 关联类型（见下） |
| `target` | string | 关联目标表名 |
| `through` | string | many_to_many 中间表名 |
| `foreign_key` | string | 外键列名（默认 `{field}_id`） |
| `accept` | string[] | media 类型允许的 MIME 类型 |
| `max_count` | int | media 最大文件数 |

### 关联类型（6 种）

| 类型 | 说明 |
|------|------|
| `one_to_one` | 源表 FK |
| `one_to_many` | 目标表 FK（反向查询） |
| `many_to_one` | 源表 FK |
| `many_to_many` | 中间表 |
| `one_way` | 源表 FK，无反向链接 |
| `many_way` | 源表 FK，无反向链接 |

### `[[indexes]]` 索引定义

```toml
[[indexes]]
fields = ["slug"]
unique = true

[[indexes]]
fields = ["status", "created_at"]
```

### `[list_view]` 管理列表配置

```toml
[list_view]
default_sort = "created_at:desc"
columns = ["title", "status", "created_at"]
```

### `[api.XXX]` API 访问控制

每个端点（list / get / create / update / delete）可独立配置：

```toml
[api.list]
access = "public"      # none / public / member / admin
cache = true
filter = 'status = "published"'
filter_auth = 'author_id = @request.auth.id'
fields = ["title", "slug", "status", "created_at"]  # 返回字段白名单（可选，默认返回全部非 private 字段）

[api.get]
access = "public"
cache = true
filter = 'status = "published"'
fields = ["title", "slug", "content", "status", "created_at"]
filter_auth = 'author_id = @request.auth.id'

[api.get]
access = "public"
cache = true
filter = 'status = "published"'

[api.create]
access = "member"

[api.update]
access = "member"
filter = 'author_id = @request.auth.id'

[api.delete]
access = "admin"
```

每个端点的完整配置项：

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `access` | string | list/get=`public`, create/update=`member`, delete=`admin` | 访问级别 |
| `cache` | bool | list/get=`false`, create/update/delete=`true` | 是否缓存响应 |
| `filter` | string | — | 对所有请求生效的行级过滤表达式 |
| `filter_auth` | string | — | 仅已认证用户的额外过滤（与 filter OR） |
| `fields` | string[] | — | list/get 端点返回字段白名单（空=全部非 private 字段），系统字段（id/status/created_at 等）始终返回 |

| 访问级别 | 说明 |
|----------|------|
| `none` | 完全禁止 |
| `public` | 无需认证 |
| `member` | 任何已认证用户 |
| `admin` | 需要 admin 角色 |

> `filter` 和 `filter_auth` 的完整语法见下方 [API Rule 表达式引擎](#api-rule-表达式引擎) 章节。

## API Rule 表达式引擎

PocketBase 风格的表达式级访问控制，支持编译为 SQL WHERE 或运行时求值。

### 什么是 API Rule

API Rule 是在 `access` 权限检查之后、数据库查询之前执行的**行级过滤**机制。它让你可以精确控制哪些记录对哪些用户可见、谁可以创建/修改/删除记录。

每个 API 端点可配置两个 rule：
- **`filter`** — 对所有通过 access 检查的请求生效
- **`filter_auth`** — 仅对已认证用户额外生效（与 filter 取 OR）

### Rule 在 TOML 中的位置

```toml
[api.list]
access = "public"
cache = true
filter = 'status = "published"'
filter_auth = 'author_id = @request.auth.id'

[api.get]
access = "public"
cache = true
filter = 'status = "published"'

[api.create]
access = "member"
filter = 'author_id = @request.auth.id'

[api.update]
access = "member"
filter = 'author_id = @request.auth.id'

[api.delete]
access = "admin"
```

### 执行流程

```
请求到来
  ↓
access 检查（none/public/member/admin）
  ↓ 通过
API Rule 求值
  ├─ filter 编译为 SQL WHERE 子句（附加到 SELECT/UPDATE/DELETE）
  ├─ filter_auth 编译为 SQL WHERE 子句（与 filter OR）
  └─ 未认证用户只应用 filter，已认证用户应用 (filter OR filter_auth)
  ↓
数据库查询
  ↓
对单条记录（get/update/delete）：额外运行时求值（检查 @request.body 等 SQL 无法表达的条件）
  ↓
返回结果
```

### 语法参考

#### 表达式基础

```
author_id = @request.auth.id                          # 字段 = 认证用户 ID
@request.auth.role = "admin"                          # 角色
status = "published"                                  # 字段比较
title ~ "%rust%"                                      # LIKE
title !~ "%spam%"                                     # NOT LIKE
tags:length > 0                                       # 长度
created_at > @now                                     # 当前时间
title:isset                                           # IS NOT NULL
price >= 100 && price <= 500                          # 范围
status = "published" || author_id = @request.auth.id  # OR
(status = "published" || featured = 1) && stock > 0   # 括号分组
```

#### 运算符

| 运算符 | 含义 | 示例 |
|--------|------|------|
| `=` `!=` `>` `>=` `<` `<=` | 比较 | `price > 100` |
| `~` | LIKE 模糊匹配 | `title ~ "%rust%"` |
| `!~` | NOT LIKE | `title !~ "%spam%"` |
| `:isset` | IS NOT NULL | `avatar:isset` |
| `:length` | 字符串/数组长度 | `tags:length > 0` |
| `&&` | AND | `a = 1 && b = 2` |
| `\|\|` | OR | `a = 1 \|\| b = 2` |
| `( )` | 分组 | `(a \|\| b) && c = 3` |

#### 特殊变量

| 变量 | 含义 | 适用阶段 |
|------|------|----------|
| `@request.auth.id` | 当前认证用户的 ID | SQL + 运行时 |
| `@request.auth.role` | 当前认证用户的角色 | SQL + 运行时 |
| `@request.body.X` | 请求体中的字段值 | 仅运行时 |
| `@request.query.X` | URL 查询参数值 | 仅运行时 |
| `@now` | 当前时间（ISO 8601） | SQL + 运行时 |
| `@table.X.Y` | 跨表引用 | 仅运行时 |

> `@request.body.X` 和 `@request.query.X` 无法编译为 SQL，仅在运行时求值阶段对单条记录检查。

#### 值类型

| 类型 | 示例 |
|------|------|
| 字符串 | `"published"`, `"admin"` |
| 数字 | `100`, `3.14` |
| 布尔 | `true`, `false` |
| Null | `null` |

### 实际场景示例

#### 场景 1：博客文章 — 公开已发布 + 作者可见自己的草稿

```toml
[api.list]
access = "public"
cache = true
filter = 'status = "published"'
filter_auth = 'status = "published" || author_id = @request.auth.id'

[api.get]
access = "public"
cache = true
filter = 'status = "published"'
filter_auth = 'status = "published" || author_id = @request.auth.id'
```

效果：
- 游客只能看到 `status = "published"` 的文章
- 作者登录后还能看到自己的草稿（`author_id = 自己的 ID`）

#### 场景 2：商品 — 仅展示有库存的已发布商品

```toml
[api.list]
access = "public"
cache = true
filter = 'status = "published" && stock > 0'

[api.get]
access = "public"
cache = true
filter = 'status = "published" && stock > 0'
```

#### 场景 3：用户只能修改自己创建的记录

```toml
[api.update]
access = "member"
filter = 'author_id = @request.auth.id'

[api.delete]
access = "member"
filter = 'author_id = @request.auth.id'
```

#### 场景 4：创建时自动校验字段值

```toml
[api.create]
access = "member"
filter = '@request.body.price >= 0 && @request.body.stock >= 0'
```

> `@request.body` 只在运行时求值，不影响 SQL 查询性能。

#### 场景 5：管理员可看所有，普通用户只看自己租户的

```toml
[api.list]
access = "member"
filter = 'tenant_id = @request.body.tenant_id'
filter_auth = '@request.auth.role = "admin"'
```

#### 场景 6：draft_publish=false 的表（没有 status 列）

```toml
[content_type]
draft_publish = false
# 注意：filter 不能引用 status，因为表中没有 status 列

[api.list]
access = "public"
# 不要写 filter = 'status = "published"'，会报错
```

### filter + filter_auth 组合逻辑

| 用户状态 | 应用的条件 |
|----------|------------|
| 未认证 | 仅 `filter` |
| 已认证（普通用户） | `(filter) OR (filter_auth)` |
| 已认证（admin + 无 X-Tenant-ID） | 无 filter（管理员看所有） |
| 已认证（admin + X-Tenant-ID） | 无 filter（按租户看） |

> 管理 API（`/admin/cms/*`）**绕过所有 API Rule**，不受 filter/filter_auth 限制。

### 环境变量配置

API Rule 引擎的所有 SQL 函数和操作符都可通过环境变量自定义，便于适配不同数据库：

```env
RULE_PREFIX_AUTH_ID=@request.auth.id
RULE_PREFIX_AUTH_ROLE=@request.auth.role
RULE_PREFIX_REQUEST_BODY=@request.body.
RULE_PREFIX_REQUEST_QUERY=@request.query.
RULE_PREFIX_NOW=@now
RULE_PREFIX_CROSS_TABLE=@table.
RULE_SQL_NOW_FN=datetime('now')       # PostgreSQL: NOW()
RULE_SQL_ISSET_OP=IS NOT NULL         # PostgreSQL: IS NOT NULL
RULE_SQL_LENGTH_FN=LENGTH             # PostgreSQL: CHAR_LENGTH
```

## REST API

系统启动时自动为每个 Content Type 注册以下路由：

### 公开 API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/cms/{plural}` | 分页列表（受 API Rule 过滤） |
| POST | `/api/v1/cms/{plural}` | 创建 |
| GET | `/api/v1/cms/{plural}/{id_or_slug}` | 获取（支持 ID 或 slug） |
| PUT | `/api/v1/cms/{plural}/{id_or_slug}` | 更新 |
| DELETE | `/api/v1/cms/{plural}/{id_or_slug}` | 删除 |

### 管理 API（绕过 API Rule）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/admin/cms/{plural}` | 管理列表（所有状态） |
| GET | `/api/v1/admin/cms/{plural}/{id_or_slug}` | 管理详情 |

### Schema 管理 API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/admin/content-types` | 列出所有 schema |
| GET | `/api/v1/admin/content-types/{singular}` | 获取单个 schema |
| POST | `/api/v1/admin/content-types` | 创建 CT（校验 → 写 TOML → 建表 → 注册） |
| PUT | `/api/v1/admin/content-types/{singular}` | 更新 CT（增量合并字段 → ALTER TABLE → 重注册） |
| DELETE | `/api/v1/admin/content-types/{singular}` | 删除 CT（删 TOML + 注销，**不删库表**） |

### 列表查询参数

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `page` | int | 1 | 页码 |
| `page_size` | int | 20 | 每页条数（上限由 `CMS_MAX_PAGE_SIZE` 控制） |
| `sort` | string | CT 默认 | 排序（如 `title:asc,created_at:desc`） |
| `status` | string | `"published"` | 状态过滤（仅 draft_publish 时） |
| `search` | string | — | 全文搜索 |
| `include` | string | — | 逗号分隔的关联字段（populate） |
| `skip_total` | bool | false | 跳过 COUNT(*)，返回 total=-1 |
| 其他 | — | — | 任意字段名作为等值过滤 |

### 版本管理 API（仅 `implements = ["versionable"]`）

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions` | 修订列表 |
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_id}` | 获取修订 |
| POST | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_id}/restore` | 恢复修订 |
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b}` | 两个修订对比 |

## 自动 Migration

系统启动时对每个 schema 执行：

1. **无表** → `CREATE TABLE IF NOT EXISTS`，包含：
   - `id TEXT PRIMARY KEY`（UUID v7）
   - `tenant_id TEXT NOT NULL DEFAULT 'default'`
   - 所有非关联字段的对应 SQL 类型
   - ManyToOne / OneToOne 关联的外键列
   - 系统列：`status`/`published_at`（draft_publish）、`created_at`/`updated_at`（timestampable protocol）、`deleted_at`（soft_deletable protocol）

2. **有表缺列** → `ALTER TABLE ADD COLUMN`（仅添加，**永不删列或改类型**）

3. **ManyToMany** → 自动创建中间表（带复合主键 + CASCADE）

4. **索引** → `CREATE UNIQUE INDEX`（unique 字段 + `[[indexes]]` 定义）

## Protocols（协议系统）

Protocol 是 Content Type 的可组合能力声明，通过 `[content_type]` 的 `implements` 字段启用：

```toml
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"
implements = ["ownable", "timestampable", "soft_deletable", "versionable"]
```

### 内置协议（5 种）

| Protocol | 自动注入列 | 行为 |
|----------|-----------|------|
| `ownable` | `created_by` TEXT, `updated_by` TEXT | 创建时注入 `created_by` + `updated_by`，更新时注入 `updated_by` |
| `timestampable` | `created_at` TEXT, `updated_at` TEXT | 创建时注入 `created_at` + `updated_at`，更新时注入 `updated_at` |
| `soft_deletable` | `deleted_at` TEXT, `deleted_by` TEXT | 删除时 UPDATE 而非 DELETE，查询自动过滤 `deleted_at IS NULL` |
| `versionable` | `version` INTEGER | 更新时自动保存历史修订到 `content_revisions` 表 |
| `cacheable` | 无 | 预留：读写时触发缓存失效事件（当前为占位符） |

### `implements` 的实际效果

| 效果 | 说明 |
|------|------|
| **Migration** | `soft_deletable` 触发建表时添加 `deleted_at`/`deleted_by` 列 |
| **查询过滤** | `soft_deletable` 自动在 SELECT/UPDATE/DELETE 中追加 `WHERE deleted_at IS NULL` |
| **数据注入** | Aspect 引擎在 `before_create`/`before_update` 时自动注入系统列 |
| **修订历史** | `versionable` 在 `after_update` 时异步保存快照 |
| **TypeScript 生成** | `ct types` 命令根据 `implements` 生成对应的接口字段 |
| **元数据存储** | `__meta` 列记录 `{"protocols": ["ownable", ...]}` |

### 注意事项

- `ownable` 和 `timestampable` 的 Aspect 对**所有** Content Type 生效（`TargetMatcher::All`），无论是否声明 `implements`
- `soft_deletable` 的**建列和查询过滤**仅对声明了 `implements = ["soft_deletable"]` 的 Content Type 生效
- `versionable` 通过 `implements = ["versionable"]` 启用

### 示例：带软删除和版本管理的文章

```toml
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"
draft_publish = true
slug_field = "title"
implements = ["ownable", "timestampable", "soft_deletable", "versionable"]

[fields.title]
type = "text"
required = true
max_length = 200

[fields.content]
type = "richtext"
required = true

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
foreign_key = "author_id"

[api.list]
access = "public"
cache = true
filter = 'status = "published"'

[api.get]
access = "public"
cache = true
filter = 'status = "published"'

[api.create]
access = "member"

[api.update]
access = "member"
filter = 'created_by = @request.auth.id'

[api.delete]
access = "admin"
```

## 缓存

- 列表和详情响应缓存在内存 `DashMap` 中
- 缓存 key 由查询参数哈希生成
- TTL 由 `CMS_CACHE_TTL` 环境变量控制（默认 30 秒）
- 对该 Content Type 的任何写操作自动清除缓存

## Plugin Hook 集成

Handler 在内容生命周期关键点派发 Plugin 钩子：

| 钩子 | 类型 | 说明 |
|------|------|------|
| `ContentCreating` | filter | 可修改创建数据 |
| `ContentCreated` | action | 创建完成副作用 |
| `ContentUpdating` | filter | 可修改更新数据 |
| `ContentUpdated` | action | 更新完成副作用 |
| `ContentDeleted` | action | 删除完成副作用 |
| `ContentViewed` | action | 内容被浏览 |

## CLI 命令

```bash
# 创建新 Content Type TOML
raisfast ct new product

# 校验 TOML 文件
raisfast ct check                    # 校验默认目录
raisfast ct check ./content_types    # 校验指定目录
```

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `CONTENT_TYPE_DIR` | `./extensions/content_types` | TOML 文件目录 |
| `CMS_CACHE_TTL` | `30` | 列表缓存 TTL（秒） |
| `CMS_MAX_PAGE_SIZE` | `100` | 列表单页最大条数 |

## 完整示例：电商商品

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
description = "商品"
draft_publish = true
slug_field = "name"
implements = ["ownable", "timestampable"]

[fields.name]
type = "text"
required = true
max_length = 200
label = "商品名称"

[fields.slug]
type = "uid"
target_field = "name"
unique = true
label = "URL 标识"

[fields.description]
type = "richtext"
label = "商品描述"

[fields.price]
type = "decimal"
required = true
min = 0
label = "价格"

[fields.stock]
type = "integer"
required = true
min = 0
default = 0
label = "库存"

[fields.sku]
type = "text"
unique = true
max_length = 50
label = "SKU"

[fields.images]
type = "media"
accept = ["image/*"]
max_count = 10
label = "商品图片"

[fields.category]
type = "relation"
relation_type = "many_to_one"
target = "product_categories"
foreign_key = "category_id"
label = "分类"

[fields.featured]
type = "boolean"
default = false
label = "推荐商品"

[[indexes]]
fields = ["slug"]
unique = true

[[indexes]]
fields = ["status", "created_at"]

[list_view]
default_sort = "created_at:desc"
columns = ["name", "price", "stock", "status", "featured", "created_at"]

[api.list]
access = "public"
cache = true

[api.get]
access = "public"
cache = true

[api.create]
access = "admin"

[api.update]
access = "admin"

[api.delete]
access = "admin"
```
