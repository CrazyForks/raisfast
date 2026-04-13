# WebAssembly 插件系统设计文档

## 1. 概述

### 1.1 目标

为博客系统引入基于 **WebAssembly (WASM)** 的插件机制，使第三方开发者无需修改/重新编译宿主程序即可扩展博客功能。

### 1.2 设计原则

| 原则 | 说明 |
|------|------|
| **沙箱隔离** | 插件运行在 WASM 沙箱中，无法直接访问宿主文件系统/网络/数据库 |
| **热加载** | 运行时扫描插件目录，新增/更新 `.wasm` 文件后自动加载，无需重启 |
| **零侵入** | 宿主代码通过 Hook 点调用插件，插件不存在时走默认逻辑 |
| **类型安全** | 宿主-插件之间通过 WIT (WebAssembly Interface Types) 定义强类型接口 |
| **多语言支持** | 插件可用 Rust / AssemblyScript / Go / C 等任何可编译为 WASI 的语言编写 |

### 1.3 技术选型

| 组件 | 选择 | 理由 |
|------|------|------|
| WASM 运行时 | **wasmtime** (Cranelift) | Bytecode Alliance 官方、快启动、成熟安全模型、WASI preview1/2 |
| 接口定义 | **wit-bindgen** + WIT 文件 | wasm-component-model 标准方案，自动生成 Rust/AS/Go 绑定 |
| 组件模型 | wasm-component-model | 类型安全的跨语言调用，替代原始 export/import 函数 |
| 插件语言 | Rust (主推) + AssemblyScript | Rust 零成本 ABI、AS 作为轻量替代 |

---

## 2. 架构总览

```
┌─────────────────────────────────────────────────────────────────┐
│                        宿主 (rust-blog)                          │
│                                                                   │
│  ┌───────────┐  ┌───────────┐  ┌───────────┐  ┌───────────┐   │
│  │ handlers/ │  │ services/ │  │ models/   │  │ middleware/│   │
│  └─────┬─────┘  └─────┬─────┘  └───────────┘  └───────────┘   │
│        │              │                                            │
│        ▼              ▼                                            │
│  ┌─────────────────────────────────────┐                         │
│  │         PluginManager               │                         │
│  │  ┌──────────────────────────────┐   │                         │
│  │  │  Hook Dispatcher             │   │                         │
│  │  │  on_post_create / on_comment │   │                         │
│  │  │  render_markdown / filter    │   │                         │
│  │  └──────────┬───────────────────┘   │                         │
│  │             │                        │                         │
│  │  ┌──────────▼───────────────────┐   │                         │
│  │  │  Engine (wasmtime::Engine)   │   │                         │
│  │  │  Store  (per-plugin)         │   │                         │
│  │  │  Linker (host functions)     │   │                         │
│  │  └──────────────────────────────┘   │                         │
│  └─────────────────────────────────────┘                         │
│                                                                   │
│  ┌─────────────────────────────────────┐                         │
│  │  Host Functions (暴露给插件的 API)  │                         │
│  │  host_log() / host_get_config()    │                         │
│  │  host_http_get() / host_db_query() │                         │
│  └─────────────────────────────────────┘                         │
└─────────────────────────────────────────────────────────────────┘
          │
          │ WASM 调用 (组件模型 / wit-bindgen)
          ▼
┌─────────────────────────────────────────────────────────────────┐
│                    插件实例 (WASM 沙箱)                           │
│                                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │ plugin-a.wasm│  │ plugin-b.wasm│  │ plugin-c.wasm│          │
│  │ (Rust 编译)  │  │ (AS 编译)    │  │ (Go 编译)    │          │
│  └──────────────┘  └──────────────┘  └──────────────┘          │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. 插件接口定义 (WIT)

### 3.1 WIT 文件

放在 `wit/plugin.wit`，作为宿主和插件之间的契约：

```wit
package rust-blog:plugin;

/// 插件元数据，由插件导出。
interface metadata {
    /// 插件唯一标识（反向域名格式，如 "com.example.my-plugin"）
    resource plugin-id {
        /// 插件名称（人类可读）
        name: func() -> string;
        /// 插件版本（semver）
        version: func() -> string;
        /// 插件描述
        description: func() -> string;
    }
}

/// 宿主暴露给插件的能力。
interface host-api {
    /// 写日志到宿主的 tracing 系统。
    /// level: "trace" | "debug" | "info" | "warn" | "error"
    log: func(level: string, message: string);

    /// 获取宿主配置项。
    get-config: func(key: string) -> option<string>;

    /// 发起 HTTP GET 请求（受白名单限制）。
    http-get: func(url: string) -> result<string, string>;

    /// 获取文章内容（只读）。
    get-post: func(slug: string) -> option<string>;

    /// 获取当前请求的上下文信息。
    get-request-context: func() -> option<request-context>;
}

/// 请求上下文，传递给插件的当前 HTTP 请求信息。
resource request-context {
    method: func() -> string;
    path: func() -> string;
    client-ip: func() -> string;
    user-id: func() -> option<string>;
    user-role: func() -> option<string>;
}

/// Hook 接口 — 插件可选实现的部分。
world blog-plugin {
    import host-api;

    export metadata;

    /// 文章生命周期钩子
    export on-post-creating: func(post: post-input) -> post-input;
    export on-post-created: func(post: post-output) -> void;
    export on-post-updating: func(old: post-output, new: post-input) -> post-input;
    export on-post-deleted: func(post-id: string) -> void;

    /// 评论生命周期钩子
    export on-comment-creating: func(comment: comment-input) -> result<comment-input, string>;
    export on-comment-created: func(comment-id: string) -> void;

    /// 内容渲染钩子
    export render-markdown: func(content: string) -> option<string>;
    export filter-html: func(html: string) -> string;

    /// 自定义路由钩子
    export handle-route: func(path: string, method: string, body: option<string>) -> option<route-response>;

    /// 认证钩子
    export on-login: func(email: string, success: bool) -> void;
}
```

### 3.2 数据类型

```wit
record post-input {
    title: string,
    content: string,
    slug: option<string>,
    excerpt: option<string>,
    category-id: option<string>,
    tag-ids: list<string>,
    status: string,
}

record post-output {
    id: string,
    title: string,
    slug: string,
    content: string,
    excerpt: option<string>,
    status: string,
    author-id: string,
    category-id: option<string>,
    view-count: u64,
    created-at: string,
    updated-at: string,
    published-at: option<string>,
}

record comment-input {
    content: string,
    nickname: option<string>,
    email: option<string>,
    parent-id: option<string>,
}

record route-response {
    status: u16,
    headers: list<tuple<string, string>>,
    body: string,
}
```

---

## 4. Hook 系统

### 4.1 Hook 分类

| 类型 | Hook 名称 | 触发时机 | 返回值影响 |
|------|-----------|---------|-----------|
| **过滤器 (Filter)** | `on-post-creating` | 创建文章前 | 可修改文章字段 |
| **过滤器** | `on-post-updating` | 更新文章前 | 可修改更新字段 |
| **过滤器** | `render-markdown` | Markdown 渲染 | 替换渲染结果 |
| **过滤器** | `filter-html` | HTML 净化后 | 可修改输出 HTML |
| **过滤器** | `on-comment-creating` | 发表评论前 | 可修改/拒绝评论 |
| **动作 (Action)** | `on-post-created` | 文章创建后 | 无返回值，执行副作用 |
| **动作** | `on-post-deleted` | 文章删除后 | 无返回值 |
| **动作** | `on-comment-created` | 评论发表后 | 无返回值 |
| **动作** | `on-login` | 登录后 | 无返回值 |
| **路由 (Route)** | `handle-route` | 未匹配路由时 | 可返回自定义响应 |

### 4.2 调度流程

```
                    ┌──────────────────────┐
                    │  业务操作触发 Hook    │
                    │  e.g. service::create │
                    └──────────┬───────────┘
                               │
                               ▼
                    ┌──────────────────────┐
                    │  PluginManager       │
                    │  .dispatch(hook, ctx)│
                    └──────────┬───────────┘
                               │
                    ┌──────────▼───────────┐
                    │  遍历注册了此 Hook    │
                    │  的插件（按优先级）   │
                    └──────────┬───────────┘
                               │
               ┌───────────────┼───────────────┐
               ▼               ▼               ▼
         ┌──────────┐   ┌──────────┐   ┌──────────┐
         │ Plugin A │   │ Plugin B │   │ Plugin C │
         │ priority │   │ priority │   │ priority │
         │   = 10   │   │   = 20   │   │   = 30   │
         └────┬─────┘   └────┬─────┘   └────┬─────┘
              │              │              │
              ▼              ▼              ▼
         ┌─────────────────────────────────────────┐
         │  Filter 类型: 链式传递，每个插件          │
         │  接收上一个插件的输出作为输入             │
         │                                          │
         │  input → A.filter → B.filter → C.filter  │
         │          → 最终输出                       │
         │                                          │
         │  Action 类型: 顺序执行，忽略返回值        │
         │  input → A.action → B.action → C.action   │
         └──────────────────────────────────────────┘
```

### 4.3 Filter 链式调用示例

```
on-post-creating 链:

  原始输入: { title: "Hello World", content: "# Hello..." }
       │
       ▼
  Plugin A (SEO 优化插件):
    → 自动生成 excerpt
    → 输出: { title: "Hello World", content: "# Hello...", excerpt: "Hello..." }
       │
       ▼
  Plugin B (内容审核插件):
    → 检查内容合规，不做修改
    → 输出: { title: "Hello World", content: "# Hello...", excerpt: "Hello..." }
       │
       ▼
  最终结果传入 service::create_post()
```

---

## 5. 插件清单 (Manifest)

每个插件目录包含一个 `plugin.toml` 清单文件：

```toml
[plugin]
id = "com.example.seo-optimizer"
name = "SEO Optimizer"
version = "1.0.0"
description = "自动优化文章 SEO：生成 meta description、OG 标签"
author = "Example Corp"
license = "MIT"
runtime = "wasi"              # 或 "reactor" (无命令行入口)
language = "rust"             # rust | assemblyscript | go | c

[permissions]
http = ["cdn.example.com/*"]  # 允许访问的域名白名单
config = ["seo.*"]            # 允许读取的配置项前缀
db-read = ["posts"]           # 允许只读访问的表
db-write = []                 # 允许写入的表（默认空）
max_memory_mb = 32            # 内存上限
timeout_ms = 5000             # 单次 Hook 执行超时

[hooks]
on-post-creating = { priority = 10 }
render-markdown = { priority = 20 }
filter-html = { priority = 5 }
handle-route = { match = "/api/v1/plugins/seo/*" }
```

---

## 6. 目录结构

```
rust-blog/
├── plugins/                          # 插件根目录
│   ├── seo-optimizer/                # 插件目录 = 插件 ID (kebab-case)
│   │   ├── plugin.toml              # 清单文件
│   │   └── plugin.wasm              # 编译后的 WASM 文件
│   ├── content-filter/
│   │   ├── plugin.toml
│   │   └── plugin.wasm
│   └── webhook-notify/
│       ├── plugin.toml
│       └── plugin.wasm
├── plugins-sdk/                      # 插件开发 SDK (Rust crate)
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                    # SDK 入口，重导出所有类型
│       ├── host.rs                   # 宿主函数绑定 (自动生成)
│       ├── types.rs                  # 数据类型 (post, comment, etc.)
│       └── hooks.rs                  # Hook 注册宏
├── plugins-examples/                 # 示例插件源码
│   ├── seo-optimizer/
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   └── content-filter/
│       ├── Cargo.toml
│       └── src/lib.rs
└── wit/
    └── plugin.wit                    # 接口定义文件
```

---

## 7. 宿主侧实现设计

### 7.1 新增 Cargo 依赖

```toml
# Cargo.toml 新增
[dependencies]
wasmtime = "28"
wasmtime-wasi = "28"
wit-bindgen = "0.36"
toml = "0.8"
notify = { version = "7", features = ["macos_kqueue"] }  # 文件监听
```

### 7.2 核心数据结构

```rust
// src/plugins/mod.rs

/// 插件系统核心管理器
pub struct PluginManager {
    engine: wasmtime::Engine,
    linker: wasmtime::Linker<PluginState>,
    plugins: HashMap<String, LoadedPlugin>,
    hook_registry: HashMap<HookPoint, Vec<HookEntry>>,
    watcher: Option<notify::RecommendedWatcher>,
}

struct LoadedPlugin {
    id: String,
    manifest: PluginManifest,
    instance: wasmtime::component::Instance,
    store: wasmtime::Store<PluginState>,
    metadata: PluginMetadata,
}

struct HookEntry {
    plugin_id: String,
    priority: i32,
    hook_fn: HookFn,
}

enum HookFn {
    PostCreating(/* typed closure */),
    PostCreated(/* ... */),
    CommentCreating(/* ... */),
    RenderMarkdown(/* ... */),
    FilterHtml(/* ... */),
    HandleRoute(/* ... */),
    // ...
}

struct PluginState {
    config: Arc<AppConfig>,
    pool: SqlitePool,
    request_ctx: Option<RequestContext>,
}

#[derive(Debug, Clone)]
struct PluginManifest {
    id: String,
    name: String,
    version: String,
    description: String,
    language: String,
    permissions: Permissions,
    hooks: HashMap<String, HookConfig>,
}
```

### 7.3 PluginManager 生命周期

```
┌─────────────────────────────────────────────────────────────────┐
│                     PluginManager 启动流程                       │
│                                                                   │
│  ┌────────────────────────┐                                      │
│  │ 1. 创建 wasmtime::Engine│                                      │
│  │    配置: 内存上限/指令计数│                                      │
│  └────────────┬───────────┘                                      │
│               ▼                                                   │
│  ┌────────────────────────┐                                      │
│  │ 2. 创建 Linker         │                                      │
│  │    注册 host functions  │                                      │
│  │    (log/config/http)    │                                      │
│  └────────────┬───────────┘                                      │
│               ▼                                                   │
│  ┌────────────────────────┐                                      │
│  │ 3. 扫描 plugins/ 目录  │                                      │
│  │    解析 plugin.toml    │                                      │
│  │    加载 plugin.wasm    │                                      │
│  │    实例化 + 注册 hooks │                                      │
│  └────────────┬───────────┘                                      │
│               ▼                                                   │
│  ┌────────────────────────┐                                      │
│  │ 4. 启动文件监听器      │                                      │
│  │    监控 plugins/ 目录   │                                      │
│  │    检测 .wasm 文件变更  │                                      │
│  │    触发热重载           │                                      │
│  └────────────────────────┘                                      │
└─────────────────────────────────────────────────────────────────┘
```

### 7.4 Hook 调度实现骨架

```rust
impl PluginManager {
    /// 调度 Filter 类型 Hook（链式调用）
    pub async fn dispatch_filter<T: Clone + Serialize + for<'de> Deserialize<'de>>(
        &self,
        hook: HookPoint,
        input: T,
    ) -> AppResult<T> {
        let entries = self.hook_registry.get(&hook);
        if let Some(entries) = entries {
            // 按优先级排序
            let mut sorted = entries.to_vec();
            sorted.sort_by_key(|e| e.priority);

            let mut current = serde_json::to_value(input)?;
            for entry in sorted {
                let plugin = self.plugins.get(&entry.plugin_id)
                    .ok_or_else(|| AppError::Internal(anyhow::anyhow!(
                        "plugin {} not found", entry.plugin_id
                    )))?;

                // 调用插件的 WASM 函数，带超时
                current = tokio::time::timeout(
                    Duration::from_millis(plugin.manifest.permissions.timeout_ms),
                    plugin.call_filter(&entry.hook_fn, current),
                ).await
                    .map_err(|_| AppError::Internal(anyhow::anyhow!(
                        "plugin {} timed out on {:?}", entry.plugin_id, hook
                    )))??;
            }
            Ok(serde_json::from_value(current)?)
        } else {
            Ok(input)
        }
    }

    /// 调度 Action 类型 Hook（顺序执行，忽略返回值）
    pub async fn dispatch_action(
        &self,
        hook: HookPoint,
        data: &impl Serialize,
    ) -> AppResult<()> {
        // 类似 dispatch_filter，但无链式传递
        // ...
    }
}
```

### 7.5 与现有代码的集成点

在 `services/` 层注入 Hook 调用，Handler 层无需改动：

```rust
// services/post.rs — create_post 中的 Hook 调用示例

pub async fn create_post(
    pool: &sqlx::SqlitePool,
    plugins: &PluginManager,
    author_id: &str,
    mut req: CreatePostRequest,
) -> AppResult<PostResponse> {
    // ── Hook: on_post_creating ──
    req = plugins.dispatch_filter(HookPoint::PostCreating, req).await?;

    // ... 原有创建逻辑 ...
    let p = post::create(pool, &req.title, &slug, ...).await?;
    post::sync_tags(pool, &p.id, tag_ids).await?;

    // ── Hook: on_post_created ──
    let resp = build_post_response_from_id(pool, &p.id).await?;
    plugins.dispatch_action(HookPoint::PostCreated, &resp).await?;

    Ok(resp)
}
```

### 7.6 自定义路由 Hook

当 axum 路由未匹配时，fallback 到插件路由系统：

```rust
// server/mod.rs — 添加 fallback 路由

let api_v1 = axum::Router::new()
    // ... 现有路由 ...
    .fallback(|State(state): State<AppState>, req: Request| async move {
        // 尝试匹配插件注册的自定义路由
        match state.plugins.dispatch_route(&req).await {
            Some(response) => response,
            None => (StatusCode::NOT_FOUND, "not found").into_response(),
        }
    });
```

---

## 8. 插件开发 (SDK 侧)

### 8.1 示例：SEO 优化插件 (Rust)

```rust
// plugins-examples/seo-optimizer/src/lib.rs

use rust_blog_plugin_sdk::*;

guest_bindgen!();  // wit-bindgen 生成宏

struct SeoOptimizer;

impl Guest for SeoOptimizer {
    type PluginId = SeoOptimizerMetadata;

    fn on_post_creating(mut post: PostInput) -> PostInput {
        // 自动从内容提取 excerpt（如果未提供）
        if post.excerpt.is_none() {
            let plain = strip_markdown(&post.content);
            post.excerpt = Some(truncate(&plain, 200));
        }
        post
    }

    fn filter_html(html: String) -> String {
        // 注入 Open Graph meta 标签
        inject_og_tags(html)
    }
}

struct SeoOptimizerMetadata;

impl GuestPluginId for SeoOptimizerMetadata {
    fn name() -> String { "SEO Optimizer".into() }
    fn version() -> String { "1.0.0".into() }
    fn description() -> String {
        "Auto-generate meta descriptions and OG tags".into()
    }
}

fn strip_markdown(md: &str) -> String {
    md.chars()
        .filter(|c| !matches!(c, '#' | '*' | '_' | '`' | '[' | ']' | '(' | ')'))
        .collect()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max]) }
}

fn inject_og_tags(html: String) -> String {
    // 在 <head> 中注入 OG 标签
    let og = r#"<meta property="og:type" content="article">"#;
    html.replacen("<head>", &format!("<head>{}", og), 1)
}
```

### 8.2 插件编译

```bash
# 编译 Rust 插件为 WASM 组件
cd plugins-examples/seo-optimizer
cargo build --target wasm32-wasip1 --release
# 或使用 wasm-component-builder
wasm-tools component new target/wasm32-wasip1/release/seo_optimizer.wasm \
    -o ../../plugins/seo-optimizer/plugin.wasm

# AssemblyScript 插件
asc assembly/index.ts --outFile ../../plugins/my-plugin/plugin.wasm \
    --use abort=wasi:cli/run
```

---

## 9. 安全模型

### 9.1 沙箱限制

```
┌───────────────────────────────────────────────────────┐
│                  WASM 沙箱边界                         │
│                                                        │
│  ┌─────────────────────────────────────────────┐      │
│  │  内存限制: 32MB (可配置)                     │      │
│  │  CPU 限制: 指令计数器 + timeout              │      │
│  │  文件系统: 无直接访问                        │      │
│  │  网络: 仅通过 host-http-get (白名单)         │      │
│  │  数据库: 仅通过 host-db-query (权限控制)     │      │
│  │  环境: 无法读取宿主环境变量                  │      │
│  └─────────────────────────────────────────────┘      │
│                                                        │
│  宿主控制的所有外部交互:                               │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐            │
│  │host_log() │  │host_http │  │host_db   │            │
│  │          │  │  _get()  │  │ _query() │            │
│  └──────────┘  └──────────┘  └──────────┘            │
│                   │              │                     │
│                   ▼              ▼                     │
│             域名白名单     表权限检查                   │
└───────────────────────────────────────────────────────┘
```

### 9.2 权限声明与执行

```rust
// 权限检查在 host function 实现中执行

fn host_http_get(mut cx: FunctionContext, url_ptr: u32, url_len: u32) -> u32 {
    let url = cx.read_string(url_ptr, url_len);
    let plugin_id = cx.data().current_plugin_id();

    // 检查 URL 是否在白名单中
    if !cx.data().is_url_allowed(plugin_id, &url) {
        return cx.write_error("URL not in whitelist");
    }

    // 执行请求
    match blocking_http_get(&url) {
        Ok(body) => cx.write_string(&body),
        Err(e) => cx.write_error(&e.to_string()),
    }
}
```

### 9.3 资源限制

| 资源 | 默认限制 | 可配置 |
|------|---------|-------|
| 内存 | 32 MB | `max_memory_mb` in manifest |
| 单次 Hook 执行时间 | 5s | `timeout_ms` in manifest |
| HTTP 请求大小 | 1 MB | 硬编码上限 |
| 并发插件数 | 无限制 | 可在宿主配置限制 |
| HTTP 外发请求频率 | 10/min/plugin | 运行时令牌桶 |

---

## 10. 配置集成

### 10.1 环境变量新增

```env
# .env 新增
PLUGIN_DIR=./plugins              # 插件目录
PLUGIN_HOT_RELOAD=true            # 是否启用热加载
PLUGIN_MAX_MEMORY_MB=32           # 全局默认内存限制
PLUGIN_DEFAULT_TIMEOUT_MS=5000    # 全局默认超时
PLUGIN_DISABLED=                  # 禁用的插件列表（逗号分隔）
```

### 10.2 AppConfig 扩展

```rust
pub struct AppConfig {
    // ... 现有字段 ...

    pub plugin_dir: String,
    pub plugin_hot_reload: bool,
    pub plugin_max_memory_mb: u32,
    pub plugin_default_timeout_ms: u64,
    pub plugin_disabled: Vec<String>,
}
```

---

## 11. 热加载机制

```
┌──────────────────────────────────────────────────────────────┐
│                     热加载流程                                 │
│                                                                │
│  文件系统监听器 (notify crate)                                 │
│       │                                                        │
│       │ 检测到 plugins/xxx/plugin.wasm 变更                   │
│       ▼                                                        │
│  ┌────────────────────────┐                                   │
│  │ 1. 读取 plugin.toml    │                                   │
│  │    验证清单完整性       │                                   │
│  └────────────┬───────────┘                                   │
│               ▼                                                │
│  ┌────────────────────────┐                                   │
│  │ 2. 卸载旧版本插件       │                                   │
│  │    - 从 hook_registry  │                                   │
│  │      移除所有 Hook 条目│                                   │
│  │    - Drop 旧 Store     │                                   │
│  └────────────┬───────────┘                                   │
│               ▼                                                │
│  ┌────────────────────────┐                                   │
│  │ 3. 加载新版本           │                                   │
│  │    - 编译为新 Instance  │                                   │
│  │    - 注册所有 Hook      │                                   │
│  │    - 验证权限声明       │                                   │
│  └────────────┬───────────┘                                   │
│               ▼                                                │
│  ┌────────────────────────┐                                   │
│  │ 4. 打印加载日志         │                                   │
│  │    tracing::info!(     │                                   │
│  │      "reloaded plugin" │                                   │
│  │    )                    │                                   │
│  └────────────────────────┘                                   │
│                                                                │
│  注意：热加载期间正在执行的 Hook 会完成后再切换               │
│  使用 Arc<RwLock<PluginManager>> 保证并发安全                 │
└──────────────────────────────────────────────────────────────┘
```

---

## 12. 实施计划

### Phase 1：基础设施 (3-4 天)

- [ ] 添加 wasmtime + wit-bindgen 依赖
- [ ] 定义 `wit/plugin.wit` 接口文件
- [ ] 实现 `PluginManifest` 解析 (plugin.toml)
- [ ] 实现 `PluginManager` 基本骨架（加载/卸载/实例化）
- [ ] 编写 `plugins-sdk` crate 骨架
- [ ] 单元测试：插件加载/卸载

### Phase 2：Hook 系统 (3-4 天)

- [ ] 实现 Hook 注册表和优先级排序
- [ ] 实现 `dispatch_filter` / `dispatch_action` 调度器
- [ ] 注册 Host Functions（log / config）
- [ ] 在 `services/post.rs` 注入 `on_post_creating` / `on_post_created`
- [ ] 在 `services/comment.rs` 注入 `on_comment_creating`
- [ ] 编写示例插件 `seo-optimizer` 作为端到端验证

### Phase 3：Host API + 权限 (3-4 天)

- [ ] 实现 `host_http_get`（带域名白名单）
- [ ] 实现 `host_get_config`（带 key 前缀过滤）
- [ ] 实现指令计数器 + timeout 机制
- [ ] 实现内存限制
- [ ] 编写 `content-filter` 示例插件（调用 host API）

### Phase 4：高级功能 (3-4 天)

- [ ] 实现 `render_markdown` Hook（可替换 Markdown 渲染器）
- [ ] 实现 `handle_route` 自定义路由
- [ ] 实现热加载（notify 文件监听）
- [ ] AppConfig 扩展（plugin 配置项）
- [ ] 性能基准测试

### Phase 5：文档与生态 (2-3 天)

- [ ] 编写插件开发指南（中文）
- [ ] 提供更多示例插件模板
- [ ] 集成测试覆盖所有 Hook 点
- [ ] CI 中添加 WASM 目标构建

---

## 13. 性能考量

| 操作 | 预期开销 | 说明 |
|------|---------|------|
| 首次加载插件 | ~10-50ms | Cranelift JIT 编译 |
| 调用 Hook（无插件注册） | <1μs | HashMap 查找，空列表直接返回 |
| 调用 Hook（有插件） | ~10-100μs | WASM → Host 上下文切换 + 执行 |
| 热加载 | ~50-100ms | 编译 + 替换实例 |
| 内存（每插件） | 1-32MB | WASM 线性内存 |

优化策略：
- 无插件注册的 Hook 点零开销（编译期内联检查）
- 使用 `wasmtime::component::InstancePre` 预编译，加速热加载
- 高频 Hook（如 `render_markdown`）可缓存结果
- 插件调用使用 `tokio::task::spawn_blocking` 避免阻塞异步运行时

---

## 14. 总结

```
宿主 (rust-blog)                插件 (.wasm)
┌─────────────────────┐        ┌─────────────────────┐
│                     │        │                     │
│  Handler → Service  │ Hook   │  自定义逻辑         │
│      ↓              │───────►│  (任意 WASM 语言)   │
│  PluginManager      │◄───────│                     │
│      ↓              │ Host   │  可调用:            │
│  Model / DB         │ API    │  log / config / http│
│                     │        │                     │
└─────────────────────┘        └─────────────────────┘
         │                              ▲
         │  wasmtime (沙箱)             │
         └──────────────────────────────┘

关键特性:
  ✅ 沙箱隔离 — 插件无法访问宿主资源
  ✅ 热加载 — 替换 .wasm 文件即时生效
  ✅ 类型安全 — WIT 定义强类型接口
  ✅ 多语言 — Rust / AS / Go / C
  ✅ 零侵入 — 插件不存在时走默认逻辑
```
