# mlua Lua 插件方案

> 在现有 WASM + JS 双引擎插件系统基础上，新增 Lua (mlua) 作为第三运行时，
> 提供最轻量的插件开发选择，原生 serde 集成，零序列化开销。

---

## 1. 背景

### 1.1 现状

当前插件系统支持两种运行时：

- **WASM (wasmtime)** — Rust 编写，编译为 WASM，安全隔离最强
- **JavaScript (rquickjs)** — JS/TS 编写，社区生态最大

### 1.2 为什么需要 Lua

| 场景 | WASM | JS | Lua |
|------|------|----|-----|
| 嵌入式/边缘部署 | 体积大（+15MB） | 中等（+2-4MB） | **最小（+0.5-1MB）** |
| 编译时间增量 | 基准 | +2-5 min | **+30-60s** |
| 数据传递 | 手动 ABI（指针+长度前缀） | JSON 字符串中转 | **原生 serde（零拷贝 table 映射）** |
| 游戏引擎/Nginx/Redis 用户 | — | — | **Lua 是通用脚本语言** |
| 配置型插件 | 过重 | 可用 | **最适合（table = config）** |
| 沙箱控制 | fuel | interrupt handler | **细粒度 stdlib 裁剪** |

### 1.3 目标

- 支持用 Lua 编写插件，体积和编译时间影响最小
- 原生 serde 集成 — Rust 结构体直接映射为 Lua table，无 JSON 中转
- 与现有 WASM/JS 插件共享 Hook 调度机制
- 安全隔离（内存限制、指令计数超时、stdlib 裁剪）

### 1.4 选型对比

| 维度 | wasmtime (WASM) | rquickjs (JS) | **mlua (Lua)** |
|------|----------------|---------------|----------------|
| 引擎体积 (release) | ~15MB | ~2-4MB | **~0.5-1MB** |
| 编译时间增量 | 基准 | +2-5 min | **+30-60s** |
| 依赖 crate 数 | ~280 | ~5 | **~3** |
| 冷启动 | ~5-50ms | ~1-5ms | **<1ms** |
| 内存/实例 | ~1-2MB | ~200-500KB | **~50-100KB** |
| 数据传递 | 手动 ABI | JSON 字符串 | **serde 原生映射** |
| 沙箱粒度 | fuel | interrupt handler | **stdlib 裁剪 + hook** |
| 语言生态 | Rust | JavaScript（全球最大） | Lua（游戏/运维/嵌入式） |
| async 支持 | N/A | AsyncRuntime | **coroutine + call_async** |

**选择 mlua 的核心理由：**

1. **最轻量** — Lua 5.4 编译后 ~300KB，对嵌入式场景友好
2. **原生 serde** — `LuaSerdeExt::to_value/from_value`，Rust struct ↔ Lua table 零开销
3. **沙箱成熟** — 按需加载 stdlib（排除 `io`/`os`/`debug`/`package`）
4. **活跃维护** — 0.11.6（2026-01），支持 Lua 5.1-5.5、LuaJIT、Luau
5. **API 稳定** — `send` feature 使其 `Send+Sync`，直接在 tokio 中使用

---

## 2. 架构设计

### 2.1 模块结构

```
src/plugins/
├── mod.rs            # PluginManager（三引擎统一分派）
├── manifest.rs       # 复用 manifest（runtime 字段区分引擎）
├── engine.rs         # WASM 引擎（不变）
├── host.rs           # WASM host functions（不变）
├── engine_js.rs      # QuickJS 引擎（不变）
├── js_host.rs        # JS 宿主函数（不变）
├── engine_lua.rs     # Lua 引擎（新增）
└── lua_host.rs       # Lua 宿主函数（新增）
```

### 2.2 类型体系

```rust
// mod.rs — 插件实例枚举
enum LoadedPluginInstance {
    Wasm(Box<RwLock<WasmInstance>>),
    Js(String),      // plugin_id
    Lua(String),     // plugin_id
}

pub struct PluginManager {
    #[cfg(feature = "plugin-wasm")]
    engine: wasmtime::Engine,
    #[cfg(feature = "plugin-js")]
    js_engine: JsEngine,
    #[cfg(feature = "plugin-lua")]
    lua_engine: LuaEngine,
    plugins: RwLock<HashMap<String, LoadedPlugin>>,
    config: Arc<AppConfig>,
    // ...
}
```

### 2.3 三引擎分派

```rust
impl PluginManager {
    pub async fn dispatch_filter<T>(&self, hook: HookPoint, input: T) -> AppResult<T> {
        let plugins = self.plugins.read().await;
        let mut sorted: Vec<_> = plugins.values().collect();
        sorted.sort_by_key(/* priority */);

        let mut current = input;
        for plugin in sorted {
            let func_name = hook.wasm_func_name();
            if !plugin.manifest.hooks.contains_key(func_name) { continue; }

            match &plugin.instance {
                LoadedPluginInstance::Wasm(wasm) => {
                    // 现有 WASM 分派
                }
                LoadedPluginInstance::Js(id) => {
                    // 现有 JS 分派
                }
                LoadedPluginInstance::Lua(id) => {
                    match self.lua_engine
                        .call_filter(id, func_name, &current)
                        .await
                    {
                        Ok(Some(result)) => current = result,
                        Ok(None) => {}
                        Err(e) => tracing::warn!("lua plugin {} hook {} failed: {e}", ...),
                    }
                }
            }
        }
        Ok(current)
    }
}
```

### 2.4 线程模型

```
┌──────────────────────────────────────────────┐
│              tokio runtime                    │
│                                              │
│  PluginManager (Arc)                         │
│  ├── wasm_engine: wasmtime::Engine           │
│  ├── js_engine: JsEngine                     │
│  │   └── AsyncRuntime (Arc<Mutex>)           │
│  ├── lua_engine: LuaEngine                   │
│  │   └── states: Mutex<HashMap<String,Lua>>  │
│  └── plugins: RwLock<HashMap>                │
│                                              │
│  axum handler                                │
│  └── dispatch_filter()                       │
│      ├── WASM: sync call                     │
│      ├── JS: ctx.with().await                │
│      └── Lua: lua.lock().call_async().await  │
│                                              │
└──────────────────────────────────────────────┘
```

- `mlua::Lua` 配合 `send` feature 后为 `Send+Sync`（内部 `Arc<ReentrantMutex>`）
- 每个插件一个 `Lua` 实例（独立全局作用域）
- 异步调用通过 `call_async()` 在 tokio 中执行
- 不需要独立线程

---

## 3. 引擎实现

### 3.1 LuaEngine

```rust
// engine_lua.rs
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use mlua::{Lua, LuaOptions, StdLib, HookTriggers, Error as LuaError, VmState, LuaSerdeExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::config::app::AppConfig;

pub struct LuaEngine {
    states: Mutex<HashMap<String, Lua>>,
    config: Arc<AppConfig>,
    timeout_instructions: i64,
}

impl LuaEngine {
    pub fn new(config: &AppConfig) -> anyhow::Result<Self> {
        Ok(Self {
            states: Mutex::new(HashMap::new()),
            config: Arc::new(config.clone()),
            // 默认 500 万条指令（约 2-5 秒）
            timeout_instructions: 5_000_000,
        })
    }

    /// 创建受限 Lua 状态
    fn create_sandboxed_lua(memory_limit_bytes: usize) -> anyhow::Result<Lua> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(memory_limit_bytes)?;
        Ok(lua)
    }

    /// 加载 Lua 插件代码
    pub async fn load_plugin(&self, id: &str, code: &str) -> anyhow::Result<()> {
        let memory_limit = (self.config.plugin_max_memory_mb as usize) * 1024 * 1024;
        let lua = Self::create_sandboxed_lua(memory_limit)?;
        let config = self.config.clone();

        // 注册宿主函数
        super::lua_host::register_host_functions(&lua, config)?;

        // 执行插件代码
        lua.load(code).exec()?;

        self.states.lock().await.insert(id.to_string(), lua);
        Ok(())
    }

    /// 卸载插件
    pub async fn unload_plugin(&self, id: &str) {
        self.states.lock().await.remove(id);
    }

    /// 调用 Filter Hook
    pub async fn call_filter<T: Serialize + DeserializeOwned + Send>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<Option<T>> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(None),
        };

        let max_instructions = self.timeout_instructions;
        let result = self.exec_with_timeout(lua, max_instructions, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(None),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

            let input_value = lua.to_value(input)?;
            let result_value = func.call::<mlua::Value>(input_value)?;
            let output: T = lua.from_value(result_value)?;
            Ok(Some(output))
        })?;

        result
    }

    /// 调用 Action Hook
    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(()),
        };

        let max_instructions = self.timeout_instructions;
        self.exec_with_timeout(lua, max_instructions, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(()),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(()),
            };

            let data_value = lua.to_value(data)?;
            func.call::<()>(data_value)?;
            Ok(())
        })
    }

    /// 调用 String Filter Hook
    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let states = self.states.lock().await;
        let lua = match states.get(plugin_id) {
            Some(l) => l,
            None => return Ok(None),
        };

        let max_instructions = self.timeout_instructions;
        self.exec_with_timeout(lua, max_instructions, || {
            let globals = lua.globals();
            let plugin_table: mlua::Table = match globals.get("Plugin") {
                Ok(t) => t,
                Err(_) => return Ok(None),
            };
            let func: mlua::Function = match plugin_table.get(func_name) {
                Ok(f) => f,
                Err(_) => return Ok(None),
            };

            let result: String = func.call(input)?;
            Ok(Some(result))
        })
    }

    /// 带超时执行 Lua 代码（指令计数 hook）
    fn exec_with_timeout<F, R>(&self, lua: &Lua, max_instructions: i64, f: F) -> anyhow::Result<R>
    where
        F: FnOnce() -> anyhow::Result<R>,
    {
        use std::sync::atomic::{AtomicI64, Ordering};

        let remaining = Arc::new(AtomicI64::new(max_instructions));
        let remaining_clone = remaining.clone();

        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1000),
            move |_lua, _debug| {
                if remaining_clone.fetch_sub(1000, Ordering::Relaxed) <= 1000 {
                    Err(LuaError::runtime("execution timeout"))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )?;

        let result = f();
        lua.remove_hook();
        result
    }

    /// 获取已加载 Lua 插件数量
    pub async fn plugin_count(&self) -> usize {
        self.states.lock().await.len()
    }
}
```

### 3.2 宿主函数

```rust
// lua_host.rs
use std::sync::Arc;

use mlua::{Function, Lua, Table};

use crate::config::app::AppConfig;

/// 注册宿主函数到 Lua 全局作用域
pub fn register_host_functions(lua: &Lua, config: Arc<AppConfig>) -> anyhow::Result<()> {
    let globals = lua.globals();
    let host = lua.create_table()?;

    // Host.log(level, message)
    let log_fn = lua.create_function(|_, (level, msg): (String, String)| {
        match level.as_str() {
            "warn" => tracing::warn!("[plugin:lua] {msg}"),
            "error" => tracing::error!("[plugin:lua] {msg}"),
            _ => tracing::info!("[plugin:lua] {msg}"),
        }
        Ok(())
    })?;
    host.set("log", log_fn)?;

    // Host.getConfig(key) -> string | nil
    let get_config_fn = lua.create_function(move |lua, key: String| {
        match get_config_value(&config, &key) {
            Some(val) => Ok(lua.create_string(&val)?.into()),
            None => Ok(mlua::Value::Nil),
        }
    })?;
    host.set("getConfig", get_config_fn)?;

    globals.set("Host", host)?;
    Ok(())
}

fn get_config_value(config: &AppConfig, key: &str) -> Option<String> {
    match key {
        "app.host" => Some(config.host.clone()),
        "app.port" => Some(config.port.to_string()),
        "app.env" => Some(config.env.clone()),
        "app.base_url" => Some(config.base_url.clone()),
        "jwt.access_expires" => Some(config.jwt_access_expires.to_string()),
        "jwt.refresh_expires" => Some(config.jwt_refresh_expires.to_string()),
        "upload.dir" => Some(config.upload_dir.clone()),
        "upload.max_size" => Some(config.max_upload_size.to_string()),
        "plugin.max_memory_mb" => Some(config.plugin_max_memory_mb.to_string()),
        "plugin.default_timeout_ms" => Some(config.plugin_default_timeout_ms.to_string()),
        _ => None,
    }
}
```

---

## 4. 数据传递：Lua vs JS

这是 Lua 方案的核心优势 — **原生 serde 映射，无 JSON 中转**。

### 4.1 JS 插件（当前实现）

```javascript
// 必须手动 JSON.parse/stringify
on_post_creating: function(inputJson) {
    var input = JSON.parse(inputJson);
    input.title = input.title.toUpperCase();
    return JSON.stringify(input);   // 必须返回字符串
}
```

### 4.2 Lua 插件（直接 table 操作）

```lua
-- 原生 table，无需序列化
function Plugin.on_post_creating(input)
    input.title = input.title:upper()
    return input
end
```

### 4.3 Rust 侧映射

```rust
// mlua 自动完成 Rust struct ↔ Lua table 转换
let input_value = lua.to_value(&post_data)?;   // Rust → Lua table
let result_value = func.call::<mlua::Value>(input_value)?;
let output: PostData = lua.from_value(result_value)?;  // Lua table → Rust

// 对比 rquickjs：需要手动 JSON 序列化
let input_json = serde_json::to_string(&post_data)?;
let result_str: String = func.call((input_json,))?;     // 字符串进出
let output: PostData = serde_json::from_str(&result_str)?;
```

### 4.4 类型映射表

| Rust 类型 | Lua 类型 | 说明 |
|-----------|----------|------|
| `String` | `string` | 直接映射 |
| `i32` / `u64` | `number` | Lua number（f64） |
| `bool` | `boolean` | 直接映射 |
| `Option<T>` | `T` 或 `nil` | None = nil |
| `Vec<T>` | `table` (array) | 索引从 1 开始（Lua 习惯） |
| `HashMap<K,V>` | `table` (dict) | 直接映射 |
| `serde_json::Value` | nested table | 自动递归映射 |
| `()` | 无返回值 | void |

---

## 5. 插件清单

### 5.1 plugin.toml 变更

```toml
# plugins/seo-optimizer-lua/plugin.toml
[plugin]
id = "com.raisfast.seo-optimizer-lua"
name = "SEO Optimizer (Lua)"
version = "1.0.0"
description = "Lua 版 SEO 优化插件"
author = "raisfast"
license = "MIT"
runtime = "lua"              # "wasm" | "js" | "lua"
language = "lua"             # 信息字段
entry = "init.lua"           # Lua 入口文件名（默认 init.lua）

[permissions]
max_memory_mb = 8
timeout_ms = 2000

[hooks.on_post_creating]
priority = 10

[hooks.filter_html]
priority = 5
```

### 5.2 manifest.rs 变更

```rust
fn default_entry() -> String {
    "index.js".into()
}
// runtime="lua" 时 entry 默认应为 "init.lua"
// 在 load_plugin_from_dir 中根据 runtime 选择默认值
```

### 5.3 插件目录结构

```
plugins/
├── seo-optimizer/           # WASM 插件
│   ├── plugin.toml
│   └── seo_optimizer.wasm
├── content-filter/          # WASM 插件
│   ├── plugin.toml
│   └── content_filter.wasm
├── welcome-email/           # JS 插件
│   ├── plugin.toml
│   └── index.js
└── excerpt-generator/       # Lua 插件
    ├── plugin.toml
    └── init.lua
```

---

## 6. Lua 插件开发规范

### 6.1 约定

Lua 插件必须设置一个全局 `Plugin` table，包含对应的 Hook 函数。

```lua
-- Plugin 全局 table
Plugin = {
    -- Filter Hook — 接收 table，返回修改后的 table
    on_post_creating = function(input)
        if not input.excerpt or input.excerpt == "" then
            input.excerpt = input.content:sub(1, 200) .. "..."
        end
        return input
    end,

    -- Action Hook — 接收 table，无返回值
    on_post_created = function(data)
        Host.log("info", "New post published: " .. data.title)
    end,

    -- String Filter Hook — 接收字符串，返回字符串
    filter_html = function(html)
        return html:gsub("<head>", '<head><meta property="og:type" content="article">')
    end,
}
```

### 6.2 Hook 方法签名

| Hook 类型 | 方法签名 | 说明 |
|-----------|---------|------|
| Table Filter | `function(table): table` | 接收/返回 Lua table（自动 serde 映射） |
| Table Action | `function(table): nil` | 接收 Lua table，无返回值 |
| String Filter | `function(string): string` | 接收/返回字符串 |
| Route Handler | `function(table): table` | 返回 `{ status = 200, body = "..." }` |

### 6.3 宿主 API

| 函数 | 签名 | 说明 |
|------|------|------|
| `Host.log(level, msg)` | `(string, string) -> nil` | 写入宿主日志 |
| `Host.getConfig(key)` | `(string) -> string|nil` | 读取宿主配置 |

### 6.4 Lua 环境限制

插件运行在受限 Lua 环境中，只加载以下标准库：

| 可用 | 不可用 |
|------|--------|
| `table`（table.insert 等） | `io`（文件读写） |
| `string`（string.match 等） | `os`（系统调用） |
| `math`（math.floor 等） | `debug`（调试器） |
| `utf8`（utf8.len 等） | `package`（模块加载） |
| `coroutine`（协程） | `dofile` / `loadfile` |

---

## 7. 示例插件

### 7.1 Excerpt Generator（Lua）

```toml
# plugins/excerpt-generator/plugin.toml
[plugin]
id = "com.raisfast.excerpt-generator"
name = "Excerpt Generator"
version = "1.0.0"
description = "自动从内容生成文章摘要"
runtime = "lua"
language = "lua"
entry = "init.lua"

[permissions]
max_memory_mb = 4
timeout_ms = 1000

[hooks.on_post_creating]
priority = 10
```

```lua
-- plugins/excerpt-generator/init.lua
Plugin = {
    on_post_creating = function(input)
        if not input.excerpt or input.excerpt == "" then
            local plain = input.content
                :gsub("```[\s\S]*?```", "")
                :gsub("[#*_`]", "")
                :gsub("%s+", " ")
                :match("^%s*(.-)%s*$")

            if #plain > 200 then
                input.excerpt = plain:sub(1, 200) .. "..."
            else
                input.excerpt = plain
            end
        end

        -- 读取宿主配置
        local env = Host.getConfig("app.env")
        if env == "development" then
            Host.log("info", "excerpt generated in dev mode")
        end

        return input
    end,
}
```

### 7.2 OG Tag Injector（Lua）

```toml
# plugins/og-tags/plugin.toml
[plugin]
id = "com.raisfast.og-tags"
name = "OG Tag Injector"
version = "1.0.0"
description = "注入 Open Graph 和 Twitter Card 标签"
runtime = "lua"

[hooks.filter_html]
priority = 5
```

```lua
-- plugins/og-tags/init.lua
Plugin = {
    filter_html = function(html)
        local base_url = Host.getConfig("app.base_url") or ""
        local og = string.format(
            '<meta property="og:type" content="article">'
            .. '<meta property="og:url" content="%s">',
            base_url
        )
        return html:gsub("<head>", "<head>" .. og)
    end,
}
```

### 7.3 Content Filter（Lua 版，对比 WASM 和 JS）

```toml
# plugins/word-filter-lua/plugin.toml
[plugin]
id = "com.raisfast.word-filter-lua"
name = "Word Filter (Lua)"
version = "1.0.0"
description = "过滤评论中的敏感词（Lua 版）"
runtime = "lua"

[hooks.on_comment_creating]
priority = 5
```

```lua
-- plugins/word-filter-lua/init.lua
local bad_words = { "badword1", "badword2", "spam" }

Plugin = {
    on_comment_creating = function(input)
        local content = input.content
        for _, word in ipairs(bad_words) do
            content = content:gsub(word, string.rep("*", #word))
        end
        input.content = content
        return input
    end,
}
```

### 7.4 三引擎同功能对比

同一功能（过滤敏感词）的三种实现：

**WASM (Rust → WASM):**

```rust
// 需要 plugins-sdk、长度前缀 ABI、alloc/dealloc、wasm32 编译
#[no_mangle]
pub unsafe extern "C" fn on_comment_creating(ptr: i32, len: i32) -> i32 {
    let input = read_input(ptr, len);
    let filtered = input.replace("badword", "********");
    write_output(&filtered)
}
```

**JS (QuickJS):**

```javascript
var badWords = ["badword1", "badword2"];
Plugin = {
    on_comment_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        badWords.forEach(function(w) {
            input.content = input.content.replace(new RegExp(w, "g"), "***");
        });
        return JSON.stringify(input);
    }
};
```

**Lua (mlua):**

```lua
local bad_words = { "badword1", "badword2" }
Plugin = {
    on_comment_creating = function(input)
        for _, w in ipairs(bad_words) do
            input.content = input.content:gsub(w, string.rep("*", #w))
        end
        return input
    end
}
```

| 维度 | WASM | JS | Lua |
|------|------|----|-----|
| 代码行数 | ~20（+ ABI 层） | ~10 | ~7 |
| 序列化 | 手动指针+长度 | JSON.parse/stringify | **无（原生 table）** |
| 工具链依赖 | Rust + wasm32 target | Node.js/esbuild（TS时） | **无** |
| 性能 | 最高 | 中等 | 轻量场景最快（无序列化开销） |

---

## 8. Feature Flag 与编译

### 8.1 Cargo.toml

```toml
[features]
default = ["db-sqlite", "plugin-all"]

# Database backend
db-sqlite  = ["sqlx/sqlite"]
db-postgres = ["sqlx/postgres"]
db-mysql   = ["sqlx/mysql"]

# Plugin runtimes
plugin-wasm = ["wasmtime", "wasmtime-wasi"]
plugin-js   = ["rquickjs"]
plugin-lua  = ["mlua"]
plugin-all  = ["plugin-wasm", "plugin-js", "plugin-lua"]

[dependencies]
# ... 现有依赖 ...

# Plugin: Lua (可选)
mlua = { version = "0.11", features = ["lua54", "vendored", "send", "serde", "async", "macros"], optional = true }
```

**Feature flag 说明：**

| Feature | 作用 |
|---------|------|
| `lua54` | 使用 Lua 5.4（推荐，最新稳定） |
| `vendored` | 从源码编译 Lua，无需系统安装 |
| `send` | 使 `Lua`/`Table`/`Function` 等为 `Send+Sync` |
| `serde` | `LuaSerdeExt` — Rust struct ↔ Lua table 自动映射 |
| `async` | `create_async_function` + `call_async` 支持 |
| `macros` | `#[mlua::lua_module]` 等便捷宏 |

### 8.2 编译示例

```bash
# 仅 WASM 插件
cargo build --features "db-sqlite,plugin-wasm"

# 仅 JS 插件
cargo build --features "db-sqlite,plugin-js"

# 仅 Lua 插件
cargo build --features "db-sqlite,plugin-lua"

# 三引擎（推荐）
cargo build --features "db-sqlite,plugin-all"

# 最小部署（仅 Lua，体积最小）
cargo build --release --features "db-sqlite,plugin-lua"
```

### 8.3 条件编译

```rust
// src/plugins/mod.rs
#[cfg(feature = "plugin-wasm")]
mod engine;
#[cfg(feature = "plugin-wasm")]
mod host;
#[cfg(feature = "plugin-js")]
mod engine_js;
#[cfg(feature = "plugin-js")]
mod js_host;
#[cfg(feature = "plugin-lua")]
mod engine_lua;
#[cfg(feature = "plugin-lua")]
mod lua_host;
```

---

## 9. 安全机制

### 9.1 stdlib 裁剪

```rust
// 只加载安全的库，排除 io/os/debug/package
let lua = Lua::new_with(
    StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
    LuaOptions::default(),
)?;
```

排除能力：

| 被排除的库 | 危险操作 |
|------------|----------|
| `io` | 文件读写、流操作 |
| `os` | 系统命令、环境变量、文件重命名/删除 |
| `debug` | 访问内部状态、设置 metamethod |
| `package` | 加载 C 模块、执行任意代码 |

### 9.2 内存限制

```rust
lua.set_memory_limit(8 * 1024 * 1024)?;  // 8MB

// 查询当前使用量
let used: usize = lua.used_memory();
```

自定义分配器跟踪每一次 alloc/dealloc，超限时返回 `Error::MemoryError`。

### 9.3 执行超时（指令计数 hook）

```rust
lua.set_hook(
    HookTriggers::new().every_nth_instruction(1000),
    move |_lua, _debug| {
        if remaining.fetch_sub(1000, Ordering::Relaxed) <= 1000 {
            Err(LuaError::runtime("execution timeout"))
        } else {
            Ok(VmState::Continue)
        }
    },
)?;
```

- 每 1000 条 VM 指令检查一次
- 超时抛出不可捕获的 `Error::RuntimeError`
- 开销 < 1%（每 1000 条指令一次原子操作）

### 9.4 三引擎安全对比

| 安全维度 | WASM | JS | Lua |
|----------|------|----|----|
| 内存隔离 | 独立线性内存 | GC 管理（同进程） | GC 管理（同进程） |
| 计算限制 | fuel（精确） | interrupt handler（时间） | **指令计数 hook** |
| 文件系统 | 无 | 无 | **stdlib 裁剪** |
| 网络访问 | 无 | 无 | **无 io 库** |
| 代码加载 | 无 | 无 | **无 package 库** |
| 逃逸风险 | 极低 | 低 | **低（stdlib 裁剪 + hook）** |
| 逃逸缓解 | fuel 耗尽 | memory limit | **set_hook + memory limit** |

---

## 10. justfile 新增命令

```justfile
# 复制 Lua 插件到 plugins/ 目录
plugins-lua-build:
    @echo "Copying Lua plugins..."
    @mkdir -p plugins/excerpt-generator
    @cp plugins-examples-lua/excerpt-generator/plugin.toml plugins/excerpt-generator/
    @cp plugins-examples-lua/excerpt-generator/init.lua plugins/excerpt-generator/
    @echo "Done. Lua plugins ready in plugins/"

# 编译/复制所有插件（WASM + JS + Lua）
plugins-all: plugins-build plugins-js-build plugins-lua-build
```

---

## 11. 实施计划

### Phase 1：最小集成（1 天）

- [ ] `Cargo.toml` 添加 `mlua` 可选依赖 + `plugin-lua` feature flag
- [ ] 创建 `engine_lua.rs`（LuaEngine 基本结构）
- [ ] 创建 `lua_host.rs`（Host.log + Host.getConfig）
- [ ] 修改 `manifest.rs`（entry 默认值根据 runtime 区分）
- [ ] 修改 `mod.rs`（三引擎分派 + `LoadedPluginInstance::Lua`）
- [ ] 创建示例 Lua 插件
- [ ] 条件编译验证

### Phase 2：功能完善（1 天）

- [ ] `dispatch_filter` / `dispatch_action` / `dispatch_render_override` Lua 分派
- [ ] 超时机制（指令计数 hook）
- [ ] 热重载支持（复用 channel 架构，监听 `.lua` 文件）
- [ ] justfile 命令

### Phase 3：测试（1 天）

- [ ] LuaEngine 单元测试（创建、加载、调用、超时、内存限制）
- [ ] 集成测试（Lua 插件 Hook 调度）
- [ ] 三引擎共存测试（WASM + JS + Lua 混合 filter chain）
- [ ] 安全机制测试（内存超限、执行超时、stdlib 限制）

---

## 12. 风险与缓解

| 风险 | 严重性 | 缓解措施 |
|------|--------|---------|
| Lua 开发者远少于 JS | 中 | 面向高级用户/内部工具，JS 面向社区 |
| Lua number 为 f64，整数大时精度丢失 | 低 | ID 用 string 传递（UUID v7 无此问题） |
| Lua table 索引从 1 开始 | 低 | mlua serde 自动处理 Vec→1-indexed |
| `send` feature 增加 Arc/Mutex 开销 | 低 | 开销极小，ReentrantMutex 为 parkin_lot 实现 |
| Lua 5.4 vs LuaJIT vs Luau 选择 | 低 | 固定 `lua54` + `vendored`，避免碎片化 |
| 指令计数 hook 有性能开销 | 低 | 每 1000 条指令检查一次，<1% 开销 |
| `#![deny(unsafe_code)]` 冲突 | 低 | mlua 内部封装 unsafe，不侵入主 crate |

---

## 13. 总结

Lua 作为第三插件运行时的定位：

| 运行时 | 定位 | 适用场景 |
|--------|------|----------|
| **WASM** | 安全最高 | 高安全要求、性能敏感、Rust 开发者 |
| **JS** | 生态最大 | 社区贡献、前端开发者、快速原型 |
| **Lua** | **最轻量** | 嵌入式部署、配置型插件、运维脚本、最小依赖 |

三引擎共存，按场景选择，共享同一套 Hook 调度和安全管理机制。
