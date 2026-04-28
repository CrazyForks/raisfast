# Plugin ES Module + JS/Lua SDK 设计方案

## 1. 目标

将 JS/Lua 插件从全局脚本模式升级为模块化模式，引入版本化 SDK，让插件开发体验更现代、更安全。

### 1.1 JS 现状

```js
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        var rows = Host.dbQuery("SELECT ...", null);
        return JSON.stringify(input);
    }
};
```

问题：
- 输入/输出全部是 JSON 字符串，需要手动 parse/stringify
- 无模块化，每个插件都是单文件
- 工具函数（ok/err/query/exec）每个插件自己实现一遍

### 1.2 Lua 现状

```lua
Plugin = {}

function Plugin.stats_overview(inputJson)
    local data = json.decode(inputJson)
    local result = Host.dbQuery("SELECT ...", nil)
    return json.encode({ status = 200, body = json.encode({ ok = true, data = {} }) })
end
```

问题相同：无模块化、无工具函数、手动 JSON 处理。

### 1.3 目标

**JS 插件：**

```js
import { query, log, ok, err } from 'sdk';

export function on_content_creating(input) {
    const rows = query("SELECT * FROM posts WHERE id = ?", [input.id]);
    log("info", `found ${rows.length} rows`);
    input.title = input.title.toUpperCase();
    return input;
}
```

**Lua 插件：**

```lua
local sdk = require("sdk")
local query, log, ok, err = sdk.query, sdk.log, sdk.ok, sdk.err

function Plugin.on_content_creating(input)
    local rows = query("SELECT * FROM posts WHERE id = ?", { input.id })
    log("info", "found " .. #rows .. " rows")
    input.title = string.upper(input.title)
    return input
end
```

改进：
- JS：`import/export` 原生模块语法
- Lua：`require("sdk")` 模块加载
- SDK 自动处理 JSON 序列化/反序列化
- 内置工具函数 `ok/err/query/exec/routeParam/genId`
- 多文件支持（JS `import './utils.js'`，Lua `require("./utils")`）
- SDK 版本化管理

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

/// JS SDK v1 源码
pub const JS_SDK_V1: &str = include_str!("../../sdk/js_plugin_v1.js");
pub const JS_SDK_V1_VERSION: &str = "1.0.0";

/// Lua SDK v1 源码
pub const LUA_SDK_V1: &str = include_str!("../../sdk/lua_plugin_v1.lua");
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";
```

### 2.3 为什么嵌入二进制而非文件系统

| 方案 | 优点 | 缺点 |
|------|------|------|
| `include_str!` 嵌入 | 零配置、无版本漂移、部署简单 | 修改 SDK 需重新编译 |
| 文件系统 | 可热更新 | 版本不一致风险、部署需额外文件 |

选择 `include_str!`：
- SDK 是框架的 API 契约，不应被用户随意修改
- 避免生产环境 SDK 文件缺失或版本不匹配
- 与 Host API（Rust 代码）版本强绑定，一起发布

### 2.4 版本策略

SDK 版本号在源码和 Rust 两端都可见：

```rust
pub const JS_SDK_V1_VERSION: &str = "1.0.0";
pub const LUA_SDK_V1_VERSION: &str = "1.0.0";
```

```js
// js_plugin_v1.js
export const SDK_VERSION = "1.0.0";
```

```lua
-- lua_plugin_v1.lua
local M = {}
M.SDK_VERSION = "1.0.0"
```

未来新增 v2 时：
```rust
pub const JS_SDK_V2: &str = include_str!("../../sdk/js_plugin_v2.js");
pub const LUA_SDK_V2: &str = include_str!("../../sdk/lua_plugin_v2.lua");
```

---

## 3. JS 架构设计

### 3.1 Module Loader

自定义 `rquickjs::Loader`，处理两种标识符：

| 标识符 | 解析规则 | 示例 |
|--------|---------|------|
| `"sdk"` | 返回嵌入的 JS SDK 源码 | `import { query } from 'sdk'` |
| `"./xxx.js"` / `"../xxx.js"` | 相对于插件目录 | `import { helper } from './utils.js'` |

```rust
struct JsModuleLoader {
    plugin_dir: PathBuf,
    sdk_source: &'static str,
}

impl rquickjs::Loader for JsModuleLoader {
    fn load(&self, ctx: &rquickjs::Ctx, name: &str) -> rquickjs::Result<rquickjs::Module> {
        let source = match name {
            "sdk" => self.sdk_source.to_string(),
            n if n.starts_with("./") || n.starts_with("../") => {
                let path = self.plugin_dir.join(n);
                let canonical = path.canonicalize().map_err(...)?;
                if !canonical.starts_with(&self.plugin_dir) {
                    return Err(...); // 路径穿越防护
                }
                std::fs::read_to_string(&canonical).map_err(...)?
            }
            _ => return Err(...),
        };
        rquickjs::Module::declare(ctx, name, source)
    }
}
```

### 3.2 Export → Plugin 对象桥接

框架通过 `Plugin.on_xxx(jsonString)` 调用。JS 插件的 `export function` 被自动收集注册：

```rust
// engine_js.rs
ctx.with(|ctx| {
    register_host_functions(ctx.clone(), ...)?;
    ctx.set_loader(Rc::new(JsModuleLoader::new(plugin_dir, sdk_source)));

    let module = ctx.compile_module("index.js", code)?;
    module.eval::<()>()?;

    // 收集 export 函数 → Plugin 对象
    let ns = module.namespace();
    let plugin_obj = Object::new(ctx.clone())?;
    for key in ns.keys::<String>() {
        let func: Function = ns.get(&key)?;
        let wrapped = Self::wrap_export(ctx.clone(), func)?;
        plugin_obj.set(key, wrapped)?;
    }
    ctx.globals().set("Plugin", plugin_obj)?;
    Ok(())
}).await?;
```

### 3.3 JS 完整加载流程

```
PluginManager.load_plugin_from_dir()
    ├─ 解析 plugin.toml (sdk_version = "v1")
    ├─ 读取 index.js
    └─ JsEngine.create_instance(code, plugin_dir, JS_SDK_V1)
         ├─ AsyncRuntime + AsyncContext
         ├─ register_host_functions() → 全局 Host 对象
         ├─ set_loader(JsModuleLoader)
         ├─ compile_module("index.js", code) + eval()
         └─ 从 module namespace 收集 export → Plugin 对象
```

---

## 4. Lua 架构设计

### 4.1 模块加载

Lua 使用 `require()` + `package.loaders` 实现自定义模块加载。

mlua 支持 `Lua::load()` 和自定义搜索器（searcher）。实现方式：

```rust
struct LuaModuleLoader {
    plugin_dir: PathBuf,
    sdk_source: &'static str,
}

impl LuaModuleLoader {
    fn install(&self, lua: &Lua) -> anyhow::Result<()> {
        let sdk_source = self.sdk_source.to_string();
        let plugin_dir = self.plugin_dir.clone();

        // 注册自定义 searcher 到 package.loaders
        let searcher = lua.create_function(move |lua, name: String| {
            match name.as_str() {
                "sdk" => {
                    // 返回 loader 函数
                    let source = sdk_source.clone();
                    let loader = lua.create_function(move |lua, _| {
                        lua.load(&source).set_name("sdk")?.exec()?;
                        // sdk 模块需要返回 table
                        let module: mlua::Table = lua.globals().get("__sdk_module")?;
                        Ok(module)
                    })?;
                    Ok(mlua::Value::Function(loader))
                }
                n if n.starts_with("./") || n.starts_with("../") => {
                    let path = plugin_dir.join(&name);
                    let canonical = path.canonicalize().map_err(...)?;
                    if !canonical.starts_with(&plugin_dir) {
                        return Err(...);
                    }
                    let source = std::fs::read_to_string(&canonical)?;
                    let loader = lua.create_function(move |lua, _| {
                        lua.load(&source).set_name(&name)?.exec()?;
                        let module: mlua::Table = lua.globals().get("__module")?;
                        Ok(module)
                    })?;
                    Ok(mlua::Value::Function(loader))
                }
                _ => Ok(mlua::Value::Nil), // 未找到，交给下一个 searcher
            }
        })?;

        let package: mlua::Table = lua.globals().get("package")?;
        let loaders: mlua::Table = package.get("loaders")?;
        // 插入到 loaders 最前面（优先于默认 searcher）
        let len = loaders.len()?;
        for i in (1..=len).rev() {
            let v: mlua::Value = loaders.get(i)?;
            loaders.set(i + 1, v)?;
        }
        loaders.set(1, searcher)?;

        Ok(())
    }
}
```

### 4.2 Lua SDK 模块返回方式

Lua 的 `require()` 期望模块返回一个 table。SDK 源码末尾：

```lua
-- lua_plugin_v1.lua
local M = {}
M.query = function(sql, params) ... end
M.log = function(level, msg) ... end
-- ...
__sdk_module = M  -- 全局变量，供 searcher 的 loader 返回
return M
```

Lua 插件中使用：

```lua
local sdk = require("sdk")
local query = sdk.query
local log = sdk.log
```

### 4.3 Lua 相对路径模块

```lua
-- utils.lua（插件目录下）
local M = {}
function M.slugify(text)
    return string.lower(text):gsub("%s+", "-"):gsub("[^%w-]", "")
end
__module = M
return M
```

```lua
-- init.lua（插件入口）
local sdk = require("sdk")
local utils = require("./utils")
local query, log, ok = sdk.query, sdk.log, sdk.ok
```

### 4.4 Lua Export → Plugin 对象

Lua 没有 JS 的 module namespace 概念。Lua 插件仍然使用全局 `Plugin` table：

```lua
-- init.lua
local sdk = require("sdk")
local query, log, ok, err = sdk.query, sdk.log, sdk.ok, sdk.err

function Plugin.on_content_creating(input)
    local rows = query("SELECT COUNT(*) as cnt FROM posts")
    log("info", "total: " .. rows[1].cnt)
    input.title = string.upper(input.title)
    return input
end
```

与 JS 的区别：
- JS：`export function` → 框架自动收集到 Plugin 对象
- Lua：插件手动定义 `Plugin.xxx` function（不变）
- Lua SDK 通过 `require("sdk")` 导入工具函数，省去每个插件重复定义

### 4.5 Lua 完整加载流程

```
PluginManager.load_plugin_from_dir()
    ├─ 解析 plugin.toml (sdk_version = "v1")
    ├─ 读取 init.lua
    └─ LuaEngine.create_instance(code, plugin_dir, LUA_SDK_V1)
         ├─ create_sandboxed_lua()
         ├─ register_host_functions() → 全局 Host table
         ├─ LuaModuleLoader::install() → package.loaders 前置自定义 searcher
         ├─ lua.load(code).set_name("init.lua").exec()
         └─ Plugin table 已在全局（与现有逻辑一致）
```

---

## 5. SDK v1 API 设计

JS 和 Lua SDK 提供完全相同的 API，只是语法不同。

### 5.1 数据库

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `query` | `(sql, params?) → any[] \| null` | `(sql, params?) → table \| nil` | SELECT 查询 |
| `exec` | `(sql, params?) → { changes }` | `(sql, params?) → { changes }` | INSERT/UPDATE/DELETE |
| `beginTransaction` | `() → string` | `() → string` | 开启事务 |
| `commit` | `(txId) → void` | `(txId) → void` | 提交 |
| `rollback` | `(txId) → void` | `(txId) → void` | 回滚 |

### 5.2 HTTP

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `httpGet` | `(url) → any \| null` | `(url) → table \| nil` | GET 请求，自动 JSON 解析 |
| `httpPost` | `(url, body) → any \| null` | `(url, body) → table \| nil` | POST 请求 |

### 5.3 配置与存储

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `getConfig` | `(key) → string \| null` | `(key) → string \| nil` | 读取配置 |
| `getData` | `(key) → string \| null` | `(key) → string \| nil` | 读取插件数据 |
| `setData` | `(key, value) → boolean` | `(key, value) → boolean` | 写入插件数据 |

### 5.4 文件系统

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `fsRead` | `(path) → string \| null` | `(path) → string \| nil` | 读取 VFS 文件 |
| `fsWrite` | `(path, content) → boolean` | `(path, content) → boolean` | 写入 VFS 文件 |
| `fsDelete` | `(path) → boolean` | `(path) → boolean` | 删除文件 |
| `fsExists` | `(path) → boolean` | `(path) → boolean` | 检查存在 |
| `fsList` | `(path) → string[] \| null` | `(path) → table \| nil` | 列出目录 |

### 5.5 响应工具（自定义路由用）

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `ok` | `(data) → string` | `(data) → string` | 成功响应 JSON |
| `err` | `(status, msg) → string` | `(status, msg) → string` | 错误响应 JSON |
| `routeParam` | `(input, index?) → string` | `(input, index?) → string` | 路径参数提取 |
| `parseBody` | `(input) → object` | `(input) → table` | 解析请求体 |

### 5.6 通用工具

| 函数 | JS 签名 | Lua 签名 | 说明 |
|------|---------|----------|------|
| `log` | `(level, msg) → void` | `(level, msg) → void` | 日志 |
| `genId` | `() → string` | `() → string` | 生成 UUID |
| `emitEvent` | `(type, data) → string` | `(type, data) → string` | 发送事件 |

---

## 6. SDK 源码

### 6.1 JS SDK（`sdk/js_plugin_v1.js`）

```js
export const SDK_VERSION = "1.0.0";

// ── 数据库 ──
export function query(sql, params) {
    const result = Host.dbQuery(sql, params ? JSON.stringify(params) : null);
    if (!result || result.startsWith("error:")) return null;
    return JSON.parse(result);
}

export function exec(sql, params) {
    const result = Host.dbExecute(sql, params ? JSON.stringify(params) : null);
    return JSON.parse(result);
}

export function beginTransaction() { return Host.dbBegin(); }
export function commit(txId) { return Host.dbCommit(txId); }
export function rollback(txId) { return Host.dbRollback(txId); }

// ── HTTP ──
export function httpGet(url) {
    const result = Host.httpGet(url);
    if (!result) return null;
    try { return JSON.parse(result); } catch { return result; }
}

export function httpPost(url, body) {
    const json = typeof body === "string" ? body : JSON.stringify(body);
    const result = Host.httpPost(url, json);
    if (!result) return null;
    try { return JSON.parse(result); } catch { return result; }
}

// ── 配置 ──
export function getConfig(key) { return Host.getConfig(key); }

// ── 存储 ──
export function getData(key) { return Host.getData(key); }
export function setData(key, value) { return Host.setData(key, value); }

// ── 文件系统 ──
export function fsRead(path) { return Host.fsRead(path); }
export function fsWrite(path, content) { return Host.fsWrite(path, content); }
export function fsDelete(path) { return Host.fsDelete(path); }
export function fsExists(path) { return Host.fsExists(path); }
export function fsList(path) {
    const result = Host.fsList(path);
    return result ? result.split(",") : null;
}

// ── 响应工具 ──
export function ok(data) {
    return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data }) });
}

export function err(status, msg) {
    return JSON.stringify({ status, body: JSON.stringify({ ok: false, error: msg }) });
}

export function routeParam(input, index) {
    let obj = input;
    if (typeof input === "string") {
        try { obj = JSON.parse(input); } catch { return ""; }
    }
    let path = (obj.path || "").replace(/\/+$/, "");
    const qIdx = path.indexOf("?");
    if (qIdx >= 0) path = path.substring(0, qIdx);
    const parts = path.split("/");
    return parts[parts.length - (index || 1)];
}

export function parseBody(input) {
    try {
        if (typeof input === "string") {
            const parsed = JSON.parse(input);
            if (parsed && typeof parsed.body === "string" && parsed.body.charAt(0) === "{") {
                return JSON.parse(parsed.body);
            }
            return parsed;
        }
        if (input?.body) return JSON.parse(input.body);
        return {};
    } catch { return {}; }
}

// ── 通用 ──
export function log(level, msg) { Host.log(level, msg); }

export function genId() {
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
        const r = Math.random() * 16 | 0;
        return (c === "x" ? r : (r & 0x3 | 0x8)).toString(16);
    });
}

export function emitEvent(type, data) {
    return Host.emitEvent(type, typeof data === "string" ? data : JSON.stringify(data));
}
```

### 6.2 Lua SDK（`sdk/lua_plugin_v1.lua`）

```lua
-- Lua SDK v1
local M = {}
M.SDK_VERSION = "1.0.0"

-- ── 数据库 ──
function M.query(sql, params)
    local paramsJson = params and json.encode(params) or nil
    local result = Host.dbQuery(sql, paramsJson)
    if not result or result:find("^error:") then return nil end
    return json.decode(result)
end

function M.exec(sql, params)
    local paramsJson = params and json.encode(params) or nil
    local result = Host.dbExecute(sql, paramsJson)
    return json.decode(result)
end

function M.beginTransaction() return Host.dbBegin() end
function M.commit(txId) return Host.dbCommit(txId) end
function M.rollback(txId) return Host.dbRollback(txId) end

-- ── HTTP ──
function M.httpGet(url)
    local result = Host.httpGet(url)
    if not result then return nil end
    local ok, decoded = pcall(json.decode, result)
    return ok and decoded or result
end

function M.httpPost(url, body)
    local jsonBody = type(body) == "string" and body or json.encode(body)
    local result = Host.httpPost(url, jsonBody)
    if not result then return nil end
    local ok, decoded = pcall(json.decode, result)
    return ok and decoded or result
end

-- ── 配置 ──
function M.getConfig(key) return Host.getConfig(key) end

-- ── 存储 ──
function M.getData(key) return Host.getData(key) end
function M.setData(key, value) return Host.setData(key, value) end

-- ── 文件系统 ──
function M.fsRead(path) return Host.fsRead(path) end
function M.fsWrite(path, content) return Host.fsWrite(path, content) end
function M.fsDelete(path) return Host.fsDelete(path) end
function M.fsExists(path) return Host.fsExists(path) end
function M.fsList(path)
    local result = Host.fsList(path)
    if not result then return nil end
    local list = {}
    for part in result:gmatch("[^,]+") do
        table.insert(list, part)
    end
    return list
end

-- ── 响应工具 ──
function M.ok(data)
    return json.encode({ status = 200, body = json.encode({ ok = true, data = data }) })
end

function M.err(status, msg)
    return json.encode({ status = status, body = json.encode({ ok = false, error = msg }) })
end

function M.routeParam(input, index)
    local obj = input
    if type(input) == "string" then
        obj = json.decode(input)
    end
    local path = (obj.path or ""):gsub("/+$", "")
    local qIdx = path:find("?")
    if qIdx then path = path:sub(1, qIdx - 1) end
    local parts = {}
    for part in path:gmatch("[^/]+") do
        table.insert(parts, part)
    end
    return parts[#parts - (index or 1) + 1] or ""
end

function M.parseBody(input)
    local ok, result = pcall(function()
        if type(input) == "string" then
            local parsed = json.decode(input)
            if parsed and type(parsed.body) == "string" and parsed.body:sub(1,1) == "{" then
                return json.decode(parsed.body)
            end
            return parsed
        end
        if input and input.body then return json.decode(input.body) end
        return {}
    end)
    return ok and result or {}
end

-- ── 通用 ──
function M.log(level, msg) Host.log(level, msg) end

function M.genId()
    local template = "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx"
    return template:gsub("[xy]", function(c)
        local r = math.random(0, 15)
        if c == "x" then return string.format("%x", r)
        else return string.format("%x", r & 0x3 | 0x8) end
    end)
end

function M.emitEvent(eventType, data)
    local dataStr = type(data) == "string" and data or json.encode(data)
    return Host.emitEvent(eventType, dataStr)
end

__sdk_module = M
return M
```

---

## 7. Manifest 变更

```toml
[plugin]
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.0.0"
runtime = "js"               # "js" 或 "lua"
language = "js"              # "js" 或 "lua"
entry = "index.js"           # JS: index.js  Lua: init.lua
sdk_version = "v1"           # 新增：SDK 版本（默认 "v1"）
```

`sdk_version` 字段：
- 可选，默认 `"v1"`
- JS 插件加载 `JS_SDK_V1`，Lua 插件加载 `LUA_SDK_V1`
- 同一个版本号，不同 runtime 使用对应的 SDK 源码

---

## 8. 插件编写对比

### 8.1 JS 旧格式 → 新格式

**旧：**
```js
var Plugin = {};
var ok = function(r) { return JSON.stringify({ status: 200, body: JSON.stringify({ ok: true, data: r }) }); };
var query = function(sql, p) { var r = Host.dbQuery(sql, p ? JSON.stringify(p) : null); if (!r || r.indexOf("error:") === 0) return null; return JSON.parse(r); };

Plugin.getDashboardStats = function(inputJson) {
    var data = JSON.parse(inputJson);
    var result = query("SELECT COUNT(*) as cnt FROM crm_contacts");
    if (!result) return err(500, "query failed");
    return ok({ total: result[0].cnt });
};
```

**新：**
```js
import { query, ok, err } from 'sdk';

export function getDashboardStats(input) {
    const result = query("SELECT COUNT(*) as cnt FROM crm_contacts");
    if (!result) return err(500, "query failed");
    return ok({ total: result[0].cnt });
}
```

### 8.2 Lua 旧格式 → 新格式

**旧：**
```lua
Plugin = {}

function Plugin.stats_overview(inputJson)
    local result = Host.dbQuery("SELECT COUNT(*) as cnt FROM posts", nil)
    if not result then return json.encode({ status = 500, body = json.encode({ ok = false, error = "query failed" }) }) end
    local data = json.decode(result)
    return json.encode({ status = 200, body = json.encode({ ok = true, data = { total = data[1].cnt } }) })
end
```

**新：**
```lua
local sdk = require("sdk")
local query, ok, err = sdk.query, sdk.ok, sdk.err

function Plugin.stats_overview(input)
    local result = query("SELECT COUNT(*) as cnt FROM posts")
    if not result then return err(500, "query failed") end
    return ok({ total = result[1].cnt })
end
```

---

## 9. 多文件插件

### 9.1 JS 多文件

```
my-plugin/
  plugin.toml
  index.js
  utils.js
```

```js
// utils.js
export function slugify(text) {
    return text.toLowerCase().replace(/\s+/g, '-').replace(/[^\w-]/g, '');
}

// index.js
import { slugify } from './utils.js';
import { query, log } from 'sdk';

export function on_content_creating(input) {
    input.slug = slugify(input.title);
    log("info", `slug: ${input.slug}`);
    return input;
}
```

### 9.2 Lua 多文件

```
my-plugin/
  plugin.toml
  init.lua
  utils.lua
```

```lua
-- utils.lua
local M = {}
function M.slugify(text)
    return string.lower(text):gsub("%s+", "-"):gsub("[^%w-]", "")
end
__module = M
return M

-- init.lua
local sdk = require("sdk")
local utils = require("./utils")
local query, log = sdk.query, sdk.log

function Plugin.on_content_creating(input)
    input.slug = utils.slugify(input.title)
    log("info", "slug: " .. input.slug)
    return input
end
```

---

## 10. 实现步骤

### Phase 1: JS Module Loader
1. 创建 `sdk/js_plugin_v1.js`（SDK 源码）
2. 创建 `src/plugins/sdk_v1.rs`（`include_str!` 常量）
3. 实现 `JsModuleLoader`（rquickjs `Loader` trait）
4. `engine_js.rs` 改用 `compile_module` + `eval`
5. 从 module namespace 收集 export → Plugin 对象

### Phase 2: Lua Module Loader
1. 创建 `sdk/lua_plugin_v1.lua`（SDK 源码）
2. 实现 `LuaModuleLoader`（自定义 `package.loaders` searcher）
3. `engine_lua.rs` 加载时安装 searcher

### Phase 3: Manifest + PluginManager
1. `manifest.rs` 新增 `sdk_version` 字段
2. `plugins.rs` 根据版本选择 SDK 源码
3. 传递 `plugin_dir` 和 `sdk_source` 到引擎

### Phase 4: 迁移现有插件
1. `crm/main.js` → JS ESM 格式
2. `ecommerce/main.js` → JS ESM 格式
3. `forum/main.js` → JS ESM 格式
4. `first-ext/init.lua` → Lua SDK 格式
5. 更新 `docs/plugin-dev-guide.md`

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
- Lua 沙箱环境不包含 `package` 标准库（`StdLib` 未包含），需要手动创建 `package.loaders` 表
- 相对路径模块需要约定 `__module` 全局变量来返回模块 table
- Lua 没有原生的模块隔离，所有 `require` 的模块共享全局作用域

### 12.3 性能
- SDK 通过 `include_str!` 嵌入，零 I/O 开销
- JS Module 编译有缓存（rquickjs 内部）
- Lua `require` 有内置缓存（加载一次后缓存结果）

### 12.4 安全
- SDK 不可被插件覆盖（Loader/searcher 优先匹配 `"sdk"`）
- 相对路径限制在插件目录内（`canonicalize` + `starts_with` 防路径穿越）
- Host API 权限校验不变

### 12.5 JS/Lua SDK API 一致性
- 两个 SDK 提供**完全相同的 API 名称和行为**
- 差异仅在于语言特性（JS `null` vs Lua `nil`，JS 数组 vs Lua 1-indexed table）
- 便于用户在两种语言间切换
