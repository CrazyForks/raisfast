# Plugin ES Module + SDK 设计方案

## 1. 目标

将 JS/Lua 插件从全局脚本模式升级为模块化模式，引入版本化 SDK，让插件开发体验更现代、更安全。

### 1.1 核心改进

- **JS**：`import/export` 原生 ESM 语法，引擎通过 rquickjs Module Loader 自动收集 export 函数
- **Lua**：`require("sdk")` 模块加载，通过 Rust 侧 `Host.jsonEncode/Decode` 避免 Lua 端手写 JSON 解析器
- **SDK 命名体系**：统一 `namespaceAction` 风格（`dbQuery`、`httpGet`、`configGet`、`storeSet`、`vfsRead`、`logInfo`、`eventEmit`）
- **响应模型**：`ok(data)` 直接返回数据，框架统一包装为 `{code:0, data}`；`fail(status, msg)` 返回错误标记
- **路由参数**：引擎侧 `extract_route_params()` 提取命名参数注入 `input.params`，插件通过 `extractJson(input, "params.dealId")` 安全获取
- **ID 生成**：`Host.newId()` 调用 Rust 侧 `Uuid::now_v7()`，与系统其他部分一致
- **JSON 提取**：`extractJson(input, "params.id")` 支持点号路径，链路中任何一级不存在安全返回 `null`
- **并发模型**：JS/Lua 采用 per-request 模式（每次调用新建 VM），WASM 采用实例池

---

## 2. SDK 分发方式

SDK 是框架的一部分，**编译进 Rust 二进制文件**，不依赖外部文件。

### 2.1 文件结构

```
plugin-sdk/
  js/
    js_plugin_v1.js      ← JS SDK v1 源码
    js_plugin_v1.ts      ← TypeScript 源码（编译用）
    tsconfig.json         ← TypeScript 配置
  lua/
    lua_plugin_v1.lua    ← Lua SDK v1 源码
src/plugins/
  sdk_v1.rs              ← include_str! 常量
```

### 2.2 嵌入方式

```rust
// src/plugins/sdk_v1.rs
pub const JS_SDK_V1: &str = include_str!("../../plugin-sdk/js/js_plugin_v1.js");
pub const JS_SDK_V1_VERSION: &str = "1.0.0";
pub const LUA_SDK_V1: &str = include_str!("../../plugin-sdk/lua/lua_plugin_v1.lua");
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";
```

### 2.3 版本路由

```rust
pub fn get_sdk_source(runtime: &str, version: &str) -> Option<&'static str> {
    match (runtime, version) {
        ("js", "v1") => Some(JS_SDK_V1),
        ("lua", "v1") => Some(LUA_SDK_V1),
        _ => None,
    }
}
```

---

## 3. JS 架构

### 3.1 per-request 模式

JS 引擎采用**方案 D（per-request）**：每次调用创建全新 QuickJS context，用完销毁。零状态泄漏，无需锁竞争，完美隔离。

```
PluginManager.call_filter("my-plugin", "on_content_creating", input)
  → JsEngine.call_filter()
    → new AsyncContext from pre-compiled module
    → extract function from Plugin object
    → call function(input) with interrupt handler + outer timeout
    → drop context (释放所有内存)
```

### 3.2 Module Loader

自定义 `rquickjs::Loader`，处理两种标识符：

| 标识符 | 解析规则 | 示例 |
|--------|---------|------|
| `"sdk"` | 返回嵌入的 JS SDK 源码 | `import { dbQuery } from 'sdk'` |
| `"./xxx.js"` / `"../xxx.js"` | 相对于插件目录（canonicalize 防路径穿越） | `import { helper } from './utils.js'` |

### 3.3 Export → Plugin 对象桥接

```text
compile_module("main.js", code) + eval()
    ↓
module.namespace() → 遍历 keys → 收集 Function → 注册到 Plugin 对象
    ↓
框架通过 Plugin.on_xxx(input) 调用
```

### 3.4 JS 返回值处理

JS 函数直接返回 JS 对象（不再 `JSON.stringify`），引擎通过 `ctx.json_stringify()` 在 C 层序列化为 `serde_json::Value`，省去 JS 端字符串分配和 GC 压力。

### 3.5 Host 函数注册

Host 函数注册到 `PLUGIN_HOST_GLOBAL`（`"RaisFastHost"`）全局对象：

```rust
global.set(PLUGIN_HOST_GLOBAL, host)?;
```

20 个 Host 函数（详见 plugin-dev-guide.md Host API 章节）。

### 3.6 内存与超时

```rust
runtime.set_memory_limit(max_memory_bytes);  // 按 permissions.max_memory_mb
runtime.run_gc();                            // 每次调用后强制 GC
set_interrupt_handler(|| is_timed_out());    // 同步中断
tokio::time::timeout(duration, ...);         // 异步外层超时
```

---

## 4. Lua 架构

### 4.1 per-request 模式

Lua 引擎同样采用**方案 D（per-request）**：每次调用创建全新 Lua VM，用完销毁。

### 4.2 模块加载

Lua 通过 Rust 侧注册的自定义 `require` 函数加载模块：

- `"sdk"` → 执行嵌入的 Lua SDK 源码，通过 `_sdk_module` 全局变量返回 table
- `"./xxx"` → 读取插件目录下的文件并执行

### 4.3 Lua SDK 依赖 Host.jsonEncode/Decode

Lua 沙箱环境不含 `json` 库。SDK 内部通过 Rust 侧暴露的 `Host.jsonEncode(val)` / `Host.jsonDecode(str)` 进行 JSON 序列化，避免在 Lua 端手写解析器。

### 4.4 Lua Export → Plugin 对象

Lua 没有 JS 的 module namespace 概念。Lua 插件仍使用全局 `Plugin` table：

```lua
local sdk = require("sdk")
Plugin = {}
Plugin.on_content_creating = function(input) ... end
```

### 4.5 Host 函数

Lua Host 注册 22 个函数（比 JS 多 `jsonEncode` / `jsonDecode`），同样注册到 `PLUGIN_HOST_GLOBAL` 全局表。

---

## 5. WASM 架构

### 5.1 实例池模式

WASM 引擎采用实例池模式：启动时预编译 N 个 wasmtime 实例，通过 `Semaphore` + round-robin 分发。

```rust
pub struct WasmInstancePool {
    instances: Vec<PooledInstance>,
    semaphore: Semaphore,  // 限制并发数 = pool_size
}
```

### 5.2 Host 函数

WASM Host 通过 WIT 包 `raisfast:plugin-wit` 定义接口，使用 snake_case 命名（`db_query`、`vfs_read`）。

WASM Host 暴露 19 个函数，**不包含 `new_uuid`**（JS/Lua 的 `newId` 在 WASM 中不可用）。

---

## 6. SDK v1 API 设计

JS 和 Lua SDK 提供完全相同的 API（28 个函数），只是语法不同。

### 6.1 数据库（5 个）

| 函数 | 说明 |
|------|------|
| `dbQuery(sql, params?)` | 参数化 SELECT；错误时抛异常 |
| `dbExec(sql, params?)` | INSERT/UPDATE/DELETE，返回 `{error?, rows_affected}` |
| `dbBegin()` | 开启事务（失败时抛异常） |
| `dbCommit()` | 提交事务（失败时抛异常） |
| `dbRollback()` | 回滚事务 |

### 6.2 HTTP（4 个）

| 函数 | 说明 |
|------|------|
| `httpGet(url)` | GET 请求，返回原始字符串 |
| `httpGetJson(url)` | GET 请求，自动解析 JSON |
| `httpPost(url, body)` | POST 请求，返回原始字符串 |
| `httpPostJson(url, body)` | POST 请求，自动解析 JSON |

### 6.3 配置与存储（3 个）

| 函数 | 说明 |
|------|------|
| `configGet(key)` | 读取配置 |
| `storeGet(key)` | 读取 KV 存储 |
| `storeSet(key, value)` | 写入 KV 存储 |

### 6.4 虚拟文件系统（6 个）

| 函数 | 说明 |
|------|------|
| `vfsRead(path)` | 读取 VFS 文件 |
| `vfsWrite(path, content)` | 写入 VFS 文件 |
| `vfsDelete(path)` | 删除文件 |
| `vfsExists(path)` | 检查存在 |
| `vfsList(path)` | 列出目录 |
| `vfsStat(path)` | 获取文件信息（size, is_dir, modified） |

### 6.5 内容查询（1 个）

| 函数 | 说明 |
|------|------|
| `getPost(slug)` | 按 slug 获取文章，返回 JSON 对象 |

### 6.6 响应工具（3 个）

| 函数 | 说明 |
|------|------|
| `ok(data)` | 成功响应：返回数据，框架包装为 `{code:0, data}` |
| `fail(status, msg)` | 错误响应：框架包装为 `{code:N, message}` |
| `extractJson(input, field?)` | 从 JSON 提取字段（支持 `params.id` 点号路径），不存在返回 `null` |

### 6.7 通用工具（6 个）

| 函数 | 说明 |
|------|------|
| `logInfo(msg)` / `logWarn(msg)` / `logError(msg)` | 日志输出 |
| `newId()` | 生成 UUID v7（时间排序，与系统一致） |
| `eventEmit(type, data)` | 发射事件 |

---

## 7. Manifest

```toml
[plugin]
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.0.0"
runtime = "js"               # "js" / "lua" / "wasm"
entry = "main.js"            # JS: main.js  Lua: init.lua  WASM: plugin.wasm
sdk_version = "v1"           # 可选，默认 "v1"

[permissions]
max_memory_mb = 16
timeout_ms = 5000
database = ["products"]
config = ["app.*"]

[dependencies]
"com.raisfast.auth" = ">=1.0.0"

[hooks.on-content-creating]
priority = 50
match = "product"
content_types = ["product"]

[[cron]]
label = "每日统计"
job_type = "daily_stats"
cron_expr = "0 0 * * *"

[[routes]]
method = "GET"
path = "/api/v1/plugins/my-plugin/stats"
handler = "getStats"
auth = "public"

[[routes.input]]
name = "page"
type = "integer"
in = "query"
default = 1

[[routes.output.fields]]
name = "total"
type = "integer"

[[content_types]]
file = "content_types/contact.toml"

[[admin_pages]]
path = "/admin/plugins/my-plugin"
label = "My Plugin"
icon = "puzzle"
```

---

## 8. 插件编写对比

### 8.1 JS

```javascript
import { dbQuery, dbExec, ok, fail, extractJson, logInfo, newId } from 'sdk';

export function on_content_creating(input) {
    const data = extractJson(input, "body");
    if (data?.title) data.title = data.title.toUpperCase();
    return ok(data);
}

export function getProduct(input) {
    const id = extractJson(input, "params.id");
    if (!id) return fail(400, "id required");
    const rows = dbQuery("SELECT * FROM products WHERE id = ?", [id]);
    return ok(rows[0]);
}
```

### 8.2 Lua

```lua
local sdk = require("sdk")
Plugin = {}

Plugin.on_content_creating = function(input)
    local data = sdk.extractJson(input, "body")
    if data and data.title then
        data.title = string.upper(data.title)
    end
    return sdk.ok(data)
end

Plugin.getProduct = function(input)
    local id = sdk.extractJson(input, "params.id")
    if not id then return sdk.fail(400, "id required") end
    local rows = sdk.dbQuery("SELECT * FROM products WHERE id = ?", { id })
    return sdk.ok(rows[1])
end
```

---

## 9. 路由参数

引擎侧在 `dispatch_route` 中通过 `extract_route_params()` 提取命名参数，注入 `input.params`：

```text
path:    /api/v1/plugins/crm/pipeline/deal-123
pattern: /api/v1/plugins/crm/pipeline/:dealId
→ input.params = {"dealId": "deal-123"}
```

插件通过 `extractJson(input, "params.dealId")` 安全获取，点号路径中任何一级不存在返回 `null`。

---

## 10. 响应处理

框架在 `call_plugin_json` 中统一处理：

1. 检查返回值是否包含 `__plugin_error: true` → 返回 `{code: status*100, message, data: null}`
2. 否则直接包装为 `{code: 0, message: "success", data: result}`

---

## 11. 向后兼容

**不兼容**。新格式不向后兼容。

- JS：必须使用 `export function`（不再支持 `var Plugin = {}`）
- Lua：仍使用 `Plugin.xxx` function，但工具函数从 SDK 导入
- `Host` 全局对象仍存在（SDK 内部使用），不推荐插件直接调用

---

## 12. 风险与注意事项

### 12.1 QuickJS ESM 限制
- `import` 在 `eval_module` 中同步执行
- 不支持 `import.meta.url`
- 不支持动态 `import()`

### 12.2 Lua 限制
- 沙箱不含 `package` 标准库，自定义 `require` 函数替代
- 相对路径模块需要约定 `_sdk_module` 全局变量返回 SDK table
- 无原生模块隔离，所有 `require` 的模块共享全局作用域

### 12.3 安全
- SDK 不可被插件覆盖（Loader/require 优先匹配 `"sdk"`）
- 相对路径限制在插件目录内（canonicalize + starts_with 防路径穿越）
- Host API 权限校验不变
- 全局对象名 `PLUGIN_HOST_GLOBAL = "RaisFastHost"`，不使用 `Host` 避免与用户代码冲突

### 12.4 JS/Lua SDK API 一致性
- 两个 SDK 提供完全相同的 28 个 API 名称和行为
- 差异仅在于语言特性（JS `null` vs Lua `nil`，JS 数组 vs Lua 1-indexed table）

### 12.5 三运行时 Host 函数差异
- JS/Lua 用 camelCase（`dbQuery`、`vfsRead`）
- WASM 用 snake_case（`db_query`、`vfs_read`）
- WASM 不暴露 `newId`/`new_uuid`
- Lua 额外暴露 `jsonEncode`/`jsonDecode`（无原生 JSON 支持）
