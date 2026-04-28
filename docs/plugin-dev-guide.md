# Plugin 开发指南

## 概述

Plugin 系统是 rust-blog 的运行时扩展机制，支持三种语言运行时，可独立于 Content Type 运行。Plugin 可以注册钩子、定时任务、自定义路由，并通过 Host API 访问数据库、HTTP、配置等受控资源。

## 架构

```
plugins/
  └── {plugin-id}/
       ├── manifest.toml    # 插件清单
       └── main.js          # 入口文件（JS/Lua/WASM）
              ↓ 启动加载
PluginManager（Arc 共享）
  ├─ 拓扑排序（依赖顺序）
  ├─ 实例池（round-robin 并发）
  └─ 热重载（文件系统监听）
              ↓ Hook 派发
Host API（沙箱权限控制）
  ├─ Host.dbQuery / dbExecute
  ├─ Host.httpGet / httpPost
  ├─ Host.getConfig
  ├─ Host.getData / setData（KV 存储）
  ├─ Host.fsRead / fsWrite（VFS）
  └─ Host.emitEvent
```

### 核心模块

| 模块 | 文件 | 职责 |
|------|------|------|
| PluginManager | `src/plugins.rs` | 加载/卸载/hook 派发/热重载/事件总线 |
| Manifest | `src/plugins/manifest.rs` | TOML 清单解析 |
| Permissions | `src/plugins/permissions.rs` | 权限校验 + SQL 注入防护 + SSRF 防护 |
| JS Engine | `src/plugins/engine_js.rs` | QuickJS 运行时 + 实例池 |
| JS Host | `src/plugins/js_host.rs` | JS → Rust Host API 桥接 |
| Lua Engine | `src/plugins/engine_lua.rs` | Lua 5.4 运行时 + 实例池 |
| Lua Host | `src/plugins/lua_host.rs` | Lua → Rust Host API 桥接 |
| WASM Engine | `src/plugins/engine.rs` | wasmtime 运行时 |
| WASM Host | `src/plugins/host.rs` | WASM → Rust Host API 桥接 |
| Host Common | `src/plugins/host_common.rs` | 共享 Host 逻辑 |
| VFS | `src/plugins/vfs.rs` | 插件隔离虚拟文件系统 |
| HTTP Client | `src/plugins/http_client.rs` | 插件 HTTP 请求 |
| CLI | `src/cli/plugin_cmd.rs` | `plugin new` / `plugin check` |

## 三种运行时

| 运行时 | Cargo Feature | 入口文件 | 引擎 |
|--------|--------------|----------|------|
| JavaScript | `plugin-js` | `main.js` | rquickjs (QuickJS) |
| Lua | `plugin-lua` | `init.lua` | mlua (Lua 5.4) |
| WASM | `plugin-wasm` | `plugin.wasm` | wasmtime |

三种运行时可同时编译、同时加载。

### 实例池

每个插件创建多个运行时实例（由 `PLUGIN_JS_POOL_SIZE` / `PLUGIN_LUA_POOL_SIZE` / `PLUGIN_WASM_POOL_SIZE` 控制），以 round-robin 方式分发请求，避免并发瓶颈。

## Manifest 文件 (`manifest.toml`)

### 最小示例

```toml
[plugin]
id = "com.example.my-plugin"
name = "My Plugin"
version = "0.1.0"
runtime = "js"
entry = "main.js"

[permissions]
max_memory_mb = 16
timeout_ms = 5000
```

### `[plugin]` 字段

| 字段 | 类型 | 必填 | 默认值 | 说明 |
|------|------|------|--------|------|
| `id` | string | 是 | — | 插件唯一 ID（建议反向域名格式） |
| `name` | string | 是 | — | 显示名称 |
| `version` | string | 是 | — | 语义版本号 |
| `description` | string | 否 | `""` | 描述 |
| `author` | string | 否 | — | 作者 |
| `license` | string | 否 | — | 许可证 |
| `runtime` | string | 否 | `"wasm"` | 运行时：`js` / `lua` / `wasm` |
| `language` | string | 否 | `"rust"` | 语言标识 |
| `entry` | string | 否 | `"index.js"` | 入口文件名 |
| `wasm` | string | 否 | `"plugin.wasm"` | WASM 文件路径 |

### `[permissions]` 权限声明

```toml
[permissions]
max_memory_mb = 16
timeout_ms = 5000
http = ["api.example.com", "*.github.com"]
config = ["app.*", "jwt.*"]
database = ["read:products", "write:orders", "categories"]
filesystem = ["read-write"]
```

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `max_memory_mb` | int | 配置默认值 | 单实例内存上限 |
| `timeout_ms` | int | 配置默认值 | Hook 执行超时 |
| `http` | string[] | `[]`（禁止） | HTTP 白名单 |
| `config` | string[] | `[]`（禁止） | 配置读取白名单 |
| `database` | string[] | `[]`（禁止） | 数据库权限 |
| `filesystem` | string[] | `[]`（禁止） | 文件系统权限 |

#### 数据库权限格式

| 格式 | 权限 |
|------|------|
| `"read:TABLE"` | 只读 |
| `"write:TABLE"` | 只写 |
| `"TABLE"` | 读写 |
| `"*"` | 所有表（受保护表除外） |

#### HTTP 白名单

- 精确域名：`api.example.com`
- 通配符子域：`*.github.com`
- 路径通配：`api.example.com/v1/*`

内置 SSRF 防护：自动阻止 localhost、127.x、10.x、172.16-31.x、192.168.x、169.254.x、::1。

#### 受保护表

以下系统表即使声明 `"*"` 也不可访问：

```
users, roles, permissions, audit_log, plugin_storage, options,
rbac_roles, rbac_permissions, rbac_role_permissions, tenants
```

### `[hooks.XXX]` 钩子注册

```toml
[hooks.on-content-created]
priority = 50

[hooks.on-content-updating]
priority = 100

[hooks.render-markdown]
priority = 10
```

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `priority` | int | 100 | 优先级，数字越小越先执行 |

钩子名使用连字符（`on-content-created`），系统自动转换为下划线（`on_content_created`）。

### 17 种钩子

| 钩子名 | 类型 | 说明 |
|--------|------|------|
| `on-content-creating` | filter | 内容创建前，可修改数据 |
| `on-content-created` | action | 内容创建后 |
| `on-content-updating` | filter | 内容更新前，可修改数据 |
| `on-content-updated` | action | 内容更新后 |
| `on-content-deleted` | action | 内容删除后 |
| `on-content-viewed` | action | 内容被浏览 |
| `on-post-creating` | filter | 文章创建前（兼容） |
| `on-post-created` | action | 文章创建后（兼容） |
| `on-post-updating` | filter | 文章更新前（兼容） |
| `on-post-updated` | action | 文章更新后（兼容） |
| `on-post-deleted` | action | 文章删除后（兼容） |
| `on-comment-creating` | filter | 评论创建前（兼容） |
| `on-comment-created` | action | 评论创建后（兼容） |
| `render-markdown` | filter | Markdown 渲染覆盖（第一个返回 wins） |
| `filter-html` | filter | HTML 过滤 |
| `on-login` | action | 用户登录后 |
| `on-cron-tick` | action | 定时任务触发 |

**filter 类型**：可修改数据，返回值传递给下一个插件。
**action 类型**：仅副作用，返回值忽略。

### `[[cron]]` 定时任务

```toml
[[cron]]
label = "每日统计"
job_type = "daily_stats"
cron_expr = "0 0 * * *"
payload = """{"type": "full"}"""
enabled = true
```

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `label` | string | 是 | 任务名称 |
| `job_type` | string | 是 | 任务类型（传给 `on_cron_tick`） |
| `cron_expr` | string | 是 | Cron 表达式 |
| `payload` | string | 否 | 附带数据 |
| `enabled` | bool | 否 | 默认 true |

### `[[routes]]` 自定义路由

```toml
[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/pipeline"
handler = "getPipeline"
auth = "admin"

[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/contacts/:contactId"
handler = "getContact"
auth = "public"
```

| 字段 | 类型 | 必填 | 默认 | 说明 |
|------|------|------|------|------|
| `method` | string | 是 | — | HTTP 方法 |
| `path` | string | 是 | — | 路由路径，支持 `:param` 占位符 |
| `handler` | string | 是 | — | 对应 Plugin 对象的函数名 |
| `auth` | string | 否 | `default` | `none` / `public` / `member` / `admin` |
| `description` | string | 否 | — | 描述 |
| `permission` | string | 否 | — | 额外权限要求 |

自定义路由的响应由框架统一包装为 `{ code: 0, message: "success", data: ... }` 格式。

## Host API

所有运行时通过统一的 `Host.*` 接口访问宿主功能：

### 数据库

```javascript
// 参数化查询（防注入）
const rows = JSON.parse(Host.dbQuery("SELECT * FROM products WHERE price > ?", JSON.stringify([100])));
const affected = Host.dbExecute("UPDATE products SET stock = stock - 1 WHERE id = ?", JSON.stringify([id]));

// 事务
Host.dbBegin();
Host.dbExecute("INSERT INTO orders ...", null);
Host.dbExecute("UPDATE products ...", null);
Host.dbCommit();  // 或 Host.dbRollback()
```

| 函数 | 说明 | 权限 |
|------|------|------|
| `Host.dbQuery(sql, params?)` | SELECT 查询 | `database` read |
| `Host.dbExecute(sql, params?)` | INSERT/UPDATE/DELETE | `database` write |
| `Host.dbBegin()` | 开启事务 | 需要 pool |
| `Host.dbCommit()` | 提交事务 | 活跃事务 |
| `Host.dbRollback()` | 回滚事务 | 活跃事务 |

> **注意**：`dbQuery` 返回的整数列是 `null`，需用 `CAST(col AS TEXT)` 转为字符串后再 `parseInt`。

### HTTP

```javascript
const html = Host.httpGet("https://api.example.com/data");
const result = Host.httpPost("https://api.example.com/webhook", JSON.stringify({event: "test"}));
```

### KV 存储

```javascript
Host.setData("last_sync", "2026-01-01");
const val = Host.getData("last_sync");
```

每个插件有独立命名空间，互不干扰。

### 配置

```javascript
const host = Host.getConfig("app.host");
const env = Host.getConfig("app.env");
```

允许的配置键（需在 `permissions.config` 白名单中）：
`app.host`, `app.port`, `app.env`, `app.base_url`, `jwt.access_expires`, `jwt.refresh_expires`, `upload.dir`, `upload.max_size`

### 文件系统（VFS）

```javascript
Host.fsWrite("/reports/daily.json", reportJson);
const data = Host.fsRead("/reports/daily.json");
const exists = Host.fsExists("/reports/daily.json");
const files = Host.fsList("/reports");
Host.fsDelete("/reports/old.json");
```

每个插件在 `{VFS_ROOT}/{plugin_id}/` 下有隔离沙箱，路径不能包含 `..`。

### 其他

```javascript
Host.log("info", "Processing order " + orderId);
Host.log("warn", "Low stock detected");
Host.log("error", "Payment failed: " + error);
Host.emitEvent("order.created", JSON.stringify({orderId: id}));
```

## 编写 JS 插件

### 项目结构

```
plugins/my-plugin/
  ├── manifest.toml
  └── main.js
```

### 代码模板

```javascript
var Plugin = {};

// ── 工具函数 ──

var ok = function(data) {
  return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};

var error = function(code, msg) {
  return JSON.stringify({ status: code, body: JSON.stringify({ error: msg }) });
};

var query = function(sql, params) {
  var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
  if (!result || result.indexOf("error:") === 0) return null;
  return JSON.parse(result);
};

// ── Hook ──

Plugin.on_content_created = function(input) {
  var data = JSON.parse(input);
  if (data.content_type === "product") {
    Host.log("info", "[my-plugin] new product: " + data.id);
  }
  return ok(data);
};

// ── 自定义路由 ──

Plugin.getStats = function(input) {
  var rows = query("SELECT COUNT(*) as cnt FROM products WHERE status = 'published'");
  return ok({ total: rows ? rows[0].cnt : 0 });
};

// ── 定时任务 ──

Plugin.on_cron_tick = function(input) {
  var data = JSON.parse(input);
  if (data.job_type === "daily_cleanup") {
    Host.dbExecute("DELETE FROM sessions WHERE expires_at < datetime('now')", null);
    Host.log("info", "[my-plugin] daily cleanup done");
  }
};
```

### 关键约定

- 必须导出全局 `Plugin` 对象
- Filter 钩子：接收 JSON 字符串，返回修改后的 JSON 字符串
- Action 钩子：接收 JSON 字符串，返回值被忽略
- 路由处理：接收 `{ path, method, body, headers }` JSON，返回 `{ status, body }` JSON 字符串
- 使用 `var` 而非 `let`/`const`（QuickJS 兼容性更好）
- 支持 ES2024 语法

## 编写 Lua 插件

### 项目结构

```
plugins/my-plugin/
  ├── manifest.toml
  └── init.lua
```

### 代码模板

```lua
Plugin = {}

function Plugin.on_content_created(data)
    if data.content_type == "product" then
        Host.log("info", "[my-plugin] new product: " .. data.id)
    end
    return data
end

function Plugin.on_cron_tick(data)
    if data.job_type == "daily_cleanup" then
        Host.dbExecute("DELETE FROM sessions WHERE expires_at < datetime('now')", nil)
        Host.log("info", "[my-plugin] cleanup done")
    end
end
```

### 关键约定

- 必须导出全局 `Plugin` 表
- Filter 钩子：接收 Lua table，返回修改后的 table（无需 JSON 序列化）
- 沙箱环境：仅暴露 `table`, `string`, `math`, `utf8`, `coroutine` 标准库（无 IO/OS/debug）
- 指令限制：5,000,000 条

## 错误恢复

- 连续 **5 次** 错误自动禁用插件
- 错误计数在成功执行时重置
- 可通过 Admin API 手动重新启用
- 插件超时/崩溃时自动回滚未提交的事务

## 热重载

当 `PLUGIN_HOT_RELOAD=true`（默认开启）时：

1. 文件系统监听器监控插件目录的 `.js` / `.lua` / `.wasm` 文件变化
2. 1 秒防抖
3. 自动卸载 + 重新加载变化的插件
4. 发出 `PluginReloaded` 事件

## 管理 API

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/api/v1/admin/plugins` | 列出所有插件（含状态/健康/指标） |
| GET | `/api/v1/admin/plugins/{id}` | 插件详情 |
| POST | `/api/v1/admin/plugins/{id}/enable` | 启用 |
| POST | `/api/v1/admin/plugins/{id}/disable` | 禁用 |
| POST | `/api/v1/admin/plugins/{id}/reload` | 热重载 |
| DELETE | `/api/v1/admin/plugins/{id}` | 卸载 |

## CLI 命令

```bash
# 创建新插件
rust-blog plugin new my-plugin --runtime js    # JavaScript
rust-blog plugin new my-plugin --runtime lua   # Lua
rust-blog plugin new my-plugin --runtime wasm  # WASM

# 校验插件
rust-blog plugin check                      # 校验默认目录
rust-blog plugin check ./plugins/my-plugin  # 校验指定目录
```

## 环境变量

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `PLUGIN_DIR` | `./extensions/plugins` | 插件目录 |
| `PLUGIN_VFS_ROOT` | `./plugins-data` | VFS 根目录 |
| `PLUGIN_VFS_MAX_FILE_SIZE` | `1048576` | 单文件最大 1MB |
| `PLUGIN_VFS_MAX_TOTAL_SIZE` | `10485760` | 总配额 10MB |
| `PLUGIN_WASM_POOL_SIZE` | `4` | WASM 实例池大小 |
| `PLUGIN_JS_POOL_SIZE` | `4` | JS 实例池大小 |
| `PLUGIN_LUA_POOL_SIZE` | `4` | Lua 实例池大小 |

## 完整示例：CRM 插件

```
plugins/crm/
  ├── manifest.toml
  └── main.js
```

**manifest.toml：**

```toml
[plugin]
id = "com.rust-blog.crm"
name = "CRM API"
version = "0.1.0"
description = "CRM sales funnel, Pipeline management, Contact timeline"
runtime = "js"
entry = "main.js"

[permissions]
max_memory_mb = 16
timeout_ms = 5000
database = ["crm_contacts", "crm_companies", "crm_deals", "crm_activities", "crm_notes"]
config = ["app.*"]

[hooks.on-content-created]
priority = 50

[hooks.on-content-updated]
priority = 50

[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/pipeline"
handler = "getPipeline"
auth = "admin"

[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/contacts"
handler = "listContacts"
auth = "admin"

[[routes]]
method = "POST"
path = "/api/v1/plugins/crm/contacts"
handler = "createContact"
auth = "admin"

[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/contacts/:contactId"
handler = "getContact"
auth = "admin"

[[routes]]
method = "GET"
path = "/api/v1/plugins/crm/dashboard"
handler = "getDashboard"
auth = "admin"
```

**main.js（节选）：**

```javascript
var Plugin = {};

var ok = function(data) {
  return JSON.stringify({ status: 200, body: JSON.stringify(data) });
};

var query = function(sql, params) {
  var result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
  if (!result || result.indexOf("error:") === 0) return null;
  return JSON.parse(result);
};

Plugin.getPipeline = function(input) {
  var stages = ["lead", "qualified", "proposal", "negotiation", "closed_won", "closed_lost"];
  var pipeline = [];
  for (var i = 0; i < stages.length; i++) {
    var rows = query(
      "SELECT COUNT(*) as cnt FROM crm_deals WHERE stage = ?",
      [stages[i]]
    );
    pipeline.push({ stage: stages[i], count: rows ? parseInt(rows[0].cnt) : 0 });
  }
  return ok({ stages: pipeline });
};

Plugin.createContact = function(input) {
  var data = JSON.parse(input);
  var body = JSON.parse(data.body);
  var id = Host.dbQuery("SELECT LOWER(HEX(RANDOMBLOB(16))) as id", null);
  var newId = JSON.parse(id)[0].id;
  Host.dbExecute(
    "INSERT INTO crm_contacts (id, name, email, company_id, status) VALUES (?, ?, ?, ?, ?)",
    JSON.stringify([newId, body.name, body.email || "", body.company_id || "", "active"])
  );
  return ok({ id: newId, name: body.name });
};

Plugin.on_content_created = function(input) {
  var data = JSON.parse(input);
  if (data.content_type === "crm_contacts") {
    Host.log("info", "[crm] new contact created: " + data.id);
    Host.emitEvent("crm.lead_created", JSON.stringify({ contactId: data.id }));
  }
  return ok(data);
};
```
