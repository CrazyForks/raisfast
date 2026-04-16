# Extension 系统设计文档

> 2026-04-16 · 将 Content Type 和 Plugin 统一为 Extension 概念，实现安装/卸载/版本管理一体化。

---

## 目录

- [1. 背景与动机](#1-背景与动机)
- [2. 设计目标](#2-设计目标)
- [3. 目录结构](#3-目录结构)
- [4. Extension 清单格式](#4-extension-清单格式)
- [5. 数据模型](#5-数据模型)
- [6. 核心架构](#6-核心架构)
  - [6.1 ExtensionManager](#61-extensionmanager)
  - [6.2 ExtensionManifest](#62-extensionmanifest)
  - [6.3 与现有系统的关系](#63-与现有系统的关系)
- [7. 启动流程改造](#7-启动流程改造)
- [8. Extension 生命周期](#8-extension-生命周期)
- [9. API 设计](#9-api-设计)
- [10. 向后兼容](#10-向后兼容)
- [11. 迁移策略](#11-迁移策略)
- [12. 实施计划](#12-实施计划)

---

## 1. 背景与动机

### 现状问题

当前系统中 Content Type 和 Plugin 是两个完全独立的子系统：

| 维度 | Content Type | Plugin |
|---|---|---|
| 目录 | `content_types/*.toml` | `plugins/*/plugin.toml` |
| 加载器 | `ContentTypeRegistry` | `PluginManager` |
| 加载时机 | `server/mod.rs:155` | `server/mod.rs:172` |
| 热加载 | Admin API 触发 | 文件监听（notify） |
| 安装状态 | 无记录（文件即存在） | 无记录（文件即存在） |
| 卸载 | 删除 TOML + 手动删表 | 删除目录 |
| 版本管理 | 无 | 无 |
| 依赖关系 | 无 | 有（`dependencies` 字段） |

### 核心矛盾

**实际业务场景中，一个扩展通常同时需要 Content Type 和 Plugin。** 例如：

- **电商扩展**：需要 `product`/`order`/`cart_item` Content Type + 支付/库存扣减插件逻辑
- **SEO 扩展**：只需要插件（Hook 拦截内容生成 sitemap/meta）
- **博客核心**：只需要 Content Type（post/category/tag），不需要插件代码

当前架构无法将这些作为一个整体安装、卸载、版本管理。

### 现有代码中的预示

Plugin manifest 已经声明了 `content_types` 字段（`src/plugins/manifest.rs:19-20`），但从未被处理。本次设计正式实现这一概念。

---

## 2. 设计目标

1. **统一概念**：Content Type 和 Plugin 都归入 Extension，一个 Extension 可包含任意组合（仅 CT / 仅 Plugin / CT + Plugin）
2. **DB 持久化**：Extension 安装状态、版本、启用/禁用记录到数据库
3. **生命周期管理**：安装、启用、禁用、卸载以 Extension 为原子单位
4. **依赖管理**：Extension 间依赖，拓扑排序加载
5. **向后兼容**：`plugins/` 目录保留兼容，`content_types/` 目录废弃（迁移到 Extension）
6. **热加载**：统一 Extension 级别的文件监听 + Admin API

---

## 3. 目录结构

### 新目录

```
项目根/
├── extensions/                    ← 新增：Extension 根目录
│   ├── blog-core/
│   │   ├── extension.toml         ← Extension 清单
│   │   └── content_types/         ← 仅 CT（无 plugin）
│   │       ├── post.toml
│   │       ├── category.toml
│   │       └── tag.toml
│   ├── seo-optimizer/
│   │   ├── extension.toml
│   │   └── plugin/                ← 仅 Plugin（无 CT）
│   │       ├── manifest.toml
│   │       └── main.js
│   └── ecommerce/
│       ├── extension.toml
│       ├── content_types/          ← CT + Plugin 组合
│       │   ├── product.toml
│       │   ├── order.toml
│       │   └── cart_item.toml
│       └── plugin/
│           ├── manifest.toml
│           └── main.js
├── plugins/                       ← 保留：向后兼容旧 Plugin
├── content_types/                 ← 废弃：迁移到 extensions/
└── ...
```

### Extension 三种形态

| 形态 | 目录内容 | 示例 |
|---|---|---|
| 纯 CT | `extension.toml` + `content_types/` | blog-core |
| 纯 Plugin | `extension.toml` + `plugin/` | seo-optimizer |
| CT + Plugin | `extension.toml` + `content_types/` + `plugin/` | ecommerce |

---

## 4. Extension 清单格式

### extension.toml

```toml
[extension]
# ── 必填 ──────────────────────────
id = "ecommerce"                        # 全局唯一标识（kebab-case）
name = "E-Commerce"                     # 显示名称
version = "1.0.0"                       # 语义化版本

# ── 可选 ──────────────────────────
description = "E-commerce extension with products, orders and payment logic"
author = "Team"
license = "MIT"
homepage = "https://github.com/example/ecommerce-ext"

# 依赖的其他 Extension（id → version range）
[extension.dependencies]
blog-core = ">=1.0.0"
payment-gateway = ">=0.5.0"

# ── 组件引用（相对于 extension 根目录）──

# Content Type 目录路径，null/省略 = 无 CT
content_types = "content_types/"

# Plugin manifest 路径，null/省略 = 无 Plugin
plugin = "plugin/manifest.toml"
```

### 字段说明

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `id` | string | 是 | 全局唯一，kebab-case，与 DB 记录对应 |
| `name` | string | 是 | 显示名称 |
| `version` | string | 是 | 语义化版本（semver） |
| `description` | string | 否 | 描述 |
| `author` | string | 否 | 作者 |
| `license` | string | 否 | 开源协议 |
| `homepage` | string | 否 | 主页 URL |
| `dependencies` | map | 否 | 依赖的 Extension，key 为 id，value 为 semver range |
| `content_types` | string/null | 否 | Content Type 目录相对路径 |
| `plugin` | string/null | 否 | Plugin manifest 相对路径 |

### 与现有 Plugin manifest 的关系

Plugin 的 `plugin.toml`（现 `manifest.toml`）保持不变，`extension.toml` 是外层包装：

```
extension.toml    → Extension 级元信息（id/version/dependencies/组件引用）
  └─ plugin/manifest.toml → Plugin 级配置（runtime/hooks/permissions/cron/routes）
  └─ content_types/*.toml → Content Type Schema（复用现有格式）
```

Extension 不重复定义 Plugin 细节（runtime、hooks、permissions），而是通过 `plugin` 字段引用 Plugin manifest。

---

## 5. 数据模型

### 新增 Migration：`014_extensions.sql`

```sql
CREATE TABLE IF NOT EXISTS extensions (
    id          TEXT PRIMARY KEY,           -- extension id (kebab-case)
    name        TEXT NOT NULL,              -- 显示名称
    version     TEXT NOT NULL,              -- 当前安装版本
    enabled     INTEGER NOT NULL DEFAULT 1, -- 是否启用（0=禁用, 1=启用）
    config      TEXT,                       -- JSON，扩展级配置（预留）
    installed_at TEXT NOT NULL,             -- 首次安装时间（ISO 8601）
    updated_at  TEXT NOT NULL,              -- 最后更新时间（ISO 8601）
    tenant_id   TEXT                        -- 多租户（预留，当前为 NULL）
);
```

### 数据示例

| id | name | version | enabled | installed_at | updated_at |
|---|---|---|---|---|---|
| blog-core | Blog Core | 1.0.0 | 1 | 2026-04-16T10:00:00+08:00 | 2026-04-16T10:00:00+08:00 |
| ecommerce | E-Commerce | 1.2.0 | 1 | 2026-04-16T10:05:00+08:00 | 2026-04-20T15:30:00+08:00 |
| seo-optimizer | SEO Optimizer | 0.3.0 | 0 | 2026-04-16T10:10:00+08:00 | 2026-04-16T10:10:00+08:00 |

### 设计决策

- **不存储 content_type 列表**：CT 列表从 `extension.toml` 的 `content_types` 目录动态读取，避免数据不一致
- **不存储 plugin 信息**：Plugin 详情从 `plugin/manifest.toml` 动态读取
- **`enabled` 字段**：禁用时不加载 CT 和 Plugin，但保留 DB 记录和文件
- **`config` JSON**：预留给 Extension 级配置（如 API Key、第三方服务地址），未来由 Admin UI 管理

---

## 6. 核心架构

### 6.1 ExtensionManager

新增 `src/extension/mod.rs`，作为 Extension 生命周期的统一入口：

```rust
// src/extension/mod.rs

pub mod manager;
pub mod manifest;
pub mod model;
pub mod service;
pub mod handler;
```

```rust
/// Extension 管理器
///
/// 统一管理 Extension 的发现、加载、启用、禁用、卸载。
/// 内部协调 ContentTypeRegistry 和 PluginManager。
pub struct ExtensionManager {
    /// 已加载的 Extension 列表
    extensions: RwLock<HashMap<String, LoadedExtension>>,
    /// Extension 清单解析结果
    manifests: RwLock<HashMap<String, ExtensionManifest>>,
    /// Content Type 注册表（复用现有）
    ct_registry: Arc<ContentTypeRegistry>,
    /// Plugin 管理器（复用现有）
    plugin_manager: Arc<PluginManager>,
    /// 数据库连接池
    pool: Pool,
    /// Extension 根目录
    extension_dir: PathBuf,
    /// DB 安装状态缓存
    installed: RwLock<HashMap<String, InstalledExtension>>,
}

/// 已加载的 Extension
pub struct LoadedExtension {
    /// 清单
    manifest: ExtensionManifest,
    /// 加载的 Content Type schema 列表
    content_types: Vec<ContentTypeSchema>,
    /// 是否包含 Plugin（plugin 已由 PluginManager 管理）
    has_plugin: bool,
    /// 是否启用
    enabled: bool,
}
```

### 6.2 ExtensionManifest

```rust
/// extension.toml 解析结果
#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionManifest {
    pub extension: ExtensionInfo,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub author: Option<String>,
    pub license: Option<String>,
    pub homepage: Option<String>,
    #[serde(default)]
    pub dependencies: HashMap<String, String>,
    /// Content Type 目录相对路径（None = 无 CT）
    pub content_types: Option<String>,
    /// Plugin manifest 相对路径（None = 无 Plugin）
    pub plugin: Option<String>,
}
```

### 6.3 与现有系统的关系

```
                    ┌─────────────────────┐
                    │  ExtensionManager   │  ← 新增，统一入口
                    │  (extension/mod.rs) │
                    └──────┬──────┬───────┘
                           │      │
              ┌────────────┘      └────────────┐
              ▼                                ▼
   ┌──────────────────┐            ┌──────────────────┐
   │ ContentTypeRegistry│           │  PluginManager   │
   │ (content_type/)   │           │  (plugins/)      │
   │  复用，不改动      │           │  复用，不改动     │
   └──────────────────┘            └──────────────────┘
```

**核心原则：ExtensionManager 是编排层，不替代 ContentTypeRegistry 和 PluginManager。**

- `ContentTypeRegistry` 的 `register()` / `unregister()` 接口不变
- `PluginManager` 的 `load_plugin_from_dir()` / `unload_plugin()` 接口不变
- `ExtensionManager` 在加载 Extension 时调用两者的现有方法

### AppState 变更

```rust
// 之前
pub struct AppState {
    pub plugins: Arc<PluginManager>,
    pub content_type_registry: Arc<ContentTypeRegistry>,
    // ...
}

// 之后（新增 extension_manager，保留原字段以兼容）
pub struct AppState {
    pub extension_manager: Arc<ExtensionManager>,   // 新增
    pub plugins: Arc<PluginManager>,                 // 保留
    pub content_type_registry: Arc<ContentTypeRegistry>, // 保留
    // ...
}
```

`extension_manager` 持有 `ct_registry` 和 `plugin_manager` 的 `Arc` 引用，可以通过它统一操作。

---

## 7. 启动流程改造

### 当前流程

```
server::start()
├── metrics::init()
├── set_site_tz()
├── build_app()
│   ├── init_pool()
│   ├── load_content_types(config, &pool)     ← 直接扫描 content_types/
│   │   ├── ContentTypeRegistry::load_from_dir()
│   │   └── repo.migrate() for each
│   ├── PluginManager::new_with_options()      ← 直接扫描 plugins/
│   │   └── load_all()
│   ├── spawn_event/audit/webhook_subscribers()
│   ├── spawn_workers()
│   └── build_router()
└── axum::serve()
```

### 改造后流程

```
server::start()
├── metrics::init()
├── set_site_tz()
├── build_app()
│   ├── init_pool()
│   ├── ContentTypeRegistry::new()             ← 空注册表
│   ├── PluginManager::new_empty()             ← 空 PluginManager（不自动扫描）
│   ├── ExtensionManager::new(ct_reg, pm, pool, config)
│   │   ├── load_installed_from_db()           ← 读取已安装记录
│   │   ├── discover_extensions()              ← 扫描 extensions/ 目录
│   │   ├── resolve_dependencies()             ← 拓扑排序
│   │   └── for each ext in order:
│   │       ├── parse extension.toml
│   │       ├── 有 content_types? → ct_registry.register() + migrate()
│   │       └── 有 plugin? → plugin_manager.load_plugin_from_dir()
│   ├── load_legacy_plugins(config, &pm)       ← 向后兼容 plugins/ 目录
│   ├── spawn_event/audit/webhook_subscribers()
│   ├── spawn_workers()
│   └── build_router()
└── axum::serve()
```

### 关键变更

1. **ContentTypeRegistry 不再自行扫描目录**：由 ExtensionManager 调用 `register()`
2. **PluginManager 新增 `new_empty()` 构造**：不自动扫描，由 ExtensionManager 调用 `load_plugin_from_dir()`
3. **ExtensionManager 是唯一加载入口**：所有 CT 和 Plugin 都通过它编排
4. **向后兼容**：`load_legacy_plugins()` 处理 `plugins/` 目录下的旧插件

### PluginManager 改造

```rust
impl PluginManager {
    /// 现有方法保持不变：完整初始化 + 自动扫描加载
    pub async fn new_with_options(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self>;

    /// 新增：空构造，不扫描目录，由 ExtensionManager 调度
    pub async fn new_empty(config: Arc<AppConfig>, opts: PluginManagerOptions) -> Arc<Self>;

    /// 现有方法保持不变：加载单个插件
    pub async fn load_plugin_from_dir(&self, dir: &Path) -> anyhow::Result<()>;

    /// 现有方法保持不变：卸载单个插件
    pub async fn unload_plugin(&self, plugin_id: &str) -> anyhow::Result<()>;
}
```

`new_empty()` 与 `new_with_options()` 的区别仅在于最后不调用 `load_all()`。内部引擎初始化（wasmtime/QuickJS/mlua）、channel 创建、watcher 启动都保持一致。

---

## 8. Extension 生命周期

### 8.1 安装（Install）

```
Admin API: POST /admin/extensions/install
Body: { "path": "/path/to/extension-dir" } 或通过上传 .tar.gz
```

流程：
1. 复制文件到 `extensions/{id}/`
2. 解析 `extension.toml`，校验格式
3. 检查 `dependencies` 是否已安装
4. 写入 `extensions` 表（enabled=1）
5. 加载 CT → 注册到 ContentTypeRegistry → migrate
6. 加载 Plugin → 委托 PluginManager
7. 返回安装结果

### 8.2 启用（Enable）

```
Admin API: POST /admin/extensions/{id}/enable
```

流程：
1. 校验 dependencies 是否都启用
2. 更新 DB `enabled = 1`
3. 加载 CT + Plugin（同 Install 步骤 5-6）
4. 触发 EventBus `ExtensionEnabled` 事件

### 8.3 禁用（Disable）

```
Admin API: POST /admin/extensions/{id}/disable
```

流程：
1. 检查是否有其他启用的 Extension 依赖此 Extension
2. 卸载 Plugin → `plugin_manager.unload_plugin()`
3. 注销 CT → `ct_registry.unregister()`
4. 更新 DB `enabled = 0`
5. 触发 EventBus `ExtensionDisabled` 事件
6. **注意：不删除数据库表和数据**，仅从内存移除

### 8.4 卸载（Uninstall）

```
Admin API: DELETE /admin/extensions/{id}
Body: { "drop_tables": false }  // 可选：是否删除 CT 对应的数据库表
```

流程：
1. 先执行禁用流程
2. 删除 `extensions/{id}/` 目录
3. 删除 DB 记录
4. 可选：`DROP TABLE` 删除 CT 创建的表
5. 触发 EventBus `ExtensionUninstalled` 事件

### 8.5 更新（Update）

```
Admin API: POST /admin/extensions/{id}/update
Body: { "path": "/path/to/new-version" } 或上传 .tar.gz
```

流程：
1. 比对版本号，校验新版本
2. 执行禁用（旧版本）
3. 替换文件
4. 执行启用（新版本）
5. 更新 DB `version` 和 `updated_at`
6. 触发 EventBus `ExtensionUpdated` 事件

### 生命周期状态机

```
  ┌──────────┐  install   ┌──────────┐
  │  不存在   │ ──────────→│  已安装   │
  └──────────┘            └────┬─────┘
                               │
                      enable ──┤── disable
                               │
                          ┌────▼─────┐
                          │  已启用   │ ←→ 热重载
                          └────┬─────┘
                               │
                      update ──┤
                               │
                          ┌────▼─────┐
                          │ 已启用(v2)│
                          └────┬─────┘
                               │
                      uninstall┤
                               │
                          ┌────▼─────┐
                          │  已卸载   │ → 删除文件 + DB
                          └──────────┘
```

---

## 9. API 设计

### Admin API 端点

| 方法 | 路径 | 说明 |
|---|---|---|
| `GET` | `/admin/extensions` | 列出所有 Extension（含未安装） |
| `GET` | `/admin/extensions/{id}` | 获取 Extension 详情 |
| `POST` | `/admin/extensions/install` | 安装 Extension |
| `POST` | `/admin/extensions/{id}/enable` | 启用 |
| `POST` | `/admin/extensions/{id}/disable` | 禁用 |
| `DELETE` | `/admin/extensions/{id}` | 卸载 |
| `POST` | `/admin/extensions/{id}/update` | 更新版本 |

### GET /admin/extensions 响应示例

```json
{
  "data": [
    {
      "id": "blog-core",
      "name": "Blog Core",
      "version": "1.0.0",
      "description": "Core blog content types",
      "enabled": true,
      "installed_at": "2026-04-16T10:00:00+08:00",
      "has_content_types": true,
      "has_plugin": false,
      "content_types": ["post", "category", "tag"],
      "dependencies": {}
    },
    {
      "id": "ecommerce",
      "name": "E-Commerce",
      "version": "1.2.0",
      "description": "E-commerce extension",
      "enabled": true,
      "installed_at": "2026-04-16T10:05:00+08:00",
      "has_content_types": true,
      "has_plugin": true,
      "content_types": ["product", "order", "cart_item"],
      "dependencies": {
        "blog-core": ">=1.0.0"
      }
    }
  ]
}
```

### Content Type API 变更

Content Type 的 CRUD API 保持不变（`/api/v1/cms/{plural}`），但新增 `extension_id` 字段标识来源：

```json
{
  "singular": "product",
  "plural": "products",
  "extension_id": "ecommerce",
  "fields": [...]
}
```

### EventBus 新增事件

```rust
pub enum ExtensionEvent {
    Installed { id: String, version: String },
    Enabled { id: String, version: String },
    Disabled { id: String, version: String },
    Uninstalled { id: String },
    Updated { id: String, old_version: String, new_version: String },
}
```

---

## 10. 向后兼容

### plugins/ 目录兼容

启动时，`ExtensionManager` 加载完成后，额外调用 `load_legacy_plugins()`：

```rust
/// 加载 plugins/ 目录下的旧格式插件（不归属任何 Extension）
async fn load_legacy_plugins(config: &AppConfig, pm: &PluginManager) {
    if let Some(plugin_dir) = &config.plugin_dir {
        let dir = Path::new(plugin_dir);
        if dir.exists() {
            tracing::info!("loading legacy plugins from {}", plugin_dir);
            // 复用 PluginManager 现有扫描逻辑，但跳过已被 Extension 加载的
            pm.load_legacy(dir).await;
        }
    }
}
```

PluginManager 需新增 `load_legacy()` 方法，逻辑与 `load_all()` 类似，但跳过已在 `ExtensionManager` 中注册的插件（通过 `manifest.plugin.id` 匹配）。

### content_types/ 目录废弃

- 启动时检查 `content_types/` 目录是否存在且非空
- 如果存在，打印警告日志并自动迁移：将 `content_types/` 下的 TOML 文件包装为 `extensions/_legacy/` Extension
- 迁移是一次性的，迁移后原目录重命名为 `content_types.migrated/`

### 配置项变更

| 现有配置 | 变更 | 说明 |
|---|---|---|
| `CONTENT_TYPE_DIR` | 废弃 | 由 `EXTENSION_DIR` 替代 |
| `PLUGIN_DIR` | 保留 | 仅用于向后兼容旧插件 |
| — | 新增 `EXTENSION_DIR` | Extension 根目录，默认 `./extensions` |
| — | 新增 `EXTENSION_HOT_RELOAD` | Extension 级文件监听，默认 `false` |

```toml
# .env 新增
EXTENSION_DIR=./extensions
EXTENSION_HOT_RELOAD=false
```

---

## 11. 迁移策略

### Phase 1：基础框架（不破坏现有功能）

1. 新增 `src/extension/` 模块（ExtensionManager、ExtensionManifest、model）
2. 新增 `014_extensions.sql` migration
3. PluginManager 新增 `new_empty()` 和 `load_legacy()` 方法
4. AppState 新增 `extension_manager` 字段
5. `server/mod.rs` 新增 ExtensionManager 初始化，但**仍保留原有加载流程**
6. 新增 Admin API（/admin/extensions）

此阶段 Extension 系统并行运行，不影响现有 Content Type 和 Plugin 加载。

### Phase 2：切换加载入口

1. `build_app()` 中将 CT 和 Plugin 加载改为由 ExtensionManager 统一编排
2. `load_content_types()` 改为仅由 ExtensionManager 调用
3. PluginManager 的 `new_with_options()` 内部调用 `new_empty()` + `load_all()`（不破坏外部接口）
4. `content_types/` 目录扫描逻辑迁移到 ExtensionManager

### Phase 3：清理

1. 废弃 `CONTENT_TYPE_DIR` 配置项
2. 移除 `server/mod.rs` 中的 `load_content_types()` 独立函数
3. 将现有 `content_types/*.toml` 迁移到 `extensions/blog-core/content_types/`
4. 前端 Admin UI 新增 Extension 管理页面

---

## 12. 实施计划

### 文件清单

| 文件 | 操作 | 说明 |
|---|---|---|
| `src/extension/mod.rs` | 新增 | 模块注册 |
| `src/extension/manifest.rs` | 新增 | `ExtensionManifest` / `ExtensionInfo` 解析 |
| `src/extension/manager.rs` | 新增 | `ExtensionManager` 核心逻辑 |
| `src/extension/model.rs` | 新增 | DB 查询（installed_extensions CRUD） |
| `src/extension/service.rs` | 新增 | 业务逻辑（安装/卸载/启用/禁用） |
| `src/extension/handler.rs` | 新增 | Admin API handler |
| `migrations/014_extensions.sql` | 新增 | extensions 表 |
| `src/lib.rs` | 修改 | 新增 `pub mod extension` |
| `src/server/mod.rs` | 修改 | 集成 ExtensionManager |
| `src/plugins/mod.rs` | 修改 | 新增 `new_empty()` / `load_legacy()` |
| `src/config/app.rs` | 修改 | 新增 `extension_dir` / `extension_hot_reload` 配置 |
| `web/src/app/admin/extensions/page.tsx` | 新增 | Extension 管理 UI |
| `web/src/app/admin/layout.tsx` | 修改 | 侧边栏新增 Extensions 入口 |

### 工作量估算

| 阶段 | 内容 | 工期 |
|---|---|---|
| Phase 1 | Extension 模块框架 + manifest 解析 + DB 表 + model | 2-3 天 |
| Phase 1 | PluginManager `new_empty()` / `load_legacy()` | 1 天 |
| Phase 1 | ExtensionManager 核心逻辑（发现/加载/拓扑排序） | 2-3 天 |
| Phase 1 | server/mod.rs 集成（并行模式） | 1 天 |
| Phase 2 | Admin API handler + service | 2-3 天 |
| Phase 2 | 切换加载入口（server 改造） | 1-2 天 |
| Phase 2 | 配置项变更 + 迁移逻辑 | 1 天 |
| Phase 3 | 前端 Extension 管理页面 | 2-3 天 |
| Phase 3 | content_types 迁移 + 清理 | 1 天 |
| 测试 | 单元测试 + 集成测试 | 2-3 天 |
| **合计** | | **15-21 天** |

### 依赖关系

```
ExtensionManifest 解析 ──→ ExtensionManager 核心 ──→ server 集成
                              │
                              ├──→ model.rs (DB)
                              ├──→ service.rs
                              └──→ handler.rs (API)

PluginManager new_empty() ──→ ExtensionManager 核心

ExtensionManager ──→ Admin API ──→ 前端 UI
```
