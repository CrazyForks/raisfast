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

---

## 2. SDK 分发方式

SDK 是框架的一部分，**编译进 Rust 二进制文件**，不依赖外部文件。

### 2.1 文件结构

```
sdk/
  js_plugin_v1.js      ← JS SDK v1
  lua_plugin_v1.lua    ← Lua SDK v1
src/plugins/
  sdk_v1.rs            ← include_str! 常量
```

### 2.2 嵌入方式

```rust
// src/plugins/sdk_v1.rs
pub const JS_SDK_V1: &str = include_str!("../../sdk/js_plugin_v1.js");
pub const JS_SDK_V1_VERSION: &str = "1.0.0";
pub const LUA_SDK_V1: &str = include_str!("../../sdk/lua_plugin_v1.lua");
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";
```

---

## 3. JS 架构

### 3.1 Module Loader

自定义 `rquickjs::Loader`，处理两种标识符：

| 标识符 | 解析规则 | 示例 |
|--------|---------|------|
| `"sdk"` | 返回嵌入的 JS SDK 源码 | `import { dbQuery } from 'sdk'` |
| `"./xxx.js"` / `"../xxx.js"` | 相对于插件目录 | `import { helper } from './utils.js'` |

### 3.2 Export → Plugin 对象桥接

```text
compile_module("index.js", code) + eval()
    ↓
module.namespace() → 遍历 keys → 收集 Function → 注册到 Plugin 对象
    ↓
框架通过 Plugin.on_xxx(input) 调用
```

### 3.3 JS 返回值处理

JS 函数直接返回 JS 对象（不再 `JSON.stringify`），引擎通过 `ctx.json_stringify()` 在 C 层序列化为 `serde_json::Value`，省去 JS 端字符串分配和 GC 压力。

### 3.4 JS 完整加载流程

```text
PluginManager.load_plugin_from_dir()
    ├─ 解析 plugin.toml (sdk_version = "v1")
    ├─ 读取 main.js
    └─ JsEngine.create_instance(code, plugin_dir, JS_SDK_V1)
         ├─ AsyncRuntime + AsyncContext
         ├─ register_host_functions() → 全局 Host 对象
         ├─ set_loader(JsModuleLoader)
         ├─ compile_module("main.js", code) + eval()
         └─ 从 module namespace 收集 export → Plugin 对象
```

---

## 4. Lua 架构

### 4.1 模块加载

Lua 通过 Rust 侧注册的自定义 `require` 函数加载模块：

- `"sdk"` → 执行嵌入的 Lua SDK 源码，通过 `_sdk_module` 全局变量返回 table
- `"./xxx"` → 读取插件目录下的文件并执行

### 4.2 Lua SDK 依赖 Host.jsonEncode/Decode

Lua 沙箱环境不含 `json` 库。SDK 内部通过 Rust 侧暴露的 `Host.jsonEncode(val)` / `Host.jsonDecode(str)` 进行 JSON 序列化，避免在 Lua 端手写解析器。

### 4.3 Lua Export → Plugin 对象

Lua 没有 JS 的 module namespace 概念。Lua 插件仍使用全局 `Plugin` table：

```lua
local sdk = require("sdk")
Plugin = {}
Plugin.on_content_creating = function(input) ... end
```

---

## 5. SDK v1 API 设计

JS 和 Lua SDK 提供完全相同的 API，只是语法不同。

### 5.1 数据库

| 函数 | 说明 |
|------|------|
| `dbQuery(sql, params?)` | 参数化 SELECT；错误时抛异常 |
| `dbExec(sql, params?)` | INSERT/UPDATE/DELETE，返回 `{error?, rows_affected}` |
| `dbBegin()` | 开启事务（失败时抛异常） |
| `dbCommit()` | 提交事务（失败时抛异常） |
| `dbRollback()` | 回滚事务 |

### 5.2 HTTP

| 函数 | 说明 |
|------|------|
| `httpGet(url)` | GET 请求，返回原始字符串 |
| `httpGetJson(url)` | GET 请求，自动解析 JSON |
| `httpPost(url, body)` | POST 请求，返回原始字符串 |
| `httpPostJson(url, body)` | POST 请求，自动解析 JSON |

### 5.3 配置与存储

| 函数 | 说明 |
|------|------|
| `configGet(key)` | 读取配置 |
| `storeGet(key)` | 读取 KV 存储 |
| `storeSet(key, value)` | 写入 KV 存储 |

### 5.4 虚拟文件系统

| 函数 | 说明 |
|------|------|
| `vfsRead(path)` | 读取 VFS 文件 |
| `vfsWrite(path, content)` | 写入 VFS 文件 |
| `vfsDelete(path)` | 删除文件 |
| `vfsExists(path)` | 检查存在 |
| `vfsList(path)` | 列出目录 |

### 5.5 响应工具

| 函数 | 说明 |
|------|------|
| `ok(data)` | 成功响应：返回数据，框架包装为 `{code:0, data}` |
| `fail(status, msg)` | 错误响应：框架包装为 `{code:N, message}` |
| `extractJson(input, field?)` | 从 JSON 提取字段（支持 `params.id` 点号路径），不存在返回 `null` |

### 5.6 通用工具

| 函数 | 说明 |
|------|------|
| `logInfo(msg)` / `logWarn(msg)` / `logError(msg)` | 日志输出 |
| `newId()` | 生成 UUID v7（时间排序，与系统一致） |
| `eventEmit(type, data)` | 发射事件 |

---

## 6. Manifest

```toml
[plugin]
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.0.0"
runtime = "js"               # "js" 或 "lua"
entry = "main.js"            # JS: main.js  Lua: init.lua
sdk_version = "v1"           # 可选，默认 "v1"
```

---

## 7. 插件编写对比

### 7.1 JS

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

### 7.2 Lua

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

## 8. 路由参数

引擎侧在 `dispatch_route` 中通过 `extract_route_params()` 提取命名参数，注入 `input.params`：

```text
path:    /api/v1/plugins/crm/pipeline/deal-123
pattern: /api/v1/plugins/crm/pipeline/:dealId
→ input.params = {"dealId": "deal-123"}
```

插件通过 `extractJson(input, "params.dealId")` 安全获取，点号路径中任何一级不存在返回 `null`。

---

## 9. 响应处理

框架在 `call_plugin_json` 中统一处理：

1. 检查返回值是否包含 `__plugin_error: true` → 返回 `{code: status*100, message, data: null}`
2. 否则直接包装为 `{code: 0, message: "success", data: result}`

---

## 10. 向后兼容

**不兼容**。新格式不向后兼容。

- JS：必须使用 `export function`（不再支持 `var Plugin = {}`）
- Lua：仍使用 `Plugin.xxx` function，但工具函数从 SDK 导入
- `Host` 全局对象仍存在（SDK 内部使用），不推荐插件直接调用

---

## 11. 风险与注意事项

### 11.1 QuickJS ESM 限制
- `import` 在 `eval_module` 中同步执行
- 不支持 `import.meta.url`
- 不支持动态 `import()`

### 11.2 Lua 限制
- 沙箱不含 `package` 标准库，自定义 `require` 函数替代
- 相对路径模块需要约定 `_sdk_module` 全局变量返回 SDK table
- 无原生模块隔离，所有 `require` 的模块共享全局作用域

### 11.3 安全
- SDK 不可被插件覆盖（Loader/require 优先匹配 `"sdk"`）
- 相对路径限制在插件目录内（canonicalize + starts_with 防路径穿越）
- Host API 权限校验不变

### 11.4 JS/Lua SDK API 一致性
- 两个 SDK 提供完全相同的 API 名称和行为
- 差异仅在于语言特性（JS `null` vs Lua `nil`，JS 数组 vs Lua 1-indexed table）
