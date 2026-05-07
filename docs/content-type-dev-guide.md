# Content Type 开发指南

## 概述

Content Type（内容类型）是 raisfast 的核心 CMS 机制，借鉴了 Strapi v5 Content Type Builder 和 PocketBase 的设计理念。每个 Content Type 由一个 TOML 文件定义，系统启动时自动加载、建表、注册 REST API。

Content Type 与 Plugin 完全独立，互不依赖。

## 架构

```
TOML 文件 (extensions/content_types/*.toml)
       ↓ 启动加载
ContentTypeRegistry (ArcSwap 无锁热加载)
       ↓ 注册时缓存
  ├─ cache_protocol_columns()  — 协议列名 + 去重
  ├─ cache_declaration()       — 协议声明聚合 + apply_config
  ├─ cache_select_columns()    — 预计算 SELECT 列
  └─ cache_rules()             — 预编译 API Rule 为 AST
       ↓ 请求到来
Handler (动态路由分发)
       ↓ Aspect 注入 + Rule SQL 编译
Repository (动态 SQL 构建)
       ↓ 参数化查询
SQLite / PostgreSQL
```

### 分层架构

```
Handler（薄层）
  ├─ 提取参数、构造 SaveContext
  ├─ 调用 Aspect 引擎注入系统列
  ├─ 调用 Service / Repository
  └─ 返回 JSON 响应（code: 0）
       ↓
Service / Repository
  ├─ 动态 SQL 构建（基于 ProtocolDeclaration）
  ├─ 协议声明驱动：query_filters / delete_strategy / lock_column / default_sort
  └─ 多租户：ct.implements_protocol("tenantable") 判断
       ↓
Protocol 系统
  ├─ ProtocolDeclaration（纯数据 struct，声明式效果）
  ├─ Aspect 引擎（AOP，命令式副作用）
  └─ ProtocolRegistry（inventory 自注册）
```

### 核心模块

| 模块 | 文件 | 职责 |
|------|------|------|
| Registry | `src/content_type.rs` | ArcSwap 无锁热加载注册表 |
| Schema | `src/content_type/schema.rs` | 数据结构 + TOML 解析 + ProtocolRef |
| Handler | `src/content_type/handler.rs` | HTTP 处理 + 路由注册 + DashMap 缓存 |
| Repository | `src/content_type/repository.rs` | 动态 SQL CRUD + meta JSON path 查询 |
| Rule Engine | `src/content_type/rule_engine.rs` | 表达式解析 → SQL 编译 + 运行时求值 |
| Migration | `src/content_type/migration.rs` | 自动建表 / ALTER TABLE（协议列动态注入） |
| Validation | `src/content_type/validation.rs` | 字段类型/必填/唯一/枚举/范围/正则校验 |
| Resolver | `src/content_type/resolver.rs` | 批量关联字段填充（populate） |
| CLI | `src/cli/ct_cmd.rs` | `ct new` / `ct check` |
| Protocols | `src/protocols.rs` + `src/protocols/*.rs` | 协议定义 + ProtocolDeclaration + inventory 自注册 |
| Aspects | `src/aspects.rs` | AOP 引擎 + Aspect trait + Pointcut 匹配 |

## TOML Schema 定义

### 最小示例

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"
description = "商品"
implements = ["ownable", "timestampable", "tenantable"]

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
| `kind` | string | `"collection"` | `collection`（多条记录）或 `single`（仅一条记录） |
| `description` | string | `""` | 描述 |
| `slug_field` | string | — | 从哪个字段自动生成 slug |
| `builtin` | bool | `false` | 内置 CT（不注入默认字段，字段全部显式定义） |
| `implements` | string[] / object[] | `[]` | 声明实现的 Protocol 列表（见 Protocols 章节） |
| `indexes` | IndexDef[] | `[]` | 索引定义（见下方） |
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
| `target_field` | string | uid 类型自动生成 slug 的源字段 |
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
| `fields` | string[] | — | list/get 端点返回字段白名单（空=全部非 private 字段），系统列（id + 协议列）始终返回 |

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
| POST | `/api/v1/admin/cms/{plural}` | 管理创建 |
| GET | `/api/v1/admin/cms/{plural}/{id_or_slug}` | 管理详情 |
| PUT | `/api/v1/admin/cms/{plural}/{id_or_slug}` | 管理更新 |
| DELETE | `/api/v1/admin/cms/{plural}/{id_or_slug}` | 管理删除 |

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
| `status` | string | — | 状态等值过滤（需 implements statusable） |
| `search` | string | — | 全文搜索 |
| `include` | string | — | 逗号分隔的关联字段（populate） |
| `skip_total` | bool | false | 跳过 COUNT(*)，返回 total=-1 |
| `{field_name}` | string | — | 任意字段名作为等值过滤 |
| `__meta.{path}` | string | — | meta JSON path 查询（如 `__meta.views=100`） |

> 排序优先级：用户 `?sort=` → 协议声明 `default_sort` → 无排序。

> `__meta.{path}` 查询需要 implements `metaable`，底层使用 `json_extract(__meta, '$.{path}') = '{value}'`。

### 版本管理 API（仅 `implements = ["versionable"]`）

协议通过 `register_routes()` 自注册以下路由：

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions` | 修订列表 |
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_id}` | 获取修订 |
| POST | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_id}/restore` | 恢复修订 |
| GET | `/api/v1/admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b}` | 两个修订对比 |

### JSON 响应格式

所有 handler 的 JSON 响应统一使用 `"code": 0` 表示成功：

```json
{
  "code": 0,
  "data": { ... }
}
```

## 自动 Migration

系统启动时对每个 schema 执行：

1. **无表** → `CREATE TABLE IF NOT EXISTS`，包含：
   - `id TEXT PRIMARY KEY`（UUID v7）
   - 所有非关联字段的对应 SQL 类型
   - ManyToOne / OneToOne 关联的外键列
   - 协议列（由 `ProtocolRegistry::columns_for()` 动态获取，如 `created_at`、`tenant_id`、`status`、`__meta` 等）

2. **有表缺列** → `ALTER TABLE ADD COLUMN`（仅添加，**永不删列或改类型**）

3. **ManyToMany** → 自动创建中间表（带复合主键 + CASCADE）

4. **索引** → `CREATE UNIQUE INDEX`（unique 字段 + `[[indexes]]` 定义）

> `tenant_id` 不再硬编码到所有表，改为 `implements = ["tenantable"]` 声明启用。

> `__meta` 不再硬编码到所有表，改为 `implements = ["metaable"]` 声明启用。

## Protocols（协议系统）

Protocol 是 Content Type 的可组合能力声明，通过 `[content_type]` 的 `implements` 字段启用：

```toml
# 简单语法
implements = ["ownable", "timestampable", "soft_deletable", "versionable"]

# 带配置语法
implements = [
  "ownable",
  "timestampable",
  "soft_deletable",
  { name = "sortable", field = "priority", direction = "desc" },
  { name = "statusable", values = "draft=1,published=10,archived=99", default = "1", mode = "numeric" },
]
```

### 协议架构

```
Protocol trait
  ├─ name(), description(), aspects(), columns(), behaviors()
  ├─ declaration() → ProtocolDeclaration（纯数据 struct）
  ├─ apply_config() → 用户配置应用到声明（如 sortable 的 field/direction）
  ├─ register_routes() → 协议自注册 API 路由（如 versionable 的 /revisions）
  └─ on_after_delete() → async hook（唯一非纯数据方法）

ProtocolDeclaration {
    query_filters: Vec<(String, String)>,   — 自动追加 WHERE 条件
    delete_strategy: DeleteStrategy,         — Soft / Hard
    snapshot_before_update: bool,            — 更新前快照
    revision_routes: bool,                   — 版本历史 API
    lock_column: Option<String>,             — 乐观锁列
    default_sort: Option<(String, SortDir)>, — 默认排序
    status_values / status_map / status_default / status_mode, — 状态配置
}

ProtocolRef enum (#[serde(untagged)])
  ├─ Simple(String)                           — "sortable"
  └─ WithConfig { name, config: HashMap }     — { name = "sortable", field = "priority" }
```

### 两个正交维度

每个协议将行为拆分为：

| 维度 | 实现方式 | 示例 |
|------|----------|------|
| **命令式**（注入值） | Aspect + Pointcut | `on_data_before_create` 注入 `created_at` |
| **声明式**（影响 SQL 行为） | `ProtocolDeclaration` 纯数据 | `query_filters` → WHERE、`default_sort` → ORDER BY |

### 协议注册

每个协议文件末尾使用 `register_protocol!` 宏自注册：

```rust
// src/protocols/sortable.rs 末尾
crate::register_protocol!(
    crate::protocols::sortable::SortableProtocol,
    crate::protocols::sortable::SortableProtocol
);
```

`lib.rs` 中一行完成所有注册：

```rust
protocol_registry.register_from_inventory();
```

### 内置协议（11 种）

| Protocol | 注入列 | behaviors | 说明 |
|----------|--------|-----------|------|
| `ownable` | `created_by`, `updated_by` | `track_owner` | 创建/更新时自动注入操作者 ID |
| `timestampable` | `created_at`, `updated_at` | `track_timestamps` | 创建/更新时自动注入时间戳 |
| `soft_deletable` | `deleted_at`, `deleted_by` | `soft_delete` | 删除时 UPDATE 而非 DELETE，查询自动过滤 `deleted_at IS NULL` |
| `versionable` | `version` | `versioning` | 更新时自动保存历史修订到 `content_revisions` 表，注册 `/revisions` 路由 |
| `lockable` | `lock_version` | `optimistic_lock` | 乐观锁，UPDATE WHERE lock_version = ? + SET lock_version += 1，冲突返回 409 |
| `sortable` | — | `sortable` | 默认排序（默认按 `created_at DESC`），支持配置 `field` / `direction` |
| `expirable` | `expires_at` | `expirable` | 过期时间列，列表查询自动过滤 `expires_at IS NULL OR expires_at > now` |
| `nestable` | `parent_id`, `depth`, `position` | `nestable` | 父子树形结构 |
| `statusable` | `status` | `statusable` | 可配置状态字段，支持字符串和数字映射两种存储模式 |
| `metaable` | `__meta` | `metaable` | 动态 JSON 元数据列，支持 `__meta.xxx` 查询参数 |
| `tenantable` | `tenant_id` | `tenantable` | 多租户隔离，自动按租户 ID 注入和过滤 |

### 协议配置（ProtocolRef WithConfig）

部分协议支持通过配置对象自定义行为：

#### sortable

```toml
implements = [{ name = "sortable", field = "priority", direction = "desc" }]
```

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `field` | `"created_at"` | 排序列名 |
| `direction` | `"desc"` | 排序方向：`asc` / `desc` |

#### statusable

```toml
# 字符串模式（默认）
implements = [{ name = "statusable", values = "draft,published,archived", default = "draft" }]

# 数字映射模式
implements = [{ name = "statusable", values = "draft=1,published=10,archived=99", default = "1", mode = "numeric" }]
```

| 配置项 | 默认值 | 说明 |
|--------|--------|------|
| `values` | — | 逗号分隔的状态值（数字模式用 `label=num`） |
| `default` | `"draft"` | 默认状态值 |
| `mode` | `"string"` | 存储模式：`string` / `numeric` |

> 数字映射模式下，API 层用字符串交互（`"draft"`），DB 层存数字（`1`）。

> 状态查询过滤不由协议处理（query_filters 是无条件的），由 API rule engine 控制：`[api.list] filter = 'status = "published"'`。

### `implements` 的实际效果

| 效果 | 说明 |
|------|------|
| **Migration** | 协议声明的列由 `ProtocolRegistry::columns_for()` 动态获取，自动加入 CREATE TABLE / ALTER TABLE |
| **查询过滤** | `ProtocolDeclaration.query_filters` 自动在 SELECT 中追加 WHERE 条件 |
| **数据注入** | Aspect 引擎在 `before_create` / `before_update` 时自动注入系统列 |
| **默认排序** | `ProtocolDeclaration.default_sort` 控制列表查询的 ORDER BY |
| **删除策略** | `ProtocolDeclaration.delete_strategy` 决定物理删除或软删除 |
| **乐观锁** | `ProtocolDeclaration.lock_column` 控制 UPDATE 的并发冲突检测 |
| **版本历史** | `versionable` 通过 `snapshot_before_update` + `revision_routes` 保存快照并注册 API |
| **修订历史** | `versionable` 在 `after_update` 时异步保存快照，`on_after_delete` 清理关联修订 |
| **多租户** | `tenantable` 判断改为 schema 级别（`ct.implements_protocol("tenantable")`），不再运行时检测 DB 列 |

### Aspect `is_protocol_column` 守卫

所有 Aspect 的 `on_data_before_create` / `on_data_before_update` 统一使用：

```rust
ctx.schema.as_ref().is_none_or(|s| s.is_protocol_column(COL_XXX))
```

- 有 schema → 只在该协议列真正需要创建时注入
- 无 schema → 放行（向后兼容单元测试）

### merge() 策略

多个协议的 `ProtocolDeclaration` 通过 `merge()` 聚合：

| 字段 | 策略 |
|------|------|
| `query_filters` | 累积（extend） |
| `delete_strategy` | Soft 优先 Hard |
| `snapshot_before_update` / `revision_routes` | OR |
| `lock_column` / `default_sort` | last-wins + warn on conflict |
| `status_*` | last-wins |

### 列冲突检测

`columns_for()` 对同名不同类型的列采用 **first-wins + warn**（不阻断启动），因为 sortable 可能用已有列排序。

### 注意事项

- `ownable` 和 `timestampable` 的 Aspect 对**所有** Content Type 生效（`TargetMatcher::All`），但只在 `is_protocol_column` 守卫通过时注入
- `soft_deletable` 的**建列和查询过滤**仅对声明了 `implements = ["soft_deletable"]` 的 Content Type 生效
- `versionable` 通过 `implements = ["versionable"]` 启用，额外注册 `/revisions` 路由
- `tenantable` 不再硬编码到所有表，必须显式声明
- `__meta` 不再硬编码到所有表，由 `metaable` 协议控制

### 示例：完整的博客文章

```toml
[content_type]
name = "Article"
singular = "article"
plural = "articles"
table = "articles"
slug_field = "title"
implements = [
  "ownable",
  "timestampable",
  "soft_deletable",
  "versionable",
  "lockable",
  { name = "sortable", field = "created_at", direction = "desc" },
  { name = "statusable", values = "draft,published,archived", default = "draft" },
  "tenantable",
]

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

[fields.author]
type = "relation"
relation_type = "many_to_one"
target = "users"
foreign_key = "author_id"

[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
through = "articles_tags"

[[indexes]]
fields = ["slug"]
unique = true

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

此配置将自动创建以下表结构：

```sql
CREATE TABLE articles (
    id TEXT PRIMARY KEY,
    -- 用户字段
    title TEXT NOT NULL,
    slug TEXT,
    content TEXT NOT NULL,
    author_id TEXT REFERENCES users(id),
    -- 协议列（自动注入）
    created_by TEXT,                    -- ownable
    updated_by TEXT,                    -- ownable
    created_at TEXT,                    -- timestampable
    updated_at TEXT,                    -- timestampable
    deleted_at TEXT,                    -- soft_deletable
    deleted_by TEXT,                    -- soft_deletable
    version INTEGER,                    -- versionable
    lock_version INTEGER,               -- lockable
    status TEXT,                        -- statusable
    tenant_id TEXT NOT NULL DEFAULT 'default'  -- tenantable
);

-- 中间表（many_to_many）
CREATE TABLE articles_tags (
    article_id TEXT NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
    tags_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (article_id, tags_id)
);

-- 版本修订表（versionable 自动使用）
-- content_revisions 表由系统预建

-- 索引
CREATE UNIQUE INDEX idx_articles_slug_unique ON articles(slug);
```

## 缓存

- 列表和详情响应缓存在内存 `DashMap` 中
- 缓存 key 由 `plural + 查询参数哈希` 生成
- TTL 由 `cms_cache_ttl_secs` 配置控制（默认 30 秒）
- 对该 Content Type 的任何写操作自动清除该 CT 的全部缓存

> 缓存由 handler 内置 DashMap TTL 处理，对应 `api.list.cache` / `api.get.cache` 配置。

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
slug_field = "name"
implements = [
  "ownable",
  "timestampable",
  "soft_deletable",
  "tenantable",
  "metaable",
  { name = "sortable", field = "created_at", direction = "desc" },
  { name = "statusable", values = "draft,published,archived", default = "draft" },
]

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

[api.list]
access = "public"
cache = true
filter = 'status = "published"'

[api.get]
access = "public"
cache = true
filter = 'status = "published"'

[api.create]
access = "admin"

[api.update]
access = "admin"

[api.delete]
access = "admin"
```
