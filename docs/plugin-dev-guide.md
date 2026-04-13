# 插件开发指南

> rust-blog 支持两种插件运行时：**WASM**（Rust 编写）和 **JavaScript**（QuickJS）。
> 本文档聚焦 JS 插件开发。

---

## 快速开始

### 1. 创建插件目录

```
plugins/my-plugin/
├── plugin.toml    # 插件清单
└── index.js       # JS 入口
```

### 2. 编写 plugin.toml

```toml
[plugin]
id = "com.example.my-plugin"      # 全局唯一 ID（反向域名格式）
name = "My Plugin"                 # 显示名称
version = "1.0.0"                  # 语义化版本
description = "插件描述"
author = "Your Name"
license = "MIT"
runtime = "js"                     # 必须设为 "js"
language = "javascript"            # 或 "typescript"
entry = "index.js"                 # JS 入口文件名（默认 index.js）

[permissions]
max_memory_mb = 8                  # 内存限制（默认 32MB）
timeout_ms = 2000                  # 单次 Hook 执行超时（默认 5000ms）

[hooks.on_post_creating]           # 注册的 Hook，key 用下划线或短横线均可
priority = 10                      # 优先级，数字越小越先执行

[hooks.filter_html]
priority = 20
```

### 3. 编写 index.js

```javascript
var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        // 修改输入数据
        input.title = input.title.trim();
        return JSON.stringify(input);
    },

    filter_html: function(html) {
        // 在 <head> 后注入 OG 标签
        return html.replace("<head>", '<head><meta property="og:type" content="article">');
    }
};
```

### 4. 部署

```bash
# 手动复制
cp -r plugins-examples-js/my-plugin/ plugins/my-plugin/

# 或用 justfile（需添加到 justfile）
just plugins-js-build
```

---

## Hook 类型

### JSON Filter — 修改数据

接收 JSON 字符串，返回修改后的 JSON 字符串。

| Hook | 触发时机 | 数据内容 |
|------|----------|----------|
| `on_post_creating` | 文章创建前 | `{ title, content, excerpt, category_id, ... }` |
| `on_post_updating` | 文章更新前 | 同上 + `id` |
| `on_comment_creating` | 评论创建前 | `{ content, post_id, parent_id, ... }` |

```javascript
on_post_creating: function(inputJson) {
    var input = JSON.parse(inputJson);
    input.excerpt = input.content.substring(0, 200);
    return JSON.stringify(input);  // 必须返回 JSON 字符串
}
```

### JSON Action — 通知/副作用

接收 JSON 字符串，无返回值。适合日志、通知、缓存清理等。

| Hook | 触发时机 | 数据内容 |
|------|----------|----------|
| `on_post_created` | 文章创建后 | `{ id, title, slug, ... }` |
| `on_post_updated` | 文章更新后 | 同上 |
| `on_post_deleted` | 文章删除后 | `{ id }` |
| `on_comment_created` | 评论创建后 | `{ id, content, post_id, ... }` |
| `on_login` | 用户登录后 | `{ email, success }` |

```javascript
on_post_created: function(dataJson) {
    var data = JSON.parse(dataJson);
    Host.log("info", "New post: " + data.title);
}
```

### String Filter — 修改原始字符串

| Hook | 触发时机 | 输入 | 返回 |
|------|----------|------|------|
| `render_markdown` | Markdown 渲染（替代默认渲染器） | Markdown 原文 | HTML 字符串 |
| `filter_html` | HTML 后处理 | HTML 字符串 | 修改后的 HTML |

```javascript
render_markdown: function(content) {
    // 自定义渲染逻辑
    return "<p>" + content + "</p>";
}
```

### Route Handler — 自定义路由

```javascript
handle_route: function(routeJson) {
    var route = JSON.parse(routeJson);
    // route.path, route.method
    return JSON.stringify({
        status: 200,
        body: JSON.stringify({ message: "Hello from plugin!" })
    });
}
```

需要在 `plugin.toml` 中配置 `match` 模式：

```toml
[hooks.handle_route]
match = "/api/v1/custom/*"    # glob 风格，* 匹配单段路径
priority = 5
```

---

## 宿主 API

插件通过全局 `Host` 对象与宿主交互。

### Host.log(level, message)

写入宿主日志。

```javascript
Host.log("info", "这条消息会出现在宿主日志中");
Host.log("warn", "警告信息");
Host.log("error", "错误信息");
```

### Host.getConfig(key)

读取宿主配置。返回字符串或 `null`。

```javascript
var env = Host.getConfig("app.env");        // "development" / "production"
var port = Host.getConfig("app.port");       // "3000"
var baseUrl = Host.getConfig("app.base_url"); // "http://localhost:3000"
var maxSize = Host.getConfig("upload.max_size");
```

支持的 key：

| Key | 说明 |
|-----|------|
| `app.host` | 监听地址 |
| `app.port` | 监听端口 |
| `app.env` | 运行环境 |
| `app.base_url` | 站点 URL |
| `jwt.access_expires` | Access Token 过期时间（秒） |
| `jwt.refresh_expires` | Refresh Token 过期时间（秒） |
| `upload.dir` | 上传目录 |
| `upload.max_size` | 上传大小限制（字节） |
| `plugin.max_memory_mb` | 插件内存限制 |
| `plugin.default_timeout_ms` | 插件超时时间 |

> 注意：`jwt.secret` 和 `database_url` 等敏感配置不暴露给插件。

---

## TypeScript 开发

### 1. 安装 SDK 类型定义

在插件目录创建 `tsconfig.json`：

```json
{
  "compilerOptions": {
    "target": "ES2021",
    "module": "ES2020",
    "strict": true,
    "noEmit": true,
    "baseUrl": "..",
    "paths": {
      "rust-blog-plugin-sdk": ["plugins-sdk-js"]
    }
  }
}
```

### 2. 编写 TypeScript

```typescript
/// <reference path="../../plugins-sdk-js/index.d.ts" />

var Plugin: PluginHooks = {
    on_post_creating(inputJson: string): string {
        var input = JSON.parse(inputJson);
        input.excerpt = input.content.substring(0, 200);
        return JSON.stringify(input);
    }
};
```

### 3. 编译为 JS

```bash
# 用 esbuild 编译
npx esbuild plugins-examples-js/my-plugin/src/index.ts \
    --outfile=plugins-examples-js/my-plugin/index.js \
    --bundle --format=iife --target=es2021

# 或用 justfile
just plugins-ts-build
```

---

## 安全限制

JS 插件运行在 QuickJS 沙箱中：

| 限制 | 说明 |
|------|------|
| **内存** | 默认 32MB，可在 `[permissions]` 中配置 |
| **超时** | 默认 5000ms，超时自动中断 |
| **无文件系统** | 不能读写文件 |
| **无网络** | 不能发起 HTTP 请求 |
| **隔离作用域** | 每个插件独立的全局对象，互不干扰 |

---

## 热重载

当 `PLUGIN_HOT_RELOAD=true` 时，修改 `plugins/` 目录下的 `.js` 或 `.wasm` 文件会自动触发插件重载：

```
 PLUGIN_HOT_RELOAD=true cargo run
```

> 热重载仅监听文件变化，不会自动编译 TypeScript。开发 TS 插件时需手动或用 watch 模式运行 esbuild。

---

## 完整示例

### Welcome Email（JS）

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

### SEO Optimizer（JS）

```javascript
// plugins/seo-optimizer-js/index.js
var Plugin = {
    on_post_creating: function(inputJson) {
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

    filter_html: function(html) {
        var meta = '<meta property="og:type" content="article">';
        return html.replace("<head>", "<head>" + meta);
    }
};
```

---

## 调试技巧

1. **查看日志** — `Host.log()` 输出到宿主 tracing 日志，开发环境默认打印到终端
2. **错误不崩溃** — 插件 Hook 抛异常时，宿主会跳过该插件继续执行，不影响请求
3. **禁用插件** — 在 `.env` 中设置 `PLUGIN_DISABLED=com.example.bad-plugin`
4. **超时测试** — 设置短超时 `[permissions] timeout_ms = 100` 来验证中断机制
