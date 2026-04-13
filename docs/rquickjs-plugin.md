# rquickjs 动态脚本插件方案

> 在现有 WASM 插件系统基础上，新增 QuickJS (rquickjs) 作为第二运行时，
> 支持 JavaScript/TypeScript 编写插件，降低社区贡献门槛。

---

## 1. 背景

### 1.1 现状

当前插件系统基于 wasmtime，插件用 Rust 编写后编译为 WASM。

**痛点：**

- 插件开发者必须安装 Rust 工具链 + wasm32-unknown-unknown target
- ABI 协议复杂（手动内存管理、长度前缀协议）
- 调试困难（WASM 黑盒）
- 编译-部署流程繁琐（cargo build → copy .wasm → 重启）

### 1.2 目标

- 支持用 JS/TS 编写插件，零 Rust 工具链依赖
- 与现有 WASM 插件共享 Hook 调度机制
- 插件加载/卸载/热重载与 WASM 一致
- 安全隔离（内存限制、执行超时）
- 编译体积影响最小

### 1.3 选型对比

| 维度 | wasmtime (现有) | deno_core (V8) | rquickjs (本方案) |
|------|----------------|----------------|-------------------|
| 引擎体积 (release) | ~15MB | ~12MB + 40MB V8 | **~2MB** |
| 编译时间增量 | 基准 | +4 min | **+30 sec** |
| 依赖 crate 数 | ~280 | ~30 | **~5** |
| 冷启动 | ~5-50ms | ~10-100ms | **~1-5ms** |
| 内存/实例 | ~1-2MB | ~2-5MB | **~200-500KB** |
| Send/Sync | 是 | 否（需独立线程） | 否（AsyncRuntime 内置） |
| API 稳定性 | 稳定 (v26) | 不稳定 (0.398) | **较稳定 (0.11)** |
| 文档覆盖率 | 优秀 | 31% | **100%** |
| JS 性能 | N/A | JIT（最快） | 解释执行（博客场景够用） |

**选择 rquickjs 的核心理由：**

1. 集成成本最低 — `AsyncRuntime` 直接在 tokio 中使用，无需独立线程
2. 体积最小 — QuickJS 编译为 ~1MB 静态库，CI 几乎无影响
3. 博客插件不需要 JIT — 每个 Hook 执行几行 JS，解释执行完全够用
4. API 稳定、文档完整 — 0.11 版本，100% 文档覆盖

---

## 2. 架构设计

### 2.1 模块结构

```
src/plugins/
├── mod.rs            # PluginManager（统一入口，按 runtime 字段分派）
├── manifest.rs       # 复用现有 manifest（runtime 字段区分引擎）
├── engine.rs         # WASM 引擎（现有，不改）
├── host.rs           # WASM host functions（现有，不改）
├── engine_js.rs      # QuickJS 引擎（新增）
└── js_host.rs        # JS 宿主函数（新增）
```

### 2.2 类型体系

```rust
// mod.rs — 插件实例枚举
enum LoadedPluginInstance {
    Wasm(RwLock<WasmInstance>),
    Js(JsPluginRef),
}

struct LoadedPlugin {
    manifest: PluginManifest,
    instance: LoadedPluginInstance,
}

struct JsPluginRef {
    plugin_id: String,
    // 通过 Arc<JsEngine> 共享引擎，异步调用
}
```

### 2.3 PluginManager 分派

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
                    let mut inst = wasm.write().await;
                    current = wasm_call_filter(&mut inst, func_name, current)?;
                }
                LoadedPluginInstance::Js(js) => {
                    current = self.js_engine
                        .call_filter(&js.plugin_id, func_name, &current)
                        .await?;
                }
            }
        }
        Ok(current)
    }
}
```

### 2.4 线程模型

```
┌─────────────────────────────────────────┐
│            tokio runtime                │
│                                         │
│  PluginManager (Arc)                    │
│  ├── wasm_engine: wasmtime::Engine      │
│  ├── js_engine: Arc<JsEngine>           │
│  │   ├── runtime: AsyncRuntime (Mutex)  │
│  │   └── contexts: HashMap<AsyncContext> │
│  └── plugins: RwLock<HashMap>           │
│                                         │
│  axum handler                           │
│  └── dispatch_filter()                  │
│      ├── .await (tokio task)            │
│      └── ctx.with(|ctx| { ... }).await  │
│                                         │
└─────────────────────────────────────────┘
```

- `AsyncRuntime` 和 `AsyncContext` 使用 tokio async Mutex
- 每个插件一个 `AsyncContext`（独立全局作用域）
- 所有 JS 调用通过 `ctx.with(|ctx| { ... }).await` 执行
- 不需要独立线程（对比 deno_core）

---

## 3. 引擎实现

### 3.1 JsEngine

```rust
// engine_js.rs
use std::collections::HashMap;
use std::sync::Arc;

use rquickjs::{AsyncRuntime, AsyncContext, CatchResultExt};
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::Mutex;

pub struct JsEngine {
    runtime: AsyncRuntime,
    contexts: Mutex<HashMap<String, AsyncContext>>,
    memory_limit: usize,
    timeout_ms: u64,
}

impl JsEngine {
    pub fn new(memory_limit_mb: u32, timeout_ms: u64) -> anyhow::Result<Self> {
        let runtime = AsyncRuntime::new()?;
        runtime.set_memory_limit((memory_limit_mb as usize) * 1024 * 1024);
        runtime.set_max_stack_size(512 * 1024);
        // TODO: set_interrupt_handler for timeout

        Ok(Self {
            runtime,
            contexts: Mutex::new(HashMap::new()),
            memory_limit: (memory_limit_mb as usize) * 1024 * 1024,
            timeout_ms,
        })
    }

    /// 加载 JS 插件代码
    pub async fn load_plugin(&self, id: &str, code: &str) -> anyhow::Result<()> {
        let ctx = AsyncContext::full(&self.runtime)?;
        ctx.with(|ctx| {
            ctx.eval(code)?;
            Ok::<_, rquickjs::Error>(())
        }).await?;

        self.contexts.lock().await.insert(id.to_string(), ctx);
        Ok(())
    }

    /// 卸载插件
    pub async fn unload_plugin(&self, id: &str) {
        self.contexts.lock().await.remove(id);
    }

    /// 调用 Filter Hook（返回修改后的数据）
    pub async fn call_filter<T: Serialize + DeserializeOwned>(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &T,
    ) -> anyhow::Result<T> {
        let contexts = self.contexts.lock().await;
        let ctx = contexts.get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("js plugin not found: {plugin_id}"))?;

        let input_json = serde_json::to_value(input)?;
        let result = ctx.with(|ctx| {
            let global = ctx.globals();
            let plugin_obj: rquickjs::Object = global.get("Plugin")?;
            let func: rquickjs::Function = plugin_obj.get(func_name)?;

            let input_val = rquickjs::String::from_json(&ctx, &input_json.to_string())?;
            let result_val = func.call1(input_val)?;

            let result_str: String = result_val.get()?;
            let output: T = serde_json::from_str(&result_str)?;
            Ok(output)
        }).await?;

        Ok(result)
    }

    /// 调用 Action Hook（无返回值）
    pub async fn call_action<T: Serialize>(
        &self,
        plugin_id: &str,
        func_name: &str,
        data: &T,
    ) -> anyhow::Result<()> {
        let contexts = self.contexts.lock().await;
        let ctx = contexts.get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("js plugin not found: {plugin_id}"))?;

        let data_json = serde_json::to_value(data)?;
        ctx.with(|ctx| {
            let global = ctx.globals();
            let plugin_obj: rquickjs::Object = global.get("Plugin")?;
            let func: rquickjs::Function = plugin_obj.get(func_name)?;
            let val = rquickjs::String::from_json(&ctx, &data_json.to_string())?;
            func.call1(val)?;
            Ok::<_, rquickjs::Error>(())
        }).await?;

        Ok(())
    }

    /// 调用 String Filter Hook（如 render_markdown、filter_html）
    pub async fn call_string_filter(
        &self,
        plugin_id: &str,
        func_name: &str,
        input: &str,
    ) -> anyhow::Result<Option<String>> {
        let contexts = self.contexts.lock().await;
        let ctx = contexts.get(plugin_id)
            .ok_or_else(|| anyhow::anyhow!("js plugin not found: {plugin_id}"))?;

        let result = ctx.with(|ctx| {
            let global = ctx.globals();
            let plugin_obj: rquickjs::Object = global.get("Plugin")?;
            let func: rquickjs::Function = plugin_obj.get(func_name)?;
            let result_val: rquickjs::String = func.call1((input,))?;
            let output: String = result_val.get()?;
            Ok(output)
        }).await?;

        Ok(Some(result))
    }
}
```

### 3.2 宿主函数

```rust
// js_host.rs
/// 注册到 JS 全局对象的宿主函数
///
/// 插件可直接调用：
///   Host.log("info", "hello from plugin");
///   var cfg = Host.getConfig("seo.keywords");
fn register_host_functions(ctx: &rquickjs::Ctx) -> anyhow::Result<()> {
    let global = ctx.globals();
    let host = rquickjs::Object::new(ctx)?;

    host.set("log", rquickjs::Function::new(ctx, |ctx: rquickjs::Ctx, level: String, msg: String| {
        match level.as_str() {
            "warn" => tracing::warn!("[plugin] {msg}"),
            "error" => tracing::error!("[plugin] {msg}"),
            _ => tracing::info!("[plugin] {msg}"),
        }
        Ok::<(), rquickjs::Error>(())
    })?)?;

    host.set("getConfig", rquickjs::Function::new(ctx, |ctx: rquickjs::Ctx, key: String| {
        // 从 OpState 读取配置
        let value = None::<String>; // TODO: 接入 AppConfig
        Ok(value)
    })?)?;

    global.set("Host", host)?;
    Ok(())
}
```

---

## 4. 插件清单

### 4.1 plugin.toml 变更

```toml
# plugins/seo-optimizer/plugin.toml
[plugin]
id = "com.rust-blog.seo-optimizer"
name = "SEO Optimizer"
version = "1.0.0"
description = "自动优化文章 SEO"
author = "rust-blog"
license = "MIT"
runtime = "js"          # "wasm" (默认) 或 "js"
language = "typescript"  # 信息字段
entry = "index.js"       # JS 入口文件名（默认 index.js）
```

### 4.2 manifest.rs 变更

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    // ... 现有字段 ...

    #[serde(default = "default_runtime")]
    pub runtime: String,      // "wasm" | "js"
    #[serde(default)]
    pub language: String,
    #[serde(default = "default_wasm")]
    pub wasm: String,         // WASM 文件名
    #[serde(default = "default_entry")]
    pub entry: String,        // JS 入口文件名
}

fn default_entry() -> String {
    "index.js".into()
}
```

### 4.3 插件目录结构

```
plugins/
├── seo-optimizer/          # WASM 插件（现有）
│   ├── plugin.toml
│   └── seo_optimizer.wasm
├── content-filter/         # WASM 插件（现有）
│   ├── plugin.toml
│   └── content_filter.wasm
└── welcome-email/          # JS 插件（新增）
    ├── plugin.toml
    └── index.js
```

---

## 5. JS 插件开发规范

### 5.1 约定

JS 插件必须导出一个全局 `Plugin` 对象，包含对应的 Hook 方法。

```javascript
// 插件全局对象
var Plugin = {
    // Hook 方法名与 HookPoint.wasm_func_name() 一致

    // Filter Hook — 接收 JSON 字符串，返回修改后的 JSON 字符串
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        if (!input.excerpt) {
            input.excerpt = input.content.substring(0, 200) + "...";
        }
        return JSON.stringify(input);
    },

    // Action Hook — 接收 JSON 字符串，无返回值
    on_post_created: function(dataJson) {
        var data = JSON.parse(dataJson);
        Host.log("info", "New post published: " + data.title);
    },

    // String Filter Hook — 接收字符串，返回修改后的字符串
    filter_html: function(html) {
        return html.replace(
            "<head>",
            '<head><meta property="og:type" content="article">'
        );
    }
};
```

### 5.2 Hook 方法签名

| Hook 类型 | 方法签名 | 说明 |
|-----------|---------|------|
| JSON Filter | `function(inputJson: string): string` | 接收/返回 JSON 字符串 |
| JSON Action | `function(dataJson: string): void` | 接收 JSON 字符串，无返回值 |
| String Filter | `function(input: string): string` | 接收/返回原始字符串 |

### 5.3 宿主 API

插件可通过全局 `Host` 对象调用宿主函数：

| 函数 | 签名 | 说明 |
|------|------|------|
| `Host.log(level, msg)` | `(string, string) -> void` | 写入宿主日志 |
| `Host.getConfig(key)` | `(string) -> string|null` | 读取宿主配置 |

### 5.4 TypeScript 支持

QuickJS 不内置 TypeScript 编译。推荐使用 esbuild 预编译：

```bash
# 开发时编译
esbuild plugins/welcome-email/src/index.ts --outfile=plugins/welcome-email/index.js --bundle --format=iife --target=es2021

# 或在 justfile 中
just plugins-js-build
```

TypeScript 类型定义（随项目提供）：

```typescript
// plugins-sdk-js/index.d.ts
interface Host {
    log(level: "info" | "warn" | "error", message: string): void;
    getConfig(key: string): string | null;
}

declare var Host: Host;

interface PluginHooks {
    on_post_creating?(inputJson: string): string;
    on_post_created?(dataJson: string): void;
    on_post_updating?(inputJson: string): string;
    on_post_updated?(dataJson: string): void;
    on_post_deleted?(dataJson: string): void;
    on_comment_creating?(inputJson: string): string;
    on_comment_created?(dataJson: string): void;
    render_markdown?(content: string): string;
    filter_html?(html: string): string;
    handle_route?(routeJson: string): string;
    on_login?(dataJson: string): void;
}

declare var Plugin: PluginHooks;
```

---

## 6. 示例插件

### 6.1 Welcome Email（JS）

```toml
# plugins/welcome-email/plugin.toml
[plugin]
id = "com.rust-blog.welcome-email"
name = "Welcome Email"
version = "1.0.0"
description = "用户注册后发送欢迎邮件"
runtime = "js"
language = "javascript"

[hooks.on-login]
priority = 20
```

```javascript
// plugins/welcome-email/index.js
var Plugin = {
    on_login: function(dataJson) {
        var data = JSON.parse(dataJson);
        if (data.success) {
            Host.log("info", "User logged in: " + data.email);
        }
    }
};
```

### 6.2 SEO Optimizer（JS 版本，替代 WASM 版本）

```toml
# plugins/seo-optimizer-js/plugin.toml
[plugin]
id = "com.rust-blog.seo-optimizer-js"
name = "SEO Optimizer (JS)"
version = "1.0.0"
description = "自动优化文章 SEO"
runtime = "js"
language = "typescript"
entry = "index.js"

[permissions]
max_memory_mb = 8
timeout_ms = 2000

[hooks.on-post-creating]
priority = 10

[hooks.filter-html]
priority = 5
```

```typescript
// plugins/seo-optimizer-js/src/index.ts → 编译为 index.js

var Plugin = {
    on_post_creating: function(inputJson: string): string {
        var input = JSON.parse(inputJson);
        if (!input.excerpt || input.excerpt === "") {
            var plain = input.content
                .replace(/```[\s\S]*?```/g, "")
                .replace(/[#*_`]/g, "")
                .replace(/\s+/g, " ")
                .trim();
            input.excerpt = plain.substring(0, 200);
            if (plain.length > 200) input.excerpt += "...";
        }
        return JSON.stringify(input);
    },

    filter_html: function(html: string): string {
        var meta = '<meta property="og:type" content="article">';
        return html.replace("<head>", "<head>" + meta);
    }
};
```

---

## 7. Feature Flag 与编译

### 7.1 Cargo.toml

```toml
[features]
default = ["db-sqlite"]

# Database backend
db-sqlite  = ["sqlx/sqlite"]
db-postgres = ["sqlx/postgres"]
db-mysql   = ["sqlx/mysql"]

# Plugin runtimes
plugin-wasm = ["wasmtime"]
plugin-js   = ["rquickjs"]
plugin-all  = ["plugin-wasm", "plugin-js"]

[dependencies]
# ... 现有依赖 ...

# Plugin: WASM (可选)
wasmtime = { version = "26", optional = true }
wasmtime-wasi = { version = "26", optional = true }

# Plugin: QuickJS (可选)
rquickjs = { version = "0.11", features = ["futures", "loader", "macro"], optional = true }
```

### 7.2 编译示例

```bash
# 仅 WASM 插件（现有行为）
cargo build --features "db-sqlite,plugin-wasm"

# 仅 JS 插件
cargo build --features "db-sqlite,plugin-js"

# 双引擎（推荐）
cargo build --features "db-sqlite,plugin-all"
```

### 7.3 条件编译

```rust
// src/plugins/mod.rs
#[cfg(feature = "plugin-wasm")]
mod engine;

#[cfg(feature = "plugin-js")]
mod engine_js;

#[cfg(feature = "plugin-js")]
mod js_host;

// PluginManager 根据 feature 编译不同的分派逻辑
```

---

## 8. 安全机制

### 8.1 内存限制

```rust
let runtime = AsyncRuntime::new()?;
runtime.set_memory_limit(32 * 1024 * 1024); // 32MB 默认
```

可在 `plugin.toml` 的 `[permissions]` 中配置 `max_memory_mb`。

### 8.2 执行超时

```rust
runtime.set_interrupt_handler(Some(Box::new(move || {
    // 检查是否超过 timeout_ms
    start_time.elapsed().as_millis() > timeout_ms as u128
})));
```

### 8.3 沙箱隔离

- 每个 JS 插件运行在独立 `AsyncContext`（独立全局作用域）
- 无文件系统访问（不注册 `std` 模块）
- 无网络访问（不注册 `fetch` 等）
- 只能通过 `Host` 全局对象与宿主交互

### 8.4 与 WASM 安全对比

| 安全维度 | WASM (wasmtime) | QuickJS (rquickjs) |
|----------|-----------------|-------------------|
| 内存隔离 | 独立线性内存 | 同进程内存（QuickJS GC 管理） |
| 计算限制 | fuel（精确指令计数） | interrupt handler（周期检查） |
| 文件系统 | 默认无 | 默认无 |
| 网络访问 | 默认无 | 默认无 |
| 逃逸风险 | 极低 | 低（依赖 QuickJS 安全更新） |

对于博客系统，QuickJS 的安全级别足够。

---

## 9. justfile 新增命令

```justfile
# ── 插件 ──────────────────────────────────────────────────────────

# 编译所有 WASM 插件
plugins-build:
    @echo "Building seo-optimizer..."
    cd plugins-examples/seo-optimizer && cargo build --target wasm32-unknown-unknown --release
    @echo "Building content-filter..."
    cd plugins-examples/content-filter && cargo build --target wasm32-unknown-unknown --release
    @mkdir -p plugins/seo-optimizer plugins/content-filter
    cp plugins-examples/seo-optimizer/target/wasm32-unknown-unknown/release/seo_optimizer.wasm plugins/seo-optimizer/
    cp plugins-examples/seo-optimizer/plugin.toml plugins/seo-optimizer/
    cp plugins-examples/content-filter/target/wasm32-unknown-unknown/release/content_filter.wasm plugins/content-filter/
    cp plugins-examples/content-filter/plugin.toml plugins/content-filter/
    @echo "Done. WASM plugins ready in plugins/"

# 编译所有 JS 插件（TypeScript → JavaScript）
plugins-js-build:
    @echo "Building JS plugins..."
    @for dir in plugins-examples-js/*/; do \
        if [ -f "$$dir/src/index.ts" ]; then \
            name=$$(basename $$dir); \
            echo "  Compiling $$name..."; \
            npx esbuild "$$dir/src/index.ts" --outfile="plugins/$$name/index.js" --bundle --format=iife --target=es2021; \
            cp "$$dir/plugin.toml" "plugins/$$name/"; \
        fi \
    done
    @echo "Done. JS plugins ready in plugins/"

# 编译所有插件（WASM + JS）
plugins-all: plugins-build plugins-js-build
```

---

## 10. 实施计划

### Phase 1：最小集成（1-2 天）

- [ ] `Cargo.toml` 添加 `rquickjs` 可选依赖
- [ ] 创建 `engine_js.rs`（JsEngine 基本结构）
- [ ] 修改 `manifest.rs`（新增 `entry` 字段）
- [ ] 修改 `mod.rs`（按 `runtime` 字段分派）
- [ ] 创建 `js_host.rs`（Host.log）
- [ ] 编写欢迎邮件示例插件
- [ ] 条件编译验证（`plugin-js` / `plugin-all`）

### Phase 2：功能完善（1-2 天）

- [ ] `dispatch_filter` / `dispatch_action` / `dispatch_render_override` JS 分派
- [ ] 安全机制（内存限制、超时）
- [ ] `js_host.rs` 完善（Host.getConfig）
- [ ] 热重载支持（复用现有文件监听）
- [ ] justfile 命令

### Phase 3：开发者体验（1-2 天）

- [ ] TypeScript 类型定义文件
- [ ] esbuild 编译配置
- [ ] JS 版 SEO Optimizer 示例插件
- [ ] 插件开发文档

### Phase 4：测试（1 天）

- [ ] JsEngine 单元测试
- [ ] 集成测试（JS 插件 Hook 调用）
- [ ] 双引擎共存测试
- [ ] 安全机制测试（内存限制、超时）

---

## 11. 风险与缓解

| 风险 | 严重性 | 缓解措施 |
|------|--------|---------|
| QuickJS 不支持部分 ES 特性 | 低 | esbuild target=es2021 polyfill |
| `#![deny(unsafe_code)]` 冲突 | 中 | QuickJS 在 rquickjs crate 内部封装，不侵入主 crate |
| rquickjs 版本升级 | 低 | 锁定版本，feature flag 可禁用 |
| JS 插件安全性弱于 WASM | 低 | 博客场景可接受，内存限制 + 超时 |
| JSON 序列化开销 | 低 | 博客 Hook 数据量小（< 100KB） |
