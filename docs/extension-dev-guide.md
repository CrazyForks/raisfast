# Extension 开发手册

> 本手册指导 AI 或开发者从零创建一个完整的 rust-blog Extension。
> rust-blog 的 Extension = Content Type（数据模型）+ Plugin（业务逻辑），打包为一个独立模块。

---

## 目录

- [1. 概览](#1-概览)
- [2. Extension 目录结构](#2-extension-目录结构)
- [3. extension.toml — Extension 清单](#3-extensiontoml--extension-清单)
- [4. Content Type TOML — 数据模型](#4-content-type-toml--数据模型)
  - [4.1 content_type 头部](#41-content_type-头部)
  - [4.2 字段类型 (FieldType)](#42-字段类型-fieldtype)
  - [4.3 字段属性](#43-字段属性)
  - [4.4 relation 字段](#44-relation-字段)
  - [4.5 media 字段](#45-media-字段)
  - [4.6 uid 字段](#46-uid-字段)
  - [4.7 enum 字段](#47-enum-字段)
  - [4.8 indexes 索引](#48-indexes-索引)
  - [4.9 list_view 列表视图](#49-list_view-列表视图)
  - [4.10 api 访问控制](#410-api-访问控制)
  - [4.11 CMS 自动生成的列](#411-cms-自动生成的列)
  - [4.12 CMS 自动生成的 REST API](#412-cms-自动生成的-rest-api)
- [5. plugin/manifest.toml — Plugin 清单](#5-pluginmanifesttoml--plugin-清单)
  - [5.1 plugin 基本信息字段](#51-plugin-基本信息字段)
  - [5.2 permissions 权限声明](#52-permissions-权限声明)
  - [5.3 routes 路由声明](#53-routes-路由声明)
  - [5.4 hooks 钩子声明](#54-hooks-钩子声明)
  - [5.5 cron 定时任务](#55-cron-定时任务)
- [6. Plugin JS 代码编写](#6-plugin-js-代码编写)
  - [6.1 框架约定](#61-框架约定)
  - [6.2 工具函数（必须复制）](#62-工具函数必须复制)
  - [6.3 Host API](#63-host-api)
  - [6.4 Route Handler 编写](#64-route-handler-编写)
  - [6.5 Hook Handler 编写](#65-hook-handler-编写)
  - [6.6 数据库事务](#66-数据库事务)
  - [6.7 事件触发（emitEvent）](#67-事件触发emitevent)
  - [6.8 QuickJS 限制](#68-quickjs-限制)
  - [6.9 框架响应格式统一](#69-框架响应格式统一)
- [7. Plugin Lua 代码编写](#7-plugin-lua-代码编写)
- [8. API Rule 表达式语法](#8-api-rule-表达式语法)
- [9. 前端对接](#9-前端对接)
- [10. Extension 版本迁移](#10-extension-版本迁移)
- [11. 完整示例：Todo Extension](#11-完整示例-todo-extension)
- [12. 常见陷阱](#12-常见陷阱)

---

## 1. 概览

一个 Extension 由两部分组成：

| 部分 | 作用 | 必需 |
|------|------|------|
| **Content Type** | 声明数据模型（表结构），CMS 自动生成 CRUD API | 否 |
| **Plugin** | 编写自定义业务逻辑（自定义路由、Hook、定时任务） | 否 |

两者可以独立存在，但大多数扩展两者都需要。

**最小化 Extension** 只需要一个 `extension.toml`。最大的 Extension 可以包含：
- 多个 Content Type TOML（每文件一张表）
- 一个 Plugin（JS 或 Lua 运行时），包含自定义路由 + Hook + 定时任务

### 加载流程

```
Extension 目录 (extensions/my-ext/)
  ├── extension.toml           ← 入口
  ├── content_types/*.toml     ← 解析 → ContentTypeSchema → 自动迁移建表 → 注册 CRUD 路由
  └── plugin/
      ├── manifest.toml        ← 解析 → 注册 Plugin 路由 + Hook + Cron
      └── main.js              ← QuickJS 沙箱执行
```

---

## 2. Extension 目录结构

```
extensions/my-extension/
├── extension.toml                  # Extension 清单（必须）
├── content_types/                  # Content Type 定义目录
│   ├── todo.toml                   # 每个 .toml 定义一张表
│   └── todo_comment.toml           # 可以有多张表
└── plugin/                         # Plugin 目录（可选）
    ├── manifest.toml               # Plugin 清单
    └── main.js                     # JS 入口（或 main.lua）
```

**规则：**
- `extension.toml` 必须存在于 Extension 根目录
- `content_types/` 目录路径在 `extension.toml` 中指定，可以为空或省略
- `plugin/` 目录路径在 `extension.toml` 中指定，可以省略
- 一个 Extension 只能有一个 Plugin（JS 或 Lua 二选一）

---

## 3. extension.toml — Extension 清单

```toml
[extension]
# ── 必填 ──
id = "my-extension"                 # 全局唯一标识（kebab-case）
name = "My Extension"               # 显示名称
version = "1.0.0"                   # 语义化版本

# ── 可选 ──
description = "描述文本"
author = "Author Name"
license = "MIT"
homepage = "https://github.com/example/my-ext"

# Content Type 目录路径（相对于 extension 根目录）
# 省略或留空 = 无 Content Type
content_types = "content_types/"

# Plugin manifest 路径（相对于 extension 根目录）
# 省略或留空 = 无 Plugin
plugin = "plugin/manifest.toml"

# 依赖的其他 Extension（id → version range）
[extension.dependencies]
# forum = ">=0.1.0"
```

### 字段说明

| 字段 | 必填 | 说明 |
|------|------|------|
| `id` | 是 | 全局唯一，kebab-case 格式 |
| `name` | 是 | 显示名称 |
| `version` | 是 | 语义化版本 |
| `description` | 否 | 描述 |
| `author` | 否 | 作者 |
| `license` | 否 | 许可证 |
| `homepage` | 否 | 主页 URL |
| `content_types` | 否 | Content Type TOML 文件所在目录路径 |
| `plugin` | 否 | Plugin manifest.toml 文件路径 |

---

## 4. Content Type TOML — 数据模型

每个 Content Type TOML 文件定义一张数据库表和对应的 CRUD API。

### 4.1 content_type 头部

```toml
[content_type]
name = "Todo"                       # 显示名称
singular = "todo"                   # 单数标识，用于 API 路径和注册 key
plural = "todos"                    # 复数标识，用于 API 路径
table = "todos"                     # 数据库表名
description = "待办事项"            # 描述
draft_publish = false               # 是否启用草稿/发布状态流
slug_field = "title"                # 自动从哪个字段生成 slug（可选）
timestamps = true                   # 自动维护 created_at / updated_at（默认 true）
soft_delete = false                 # 软删除（添加 deleted_at 列）
```

| 字段 | 默认值 | 说明 |
|------|--------|------|
| `name` | — | 显示名称 |
| `singular` | — | 单数标识，如 `"todo"` |
| `plural` | — | 复数标识，如 `"todos"` |
| `table` | — | 数据库表名，如 `"todos"` |
| `description` | `""` | 描述 |
| `draft_publish` | `false` | `true` = 表有 `status` 列（draft/published/archived），`false` = 无 status 列 |
| `slug_field` | `None` | 设置后自动从该字段值生成 URL slug |
| `timestamps` | `true` | 自动添加 `created_at` / `updated_at` 列 |
| `soft_delete` | `false` | 自动添加 `deleted_at` 列，删除时更新而非物理删除 |

**重要：** `draft_publish = false` 的表**没有 `status` 列**。API Rule 和 Plugin 查询不能引用 `status`。

### 4.2 字段类型 (FieldType)

| type | SQL 类型 | 说明 |
|------|----------|------|
| `text` | `TEXT` | 短文本、字符串 |
| `richtext` | `TEXT` | 富文本（Markdown） |
| `integer` | `INTEGER` | 32 位整数 |
| `big_int` | `INTEGER` | 64 位整数 |
| `decimal` | `REAL` | 高精度小数 |
| `float` | `REAL` | 浮点数 |
| `boolean` | `INTEGER` | 布尔（0/1） |
| `date` | `TEXT` | 日期 (ISO 8601) |
| `datetime` | `TEXT` | 日期时间 (ISO 8601) |
| `time` | `TEXT` | 时间 |
| `email` | `TEXT` | 邮箱（带格式校验） |
| `password` | `TEXT` | 密码（自动 hash） |
| `enum` | `TEXT` | 枚举（需配合 `enum_values`） |
| `uid` | `TEXT` | 自动生成 URL 标识 |
| `json` | `TEXT` | JSON 对象 |
| `media` | `TEXT` | 媒体文件引用 |
| `relation` | — | 关联关系（不生成列，生成 foreign_key 列） |

### 4.3 字段属性

```toml
[fields.title]
type = "text"
required = true                     # 必填
unique = true                       # 唯一约束
max_length = 200                    # 最大长度（text/email/password）
min = 0                             # 最小值（数值类型）
max = 100                           # 最大值（数值类型）
default = "draft"                   # 默认值
auto_fill = "user_id"               # 自动填充（见下文）
private = true                      # 私有字段，不出现在 API 响应中
immutable = true                    # 创建后不可修改
label = "标题"                      # Admin UI 显示标签
description = "文章标题"            # 字段说明
pattern = "^[a-z]+$"                # 正则校验（text/email）
```

#### auto_fill 自动填充

| 值 | 说明 |
|----|------|
| `"user_id"` | 当前认证用户 ID |
| `"user_role"` | 当前认证用户角色 |
| `"current_tenant_id"` | 当前租户 ID |
| `"current_timestamp"` | 当前 ISO 8601 时间戳 |

`auto_fill` 优先级高于 `default` 和客户端传值。用于 `author_id` 等字段自动注入当前用户。

### 4.4 relation 字段

```toml
[fields.board]
type = "relation"
relation_type = "many_to_one"       # 关系类型
target = "forum_boards"             # 目标 content type 的 plural 名称
foreign_key = "board_id"            # 外键列名
required = true
label = "所属版块"
```

**relation_type 可选值：**

| 值 | 说明 | 生成的列 |
|----|------|----------|
| `one_to_one` | 一对一 | `foreign_key` 列 + UNIQUE |
| `one_to_many` | 一对多 | `foreign_key` 列（在 target 表） |
| `many_to_one` | 多对一 | `foreign_key` 列（在当前表） |
| `many_to_many` | 多对多 | 需指定 `through` 中间表 |
| `one_way` | 单向引用 | `foreign_key` 列 |
| `many_way` | 多向引用 | `foreign_key` 列 |

**many_to_many 示例：**

```toml
[fields.tags]
type = "relation"
relation_type = "many_to_many"
target = "tags"
through = "articles_tags"           # 中间表名
label = "标签"
```

**重要：** 字段名和数据库列名不同。例如 `board` 字段的列名是 `board_id`。CMS Handler 自动处理映射：
- API 响应用字段名（`board`）
- 前端 filter 用字段名（`?board=xxx`）
- Handler 自动映射为列名（`board_id`）

### 4.5 media 字段

```toml
[fields.cover_image]
type = "media"
accept = ["image/*"]                # 接受的 MIME 类型
max_count = 1                       # 最大文件数量（默认 1）
label = "封面图片"
```

存储为 JSON 字符串，包含文件路径和元信息。

### 4.6 uid 字段

```toml
[fields.slug]
type = "uid"
target_field = "title"              # 从哪个字段生成
unique = true
label = "URL 标识"
```

自动从 `target_field` 的值生成 URL 友好的 slug。

### 4.7 enum 字段

```toml
[fields.status]
type = "enum"
enum_values = ["draft", "published", "archived"]
default = "draft"
label = "状态"
```

### 4.8 indexes 索引

```toml
[[indexes]]
fields = ["slug"]
unique = true

[[indexes]]
fields = ["board_id", "is_pinned", "last_reply_at"]

[[indexes]]
fields = ["author_id", "created_at"]
```

- 使用 `[[indexes]]` 数组语法（每个索引一个 `[[indexes]]`）
- `fields` 是列名数组（用 foreign_key 列名，不是字段名）
- `unique = true` 表示唯一索引

### 4.9 list_view 列表视图

```toml
[list_view]
default_sort = "is_pinned:desc,last_reply_at:desc"
columns = ["title", "board", "author_id", "reply_count", "created_at"]
```

- `default_sort`：默认排序，格式 `字段名:asc/desc`，多字段用逗号分隔
- `columns`：列表页显示的列

### 4.10 api 访问控制

```toml
[api.list]
access = "public"                   # public / member / admin / none
cache = true                        # 是否启用服务端缓存
filter = 'status = "published"'     # 数据过滤表达式
filter_auth = 'author_id = @request.auth.id'  # 已登录用户的额外过滤

[api.get]
access = "public"
cache = true

[api.create]
access = "member"                   # 需要登录

[api.update]
access = "member"
filter = "@auth.id == author_id || @auth.role == 'admin'"  # API Rule

[api.delete]
access = "member"
filter = "@auth.id == author_id || @auth.role == 'admin'"
```

**access 级别：**

| 值 | 说明 |
|----|------|
| `none` | 完全禁止 |
| `public` | 公开，无需认证 |
| `member` | 需要登录 |
| `admin` | 需要管理员角色 |

**cache 默认为 `false`。** 需要显式设置 `cache = true` 才会启用缓存。

**filter vs filter_auth：**
- `filter`：对所有通过 access 检查的请求生效（SQL WHERE 附加条件）
- `filter_auth`：仅对已登录用户额外生效，与 `filter` 取 OR 关系

### 4.11 CMS 自动生成的列

CMS 框架会自动添加以下列，**不需要在 fields 中声明**：

| 条件 | 自动添加的列 |
|------|-------------|
| 始终 | `id TEXT PRIMARY KEY` |
| 始终 | `tenant_id TEXT NOT NULL DEFAULT 'default'` |
| `draft_publish = true` | `status TEXT DEFAULT 'draft'`、`published_at TEXT` |
| `timestamps = true`（默认） | `created_at TEXT`、`updated_at TEXT` |
| `soft_delete = true` | `deleted_at TEXT` |

**Plugin INSERT 语句必须包含 `tenant_id` 列（值为 `'default'`）。**

**Plugin INSERT 语句如果 `timestamps = true`，必须包含 `created_at` 和 `updated_at` 列。**

### 4.12 CMS 自动生成的 REST API

每个 Content Type 自动注册以下 5 个 API 端点：

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/cms/{plural}` | 列表查询（支持分页、排序、field filter） |
| GET | `/api/v1/cms/{plural}/{id}` | 单条查询 |
| POST | `/api/v1/cms/{plural}` | 创建 |
| PUT | `/api/v1/cms/{plural}/{id}` | 更新 |
| DELETE | `/api/v1/cms/{plural}/{id}` | 删除 |

**列表查询参数：**

| 参数 | 说明 | 示例 |
|------|------|------|
| `page` | 页码（默认 1） | `?page=2` |
| `page_size` | 每页条数（默认 20） | `?page_size=50` |
| `sort` | 排序字段 | `?sort=created_at:desc` |
| `{field_name}` | 按字段过滤 | `?board=xxx` |

---

## 5. plugin/manifest.toml — Plugin 清单

### 5.1 plugin 基本信息字段

```toml
[plugin]
id = "com.rust-blog.forum"          # 全局唯一 ID（推荐反向域名格式）
name = "Forum API"                  # 显示名称
version = "0.1.0"                   # 语义化版本
description = "论坛 API"
author = "rust-blog"
license = "MIT"
runtime = "js"                      # "js" 或 "lua"
language = "js"                     # "js" 或 "lua"
entry = "main.js"                   # 入口文件名
```

| 字段 | 必填 | 默认值 | 说明 |
|------|------|--------|------|
| `id` | 是 | — | 全局唯一 Plugin ID |
| `name` | 是 | — | 显示名称 |
| `version` | 是 | — | 语义化版本 |
| `description` | 否 | `""` | 描述 |
| `author` | 否 | — | 作者 |
| `license` | 否 | — | 许可证 |
| `runtime` | 否 | `"wasm"` | 运行时：`"js"` / `"lua"` / `"wasm"` |
| `language` | 否 | `"rust"` | 语言：`"js"` / `"lua"` / `"rust"` 等 |
| `entry` | 否 | `"index.js"` | 入口文件名 |

### 5.2 permissions 权限声明

```toml
[permissions]
max_memory_mb = 16                  # 内存限制（MB）
timeout_ms = 5000                   # 单次执行超时（毫秒）
database = [                        # 允许访问的数据库表
    "forum_boards",                 # 读写权限
    "forum_topics",
    "read:product_categories",      # 只读权限（read: 前缀）
]
http = ["api.example.com/*"]        # 允许的 HTTP 请求（预留）
config = ["seo.*"]                  # 允许读取的配置 key（预留）
filesystem = ["read-write"]         # 文件系统权限（预留）
```

**database 权限格式：**
- `"table_name"` = 读写权限
- `"read:table_name"` = 只读权限

### 5.3 routes 路由声明

```toml
[[routes]]
method = "GET"
path = "/api/v1/plugins/forum/boards/:slug/topics"
handler = "listBoardTopics"
# auth = "member"                  # 可选：public（默认）/ member / admin

[[routes]]
method = "POST"
path = "/api/v1/plugins/forum/vote"
handler = "vote"
auth = "member"                     # 需要登录
```

**规则：**
- 路径必须以 `/api/v1/plugins/{extension_id}/` 开头
- `:param` 是路径参数，如 `:id`、`:slug`、`:pollId`
- `handler` 对应 JS 中 `Plugin.xxx` 函数名
- `auth` 省略时为 `public`（无需认证）
- 框架自动将 Plugin 返回值统一为 `{code, message, data}` 格式

### 5.4 hooks 钩子声明

```toml
[hooks.on-content-creating]
priority = 50                       # 优先级（数字越小越先执行）

[hooks.on-content-created]
priority = 50

[hooks.on-content-deleted]
priority = 50

[hooks.on-content-viewed]
priority = 50
```

**Hook 名称（TOML 中用短横线，框架自动转为下划线）：**

| Hook 名（TOML） | JS 函数名 | 触发时机 | 数据内容 |
|------------------|-----------|----------|----------|
| `on-content-creating` | `on_content_creating` | CMS 内容创建前 | `{content_type, data: {...}, ...}` |
| `on-content-created` | `on_content_created` | CMS 内容创建后 | `{content_type, id, ...}` |
| `on-content-updating` | `on_content_updating` | CMS 内容更新前 | `{content_type, id, data: {...}}` |
| `on-content-updated` | `on_content_updated` | CMS 内容更新后 | `{content_type, id, ...}` |
| `on-content-deleted` | `on_content_deleted` | CMS 内容删除后 | `{content_type, id}` |
| `on-content-viewed` | `on_content_viewed` | CMS 内容查看后 | `{content_type, id}` |
| `on-post-creating` | `on_post_creating` | 文章创建前 | `{title, content, ...}` |
| `on-post-created` | `on_post_created` | 文章创建后 | `{id, title, ...}` |
| `on-post-updating` | `on_post_updating` | 文章更新前 | 同上 + `id` |
| `on-post-updated` | `on_post_updated` | 文章更新后 | 同上 |
| `on-post-deleted` | `on_post_deleted` | 文章删除后 | `{id}` |
| `on-comment-creating` | `on_comment_creating` | 评论创建前 | `{content, post_id, ...}` |
| `on-comment-created` | `on_comment_created` | 评论创建后 | `{id, ...}` |
| `render-markdown` | `render_markdown` | Markdown 渲染 | 字符串 |
| `filter-html` | `filter_html` | HTML 后处理 | 字符串 |
| `on-login` | `on_login` | 用户登录后 | `{email, success}` |
| `on-cron-tick` | `on_cron_tick` | 定时任务触发 | `{job_type, payload}` |

**on-content-creating vs on-post-creating：**
- `on-content-*` 是通用 Hook，对所有 CMS content type 生效
- `on-post-*` 是专用 Hook，仅对内置 posts 表生效
- 推荐使用 `on-content-*` 系列

### 5.5 cron 定时任务

```toml
[[cron]]
label = "Cleanup Sessions"
job_type = "cleanup_sessions"
payload = '{"max_age_hours": 24}'
cron_expr = "0 0 */6 * * *"         # 七段式（含秒）
enabled = true                       # 默认 true
```

| 字段 | 必填 | 说明 |
|------|------|------|
| `label` | 是 | 可读标签 |
| `job_type` | 是 | 自定义任务类型字符串 |
| `payload` | 否 | JSON payload |
| `cron_expr` | 是 | 七段式 Cron 表达式（秒 分 时 日 月 周） |
| `enabled` | 否 | 默认 `true` |

---

## 6. Plugin JS 代码编写

### 6.1 框架约定

1. 必须声明全局 `var Plugin = {};` 对象
2. Route Handler 定义为 `Plugin.handlerName = function(input) { ... }`
3. Hook Handler 定义为 `Plugin.hookName = function(input) { ... }`
4. 所有 Handler 返回**JSON 字符串**
5. 框架传入的 `input` 是**JSON 字符串**（需要双重解析）

### 6.2 工具函数（必须复制）

以下工具函数是每个 Plugin 的基础骨架，直接复制使用：

```javascript
var Plugin = {};

function ok(result) {
    if (result && result._error) {
        return JSON.stringify({ status: result._status || 400, body: JSON.stringify({ ok: false, error: result._error }) });
    }
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: result }) });
}

function err(status, msg) {
    return { _error: msg, _status: status };
}

function parseBody(input) {
    try {
        if (typeof input === "string") {
            var parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input && input.body) return JSON.parse(input.body);
        return {};
    } catch (e) { return {}; }
}

function routeParam(input, index) {
    var obj = input;
    if (typeof input === "string") {
        try { obj = JSON.parse(input); } catch (e) { return ""; }
    }
    var path = (obj.path || "").replace(/\/+$/, "");
    var qIdx = path.indexOf("?");
    if (qIdx >= 0) path = path.substring(0, qIdx);
    var parts = path.split("/");
    return parts[parts.length - (index || 1)];
}

function genId() {
    return "xxxxxxxx-xxxx-7xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
        var r = (Math.random() * 16) | 0;
        var v = c === "x" ? r : (r & 0x3) | 0x8;
        return v.toString(16);
    });
}

function nowISO() {
    return new Date().toISOString();
}

function query(sql, params) {
    var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
}

function exec(sql, params) {
    var result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}
```

#### parseBody 详解

框架传给 JS 插件的 `input` 是 JSON 字符串，结构如下：

```json
{
    "path": "/api/v1/plugins/forum/vote?user_id=xxx",
    "method": "POST",
    "body": "{\"user_id\":\"xxx\",\"target_type\":\"topic\"}",
    "headers": {...}
}
```

`parseBody` 做了两层解析：
1. 先 `JSON.parse(input)` 得到外层对象
2. 如果外层对象有 `body` 字段且是 JSON 字符串，再 `JSON.parse(body)`

#### routeParam 详解

从 URL path 中提取路径参数。`index` 参数表示从路径末尾倒数第几个段：

```
path = "/api/v1/plugins/forum/boards/my-slug/topics"
parts = ["", "api", "v1", "plugins", "forum", "boards", "my-slug", "topics"]

routeParam(input, 1)  → "topics"    (最后一段)
routeParam(input, 2)  → "my-slug"   (倒数第二段)
routeParam(input, 3)  → "boards"    (倒数第三段)
```

对应路由 `/api/v1/plugins/forum/boards/:slug/topics`，`:slug` 是倒数第二段。

**注意：** 框架传递的 `path` 包含 query string，`routeParam` 已自动处理（截取 `?` 之前的部分）。

#### ok / err 详解

```javascript
// 成功响应
return ok({ id: "xxx", name: "hello" });
// → 框架转为 {code: 0, message: "ok", data: {id: "xxx", name: "hello"}}

// 错误响应
return ok(err(404, "not found"));
// → 框架转为 {code: 40400, message: "not found", data: null}

// 错误响应（自定义状态码）
return ok(err(403, "forbidden"));
// → 框架转为 {code: 40300, message: "forbidden", data: null}
```

### 6.3 Host API

Plugin 通过全局 `Host` 对象与宿主交互：

| API | 说明 | 返回值 |
|-----|------|--------|
| `Host.dbQuery(sql, paramsJson)` | 执行 SELECT 查询（支持参数化） | JSON 字符串数组，或 `"error:..."` |
| `Host.dbExecute(sql, paramsJson)` | 执行写操作（INSERT/UPDATE/DELETE） | `{"rows_affected":N,"error":null}` |
| `Host.dbBegin()` | 开启数据库事务 | `{"ok":true}` 或 `{"error":"..."}` |
| `Host.dbCommit()` | 提交事务 | `{"ok":true}` 或 `{"error":"..."}` |
| `Host.dbRollback()` | 回滚事务 | `{"ok":true}` 或 `{"error":"..."}` |
| `Host.emitEvent(eventType, dataJson)` | 触发自定义事件（广播到 EventBus + WebSocket） | `{"ok":true}` 或 `{"error":"..."}` |
| `Host.log(level, message)` | 写日志 | 无 |
| `Host.getConfig(key)` | 读取配置 | 字符串或 `null` |

**dbQuery 返回值：**
- 成功：`[{"id":"xxx","name":"hello","count":"5"},...]`（注意整数列返回字符串或 null）
- 失败：`"error: table not found"`

**dbQuery 参数化查询（推荐，防 SQL 注入）：**

```javascript
// ✅ 推荐：使用参数化查询
var rows = query("SELECT id, name FROM forum_topics WHERE board_id = ? AND is_pinned = ?", [boardId, 1]);

// ❌ 危险：字符串拼接（有 SQL 注入风险）
var rows = query("SELECT id, name FROM forum_topics WHERE board_id = '" + boardId + "'");
```

**dbExecute 参数化查询：**
- `{"rows_affected":1,"last_insert_rowid":null,"error":null}`

**整数列问题：** QuickJS SQLite 绑定返回整数列为 `null`。必须用 `CAST(col AS TEXT)` 转换：

```javascript
// ❌ 错误 — vote_count 为 null
var rows = query("SELECT id, vote_count FROM forum_poll_options");

// ✅ 正确 — 用 CAST 转为字符串再 parseInt
var rows = query("SELECT id, CAST(vote_count AS TEXT) as vote_count FROM forum_poll_options");
var count = parseInt(rows[0].vote_count, 10) || 0;
```

### 6.4 Route Handler 编写

Route Handler 是 `Plugin.xxx` 函数，框架传入 `input`（JSON 字符串），返回 JSON 字符串。

**模板：**

```javascript
Plugin.myHandler = function(input) {
    var data = parseBody(input);          // 解析请求 body
    var paramId = routeParam(input, 1);   // 提取路径参数

    // 参数校验
    if (!paramId) return ok(err(400, "id required"));

    // 数据库查询
    var rows = query("SELECT * FROM my_table WHERE id = '" + escapeSQL(paramId) + "'");
    if (!rows || rows.length === 0) return ok(err(404, "not found"));

    // 业务逻辑
    return ok(rows[0]);
};
```

**从 query string 获取参数：**

```javascript
Plugin.getPoll = function(input) {
    var topicId = routeParam(input, 1);
    var obj = input;
    if (typeof input === "string") { try { obj = JSON.parse(input); } catch (e) {} }
    var fullPath = obj.path || "";
    var qsIdx = fullPath.indexOf("?");
    var userId = "";
    if (qsIdx >= 0) {
        var qs = fullPath.substring(qsIdx + 1);
        var pairs = qs.split("&");
        for (var p = 0; p < pairs.length; p++) {
            if (pairs[p].indexOf("user_id=") === 0) {
                userId = decodeURIComponent(pairs[p].substring(8));
            }
        }
    }
    // ... 使用 userId
};
```

**写入数据（INSERT 必须包含 tenant_id 和 timestamps）：**

```javascript
Plugin.createItem = function(input) {
    var data = parseBody(input);
    var id = genId();
    var now = nowISO();
    exec(
        "INSERT INTO my_table (id, tenant_id, name, created_at, updated_at) VALUES (?, 'default', ?, ?, ?)",
        [id, data.name, now, now]
    );
    return ok({ id: id, name: data.name });
};
```

### 6.5 Hook Handler 编写

**on_content_creating — 创建前拦截/修改：**

```javascript
Plugin.on_content_creating = function(input) {
    var data = parseBody(input);
    var ct = data.content_type;        // content type 名称
    var body = data.data || {};        // 请求数据

    if (ct === "forum_reply") {
        var topicId = body.topic_id;
        if (topicId) {
            var topics = query("SELECT is_locked FROM forum_topics WHERE id = '" + escapeSQL(topicId) + "'");
            if (topics && topics.length > 0 && topics[0].is_locked) {
                return JSON.stringify({ status: 400, body: JSON.stringify({ ok: false, error: "topic is locked" }) });
            }
        }
    }

    return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};
```

**注意：** `on_content_creating` 返回的不是 `ok()`/`err()` 格式，而是直接返回 `{status, body}` JSON。

**on_content_created — 创建后副作用：**

```javascript
Plugin.on_content_created = function(input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        var topics = query("SELECT board_id FROM forum_topics WHERE id = '" + escapeSQL(id) + "'");
        if (topics && topics.length > 0) {
            var now = nowISO();
            exec("UPDATE forum_boards SET topic_count = topic_count + 1, post_count = post_count + 1, last_activity_at = ?, last_topic_id = ? WHERE id = ?",
                [now, id, topics[0].board_id]);
        }
    }

    return ok(data);
};
```

**on_content_deleted — 删除后副作用：**

```javascript
Plugin.on_content_deleted = function(input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        var topics = query("SELECT board_id FROM forum_topics WHERE id = '" + escapeSQL(id) + "'");
        if (topics && topics.length > 0) {
            exec("UPDATE forum_boards SET topic_count = CASE WHEN topic_count > 0 THEN topic_count - 1 ELSE 0 END WHERE id = ?",
                [topics[0].board_id]);
        }
    }

    return ok(data);
};
```

**on_content_viewed — 查看后副作用：**

```javascript
Plugin.on_content_viewed = function(input) {
    var data = parseBody(input);
    var ct = data.content_type;
    var id = data.id;

    if (ct === "forum_topic") {
        exec("UPDATE forum_topics SET view_count = view_count + 1 WHERE id = ?", [id]);
    }

    return ok(data);
};
```

### 6.6 数据库事务

使用 `Host.dbBegin()` / `dbCommit()` / `dbRollback()` 实现事务包裹：

```javascript
Plugin.checkout = function(input) {
    var data = parseBody(input);
    var userId = data.user_id;

    // 开启事务
    var beginResult = JSON.parse(Host.dbBegin());
    if (!beginResult.ok) return ok(err(500, "failed to begin transaction"));

    // 执行多步操作
    var orderId = genId();
    var r = exec(
        "INSERT INTO orders (id, tenant_id, user_id, status, created_at, updated_at) VALUES (?, 'default', ?, 'pending', ?, ?)",
        [orderId, userId, nowISO(), nowISO()]
    );
    if (r.error) {
        Host.dbRollback();  // 失败时必须回滚
        return ok(err(500, "order failed: " + r.error));
    }

    var r2 = exec("UPDATE products SET stock = stock - 1 WHERE id = ?", [data.product_id]);
    if (r2.error || r2.rows_affected === 0) {
        Host.dbRollback();
        return ok(err(500, "stock deduction failed"));
    }

    // 提交事务
    var commitResult = JSON.parse(Host.dbCommit());
    if (!commitResult.ok) return ok(err(500, "commit failed"));

    return ok({ order_id: orderId });
};
```

**事务规则：**
- 同一时刻只能有一个活跃事务
- 插件超时/崩溃时框架自动 rollback
- 失败时必须手动调用 `dbRollback()`，否则事务残留到超时

### 6.7 事件触发（emitEvent）

Plugin 可通过 `Host.emitEvent()` 主动触发自定义事件：

```javascript
Plugin.checkout = function(input) {
    // ... 创建订单逻辑 ...

    // 触发自定义事件，其他插件可通过 Hook 监听
    Host.emitEvent("OrderCreated", JSON.stringify({
        order_id: orderId,
        user_id: userId,
        total: totalAmount
    }));

    return ok({ order_id: orderId });
};
```

**触发的事件会：**
1. 通过 `EventBus` 广播到所有订阅者
2. 推送到 WebSocket 客户端（前端可通过 WS 实时接收）
3. 事件类型名 `eventType` 用于 WS/SSE 过滤

**前端接收：**

```typescript
// WebSocket 接收自定义事件
ws.onmessage = (event) => {
    const msg = JSON.parse(event.data);
    if (msg.type === "event" && msg.event === "OrderCreated") {
        console.log("New order:", msg.data);
    }
};

// 订阅特定事件类型
ws.send(JSON.stringify({
    type: "subscribe",
    filter: ["OrderCreated", "PaymentReceived"]
}));
```

### 6.8 QuickJS 语法支持

JS 插件运行在 QuickJS 沙箱中，支持 **ES2024** 绝大多数特性：

| 特性 | 支持情况 | 示例 |
|------|----------|------|
| `let` / `const` | ✅ | `const name = "hello"; let count = 0;` |
| 箭头函数 | ✅ | `const add = (a, b) => a + b;` |
| 模板字符串 | ✅ | `` `hello ${name}` `` |
| 可选链 `?.` | ✅ | `data?.name?.first` |
| 空值合并 `??` | ✅ | `value ?? "default"` |
| `for...of` | ✅ | `for (const item of arr) { ... }` |
| 解构赋值 | ✅ | `const {a, b} = obj;` |
| 默认参数 | ✅ | `function greet(name = "world") { ... }` |
| 展开运算符 `...` | ✅ | `const b = [...a, 3];` |
| 对象简写 | ✅ | `const obj = {x, y};` |
| `class` 语法 | ✅ | `class Foo { constructor() {} }` |
| `async` / `await` | ✅（但插件函数为同步调用） | `async function f() { ... }` |
| 指数运算符 `**` | ✅ | `2 ** 10` |
| `import` / `export` | ❌ | 单文件，使用全局 `Plugin` 对象 |
| `new URL()` | ❌ | 手动解析字符串 |

**最佳实践**：推荐使用 `const`/`let`、箭头函数、可选链、模板字符串等现代语法编写插件。

### 6.9 框架响应格式统一

**重要：** 框架在 `src/plugins.rs` 中将 Plugin 的返回值自动统一为 `{code, message, data}` 格式。

Plugin 返回的原始格式：
```json
{"status": 200, "body": "{\"ok\": true, \"data\": {...}}"}
```

框架转换后（前端实际收到的）：
```json
{"code": 0, "message": "ok", "data": {...}}
```

**前端无需特殊处理**，直接用 `apiRequest` 调用 Plugin 路由即可。

---

## 7. Plugin Lua 代码编写

Lua 插件使用 `mlua` 运行时。结构类似：

```lua
Plugin = {}

Plugin.stats_overview = function(input)
    local result = Host.dbQuery("SELECT COUNT(*) as total FROM posts")
    if not result then return {status = 500, body = '{"code":50000,"message":"query failed"}'} end

    local total = tonumber(result:match('"total":"?(%d+)"?')) or 0
    local data = '{"total_posts":' .. total .. '}'

    return {
        status = 200,
        body = '{"code":0,"message":"ok","data":' .. data .. '}'
    }
end
```

**Lua 特点：**
- Host API 相同：`Host.dbQuery(sql)`、`Host.dbExecute(sql, params)`、`Host.log(level, msg)`
- 返回 Lua table `{status = N, body = "json string"}`
- `dbQuery` 返回原始 JSON 字符串，需要手动解析（用 Lua 模式匹配）
- 不需要 `parseBody`（框架直接传 Lua table）

---

## 8. API Rule 表达式语法

在 Content Type TOML 的 `[api.*]` 中使用 `filter` 字段控制数据访问：

```toml
filter = "@auth.id == author_id || @auth.role == 'admin'"
```

### 操作数

| 操作数 | 说明 | 示例 |
|--------|------|------|
| `field_name` | 当前表字段 | `author_id`、`status` |
| `@auth.id` | 当前认证用户 ID | `@auth.id == author_id` |
| `@auth.role` | 当前认证用户角色 | `@auth.role == "admin"` |
| `@request.body.field` | 请求体字段 | `@request.body.title != ""` |
| `@request.query.param` | URL 查询参数 | `@request.query.category = "news"` |
| `@now` | 当前 ISO 8601 时间 | `created_at > @now` |
| `"string"` | 字符串字面量 | `"published"` |
| `123` | 数字字面量 | `42`、`3.14` |
| `true` / `false` | 布尔字面量 | `true` |
| `null` | 空值 | `null` |

### 比较运算符

| 运算符 | 说明 | 示例 |
|--------|------|------|
| `=` 或 `==` | 等于 | `status = "published"` |
| `!=` | 不等于 | `status != "draft"` |
| `>` | 大于 | `price > 0` |
| `>=` | 大于等于 | `stock >= 1` |
| `<` | 小于 | `created_at < @now` |
| `<=` | 小于等于 | `score <= 100` |
| `~` | 包含（LIKE） | `title ~ "rust"` |
| `!~` | 不包含 | `title !~ "spam"` |

### 逻辑运算符

| 运算符 | 说明 | 示例 |
|--------|------|------|
| `&&` | 与 | `status = "published" && author_id = @auth.id` |
| `\|\|` | 或 | `@auth.id == author_id \|\| @auth.role == 'admin'` |

### 后缀操作

| 操作 | 说明 | 示例 |
|------|------|------|
| `field:isset` | 字段非空 | `avatar:isset` |
| `field:length > N` | 字符串/数组长度 | `title:length > 0` |

---

## 9. 前端对接

### 调用 CMS CRUD API

```typescript
import { apiRequest } from "@/lib/api";

// 列表
const data = await apiRequest("GET", "/cms/todos?page=1&page_size=20");
// data = { items: [...], total: 100, page: 1, page_size: 20 }

// 单条
const todo = await apiRequest("GET", "/cms/todos/" + id);

// 创建
await apiRequest("POST", "/cms/todos", { title: "My Todo", done: false });

// 更新
await apiRequest("PUT", "/cms/todos/" + id, { done: true });

// 删除
await apiRequest("DELETE", "/cms/todos/" + id);

// Field filter（用字段名，不用列名）
const data = await apiRequest("GET", "/cms/forum_replies?topic=" + topicId);
```

### 调用 Plugin 路由

```typescript
// Plugin 路由和 CMS 路由使用相同的 apiRequest
const result = await apiRequest("POST", "/plugins/forum/vote", {
    target_type: "topic",
    target_id: topicId,
    value: 1
});
```

### Boolean 字段注意

CMS boolean 字段从 API 返回的是整数 `0`/`1` 而非 `true`/`false`。前端条件渲染必须严格比较：

```tsx
// ❌ 错误 — 0 && <Component> 会渲染 "0"
{topic.is_pinned && <PinIcon />}

// ✅ 正确
{topic.is_pinned === true && <PinIcon />}
// 或者
{topic.is_pinned === 1 && <PinIcon />}
```

---

## 10. Extension 版本迁移

Extension 支持版本升级时执行自定义 SQL 迁移。

### 迁移文件目录

```
extensions/my-extension/
├── extension.toml
├── migrations/          ← 版本迁移 SQL 文件
│   ├── 0.2.0.sql        ← 升级到 0.2.0 时执行
│   └── 0.3.0.sql        ← 升级到 0.3.0 时执行
└── ...
```

### 工作原理

1. Extension 加载时，框架读取数据库中 `extensions` 表记录的 `version`
2. 如果 TOML 中的 `version` 大于数据库记录的 `version`，触发升级
3. 框架扫描 `migrations/` 目录，执行所有版本号 > old_version 且 ≤ new_version 的 `.sql` 文件
4. 升级完成后更新数据库中的 `version` 和 `updated_at`

### 迁移文件格式

文件名为版本号（如 `0.2.0.sql`），内容为标准 SQL：

```sql
-- migrations/0.2.0.sql
ALTER TABLE my_table ADD COLUMN priority TEXT DEFAULT 'medium';
CREATE INDEX IF NOT EXISTS idx_my_table_priority ON my_table(priority);
```

### 注意事项

- 版本比较使用字符串序（`"0.2.0" < "0.3.0" < "1.0.0"`）
- Content Type TOML 的新增列会由 CMS 自动 ALTER TABLE，不需要手动写迁移
- 迁移 SQL 执行失败会记录错误日志但不阻止 Extension 加载
- 没有 `migrations/` 目录的 Extension 不受影响

---

## 11. 完整示例：Todo Extension

### 文件结构

```
extensions/todo/
├── extension.toml
├── content_types/
│   └── todo.toml
└── plugin/
    ├── manifest.toml
    └── main.js
```

### extension.toml

```toml
[extension]
id = "todo"
name = "Todo"
version = "1.0.0"
description = "待办事项扩展"
author = "Team"
license = "MIT"
content_types = "content_types/"
plugin = "plugin/manifest.toml"

[extension.dependencies]
```

### content_types/todo.toml

```toml
[content_type]
name = "Todo"
singular = "todo"
plural = "todos"
table = "todos"
description = "待办事项"
draft_publish = false
timestamps = true
soft_delete = false

[fields.title]
type = "text"
required = true
max_length = 200
label = "标题"

[fields.description]
type = "richtext"
label = "描述"

[fields.done]
type = "boolean"
default = false
label = "已完成"

[fields.due_date]
type = "datetime"
label = "截止日期"

[fields.priority]
type = "enum"
enum_values = ["low", "medium", "high"]
default = "medium"
label = "优先级"

[fields.author_id]
type = "text"
required = true
auto_fill = "user_id"
label = "创建者"

[[indexes]]
fields = ["author_id", "created_at"]

[[indexes]]
fields = ["done", "priority"]

[list_view]
default_sort = "created_at:desc"
columns = ["title", "done", "priority", "due_date", "created_at"]

[api.list]
access = "public"
cache = true

[api.get]
access = "public"
cache = true

[api.create]
access = "member"

[api.update]
access = "member"
filter = "@auth.id == author_id || @auth.role == 'admin'"

[api.delete]
access = "member"
filter = "@auth.id == author_id || @auth.role == 'admin'"
```

### plugin/manifest.toml

```toml
[plugin]
id = "com.example.todo"
name = "Todo API"
version = "1.0.0"
description = "待办事项统计和批量操作"
author = "Team"
license = "MIT"
runtime = "js"
language = "js"
entry = "main.js"

[permissions]
max_memory_mb = 8
timeout_ms = 3000
database = ["todos"]

[hooks.on-content-created]
priority = 50

[[routes]]
method = "GET"
path = "/api/v1/plugins/todo/stats"
handler = "getStats"
```

### plugin/main.js

```javascript
var Plugin = {};

function ok(result) {
    if (result && result._error) {
        return JSON.stringify({ status: result._status || 400, body: JSON.stringify({ ok: false, error: result._error }) });
    }
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: result }) });
}

function err(status, msg) {
    return { _error: msg, _status: status };
}

function parseBody(input) {
    try {
        if (typeof input === "string") {
            var parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input && input.body) return JSON.parse(input.body);
        return {};
    } catch (e) { return {}; }
}

function query(sql, params) {
    var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.indexOf("error:") === 0) return null;
    return JSON.parse(result);
}

function exec(sql, params) {
    var result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}

// ── Hooks ───────────────────────────────────────────────────

Plugin.on_content_created = function(input) {
    var data = parseBody(input);
    if (data.content_type === "todo") {
        Host.log("info", "[todo] new todo created: " + data.id);
    }
    return ok(data);
};

// ── GET /stats ──────────────────────────────────────────────

Plugin.getStats = function(input) {
    var data = parseBody(input);
    var userId = data.user_id;

    var totalResult = query("SELECT COUNT(*) as cnt FROM todos");
    var total = (totalResult && totalResult[0]) ? parseInt(totalResult[0].cnt, 10) : 0;

    var doneResult = query("SELECT COUNT(*) as cnt FROM todos WHERE done = 1");
    var done = (doneResult && doneResult[0]) ? parseInt(doneResult[0].cnt, 10) : 0;

    return ok({
        total: total,
        done: done,
        pending: total - done
    });
};

---

## 12. 常见陷阱

### 1. INSERT 忘记 tenant_id

```javascript
// ❌ 错误
exec("INSERT INTO my_table (id, name) VALUES (?, ?)", [id, name]);

// ✅ 正确
exec("INSERT INTO my_table (id, tenant_id, name, created_at, updated_at) VALUES (?, 'default', ?, ?, ?)",
    [id, name, now, now]);
```

### 2. 整数列返回 null

```javascript
// ❌ 错误 — QuickJS 中 vote_count 为 null
var rows = query("SELECT vote_count FROM my_table");
var count = rows[0].vote_count; // null

// ✅ 正确 — 用 CAST 转为字符串
var rows = query("SELECT CAST(vote_count AS TEXT) as vote_count FROM my_table");
var count = parseInt(rows[0].vote_count, 10) || 0;
```

### 3. draft_publish = false 时查询 status

```javascript
// ❌ 错误 — 该表没有 status 列
var rows = query("SELECT * FROM forum_topics WHERE status = 'published'");

// ✅ 正确 — draft_publish = false 的表无 status 列
var rows = query("SELECT * FROM forum_topics WHERE id = ?", [id]);
```

### 4. 仍不支持 `import/export` 和 `new URL()`

```javascript
// ❌ 错误 — QuickJS 不支持 ES Module
import { something } from "lib";

// ❌ 错误 — QuickJS 不内置 URL 构造函数
const url = new URL("https://example.com/path");

// ✅ 正确 — 单文件，全局 Plugin 对象；手动解析路径
const path = input.path || "";
const parts = path.split("/");
```

### 5. 前端 Boolean 比较

```tsx
// ❌ 错误 — 0 && <Component> 渲染 "0"
{todo.done && <CheckIcon />}

// ✅ 正确
{todo.done === true && <CheckIcon />}
{todo.done === 1 && <CheckIcon />}
```

### 6. Hook on_content_creating 返回格式不同

```javascript
// on_content_creating 返回原始 {status, body} 格式（不用 ok/err）
Plugin.on_content_creating = function(input) {
    return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};

// 其他 Hook 可以用 ok()
Plugin.on_content_created = function(input) {
    return ok(data);
};
```

### 7. CMS 字段名 vs 数据库列名

```javascript
// 字段名: board → 数据库列名: board_id
// 字段名: topic → 数据库列名: topic_id

// ✅ Plugin SQL 用列名
query("SELECT board_id FROM forum_topics WHERE id = ?", [id]);

// ✅ 前端 API filter 用字段名
apiRequest("GET", "/cms/forum_replies?topic=" + topicId);
```

### 8. 字符串拼接 SQL 注入

```javascript
// ❌ 危险 — 有 SQL 注入风险
var rows = query("SELECT * FROM topics WHERE slug = '" + slug + "'");

// ✅ 安全 — 使用参数化查询
var rows = query("SELECT * FROM topics WHERE slug = ?", [slug]);
```

### 9. 事务未回滚

```javascript
// ❌ 错误 — 失败后未回滚，事务残留
Plugin.checkout = function(input) {
    Host.dbBegin();
    var r = exec("INSERT INTO orders ...");
    if (r.error) return ok(err(500, "failed")); // 事务未关闭！
    Host.dbCommit();
};

// ✅ 正确 — 失败时回滚
Plugin.checkout = function(input) {
    var begin = JSON.parse(Host.dbBegin());
    if (!begin.ok) return ok(err(500, "begin failed"));
    var r = exec("INSERT INTO orders ...");
    if (r.error) { Host.dbRollback(); return ok(err(500, r.error)); }
    Host.dbCommit();
    return ok({ success: true });
};
```

## 附录 A：现有 Extension 参考

| Extension | Content Types | Plugin 路由 | Plugin Hooks | 运行时 |
|-----------|--------------|-------------|-------------|--------|
| `first-ext` | 1 (article) | 4 (stats) | 0 | Lua |
| `ecommerce` | 5 (product, category, cart_item, order, order_item) | 9 | 0 | JS |
| `forum` | 7 (board, topic, reply, vote, poll, poll_option, poll_vote) | 8 | 4 | JS |

**学习顺序推荐：**
1. `first-ext` — 最简单的 Extension 结构
2. `ecommerce` — 展示 Content Type 多样性和复杂 Plugin 路由
3. `forum` — 最完整，包含 Hook + 复杂查询 + 计数维护

## 附录 B：Content Type TOML → SQL 映射

| TOML 字段类型 | SQLite 列类型 | 约束 |
|---------------|---------------|------|
| `text` | `TEXT` | — |
| `richtext` | `TEXT` | — |
| `integer` | `INTEGER` | — |
| `big_int` | `INTEGER` | — |
| `decimal` | `REAL` | — |
| `float` | `REAL` | — |
| `boolean` | `INTEGER` | DEFAULT 0 |
| `date` | `TEXT` | — |
| `datetime` | `TEXT` | — |
| `time` | `TEXT` | — |
| `email` | `TEXT` | — |
| `password` | `TEXT` | — |
| `enum` | `TEXT` | CHECK(col IN (...)) |
| `uid` | `TEXT` | UNIQUE if specified |
| `json` | `TEXT` | — |
| `media` | `TEXT` | — |
| `relation` (many_to_one) | `TEXT` | FOREIGN KEY via foreign_key |

## 附录 C：CMS 自动添加的列汇总

```
id            TEXT PRIMARY KEY     ← 始终
tenant_id     TEXT NOT NULL DEFAULT 'default'  ← 始终
status        TEXT DEFAULT 'draft' ← draft_publish = true
published_at  TEXT                 ← draft_publish = true
created_at    TEXT                 ← timestamps = true（默认）
updated_at    TEXT                 ← timestamps = true（默认）
deleted_at    TEXT                 ← soft_delete = true
```
