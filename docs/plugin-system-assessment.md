# 插件系统评估

> 基于 WASM/JS/Lua 三引擎插件系统完成后的全面评估，分析当前水平与差距，确定改进方向。

---

## 1. 当前水平

**中等偏上** — 对博客系统而言优秀，对平台框架而言不够。

---

## 2. 已有的亮点

| 能力 | 说明 |
|------|------|
| **三引擎沙箱** | WASM (wasmtime) / JavaScript (rquickjs) / Lua (mlua)，比大多数开源博客项目丰富 |
| **Hook 四类分派** | Filter（链式）/ Action（顺序）/ RenderOverride（首覆写）/ Route（自定义路由） |
| **热重载** | notify 文件监听 + mpsc channel + 1 秒防抖，支持 `.wasm` / `.js` / `.lua` |
| **优先级调度** | manifest 声明 priority，按数值排序执行 |
| **安全沙箱** | WASM fuel/memory 限制；JS interrupt handler 超时；Lua 禁用 io/os/debug 库 + 指令计数超时 |
| **Host API 跨引擎统一** | `Host.log()` + `Host.getConfig()` 在三引擎中行为一致 |
| **测试覆盖** | 117 单元测试 + 60 集成测试，含三引擎交叉测试 |
| **Feature flag 隔离** | `plugin-wasm` / `plugin-js` / `plugin-lua` / `plugin-all`，按需编译 |

---

## 3. 缺什么才算「先进」

### 3.1 P0 — 不补就做不了有用的插件

#### Host API 严重不足

当前仅 `log` + `getConfig` 两个 Host 函数。插件无法与外部世界交互，实际能做的事极其有限。

**需要补齐：**

| Host 函数 | 说明 | 引擎 |
|-----------|------|------|
| `host_http_get(url)` | HTTP GET 请求（受域名白名单限制） | 三引擎 |
| `host_http_post(url, body)` | HTTP POST 请求（受域名白名单限制） | 三引擎 |
| `host_get_post(slug)` | 获取文章内容（只读） | 三引擎 |
| `host_db_query(sql, params)` | 数据库只读查询（受表权限控制） | 三引擎 |
| `host_kv_get(key)` | 插件 KV 存储 — 读取 | 三引擎 |
| `host_kv_set(key, value, ttl?)` | 插件 KV 存储 — 写入 | 三引擎 |

**实现要点：**

- HTTP 请求需校验 manifest 中 `permissions.http` 白名单
- DB 查询需校验 `permissions.db-read` 表白名单，且强制只读（SELECT only）
- KV store 可用 SQLite `plugin_storage` 表（`plugin_id TEXT, key TEXT, value TEXT, expires_at TEXT`）

#### 权限声明未强制执行

manifest 中声明了 `permissions`（http、config、db-read、db-write），但运行时没有任何检查逻辑。

**修复方案：**

```rust
// src/plugins/permissions.rs
impl Permissions {
    pub fn is_url_allowed(&self, url: &str) -> bool {
        // 解析 URL domain，检查是否匹配 permissions.http 白名单
    }

    pub fn is_config_key_allowed(&self, key: &str) -> bool {
        // 检查 key 是否匹配 permissions.config 前缀
    }

    pub fn is_table_readable(&self, table: &str) -> bool {
        self.db_read.iter().any(|t| t == table)
    }
}
```

每个 Host 函数实现中必须在执行前检查权限，拒绝则返回错误。

---

### 3.2 P1 — 不补就做不好

#### 插件管理 API

运行时无法通过 API 查看插件状态、启用/禁用插件。

**需要：**

```
GET    /api/v1/admin/plugins          — 列出所有插件及状态
GET    /api/v1/admin/plugins/:id      — 插件详情
POST   /api/v1/admin/plugins/:id/enable   — 启用
POST   /api/v1/admin/plugins/:id/disable  — 禁用
POST   /api/v1/admin/plugins/:id/reload   — 重载
DELETE /api/v1/admin/plugins/:id      — 卸载（从内存+磁盘移除）
```

#### 生命周期 Hook

插件无法在加载/卸载时执行初始化/清理逻辑。

**需要：**

| Hook | 触发时机 | 用途 |
|------|---------|------|
| `on_load` | 插件加载完成 | 初始化连接、注册定时任务、预加载数据 |
| `on_unload` | 插件卸载前 | 清理资源、关闭连接、保存状态 |
| `on_config_change` | 宿主配置变更时 | 热更新插件行为 |

#### 插件持久状态

插件没有地方存储自己的数据。例如 SEO 插件想缓存分析结果、通知插件想记录已发送的邮件。

**方案 A — SQLite plugin_storage 表：**

```sql
CREATE TABLE IF NOT EXISTS plugin_storage (
    plugin_id TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    expires_at TEXT,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (plugin_id, key)
);
```

**方案 B — KV Host API：**

```lua
-- Lua 插件中
Host.setData("last_notified", "2026-04-14T00:00:00Z")
local last = Host.getData("last_notified")
```

---

### 3.3 P2 — 做大时才需要

#### 错误恢复

当前插件抛异常后静默跳过，无法感知插件健康状态。

**需要：**

- 每个插件维护 `error_count` 和 `last_error`
- 连续 N 次错误（如 5 次）后自动禁用该插件
- 记录错误日志 + 可选告警通知
- 管理 API 中展示插件健康状态

```rust
struct PluginHealth {
    error_count: u32,
    last_error: Option<String>,
    last_error_at: Option<String>,
    auto_disabled: bool,
}
```

#### EventBus（事件总线）

当前 Hook 是同步链式调用，无法解耦。平台级系统需要事件驱动架构。

**差距：**

- 插件无法订阅内部事件（如"用户注册"不在 Hook 中则无法感知）
- 一个 Hook 的处理阻塞后续所有插件
- 无法异步响应（如"文章创建后异步触发邮件发送"）

**方案：**

```rust
// src/eventbus/mod.rs
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
}

// 订阅者
EventBus.emit(PostCreated)
    ├── PluginManager.dispatch_action(PostCreated)
    ├── NotificationService.notify(author)
    ├── SearchIndexer.index(post)
    └── AuditLogger.log("post.create", ...)
```

#### 插件间通信

当前插件之间完全隔离，无法互相发现或调用。

**可能的方案：**

- 注册表模式：插件 A 在宿主注册一个 named service，插件 B 通过 `Host.getService("name")` 调用
- 需要仔细设计以避免循环依赖和安全问题

#### 性能指标

缺少插件执行的可观测性。

**需要：**

| 指标 | 说明 |
|------|------|
| Hook 执行耗时 | 每个 Hook × 每个插件的 P50/P95/P99 |
| 内存占用 | 每个插件的实际内存使用 |
| 调用次数 | 每个 Hook 的调用频率 |
| 错误率 | 每个插件的错误次数/频率 |

可集成 Prometheus metrics 或通过管理 API 暴露。

#### 依赖管理

无插件版本冲突检测、无依赖声明。

**manifest 扩展：**

```toml
[plugin]
id = "com.example.seo-optimizer"
version = "1.0.0"

[dependencies]
"com.example.utils" = ">=1.0.0"
```

加载时检测依赖是否满足，缺失则拒绝加载。

---

## 4. 与成熟系统对比

| 能力 | WordPress | Ghost | **raisfast** |
|------|-----------|-------|---------------|
| 沙箱隔离 | 无（PHP 直接执行） | 无（Node 直接 require） | **有（三引擎）** |
| 热重载 | 需插件自身支持 | 需重启 | **有** |
| Hook 数量 | ~2000 个 filter/action | ~30 个事件 | 11 个 Hook |
| Host API | 完整（DB/HTTP/FS/Option） | 完整 | 2 个函数 |
| 权限控制 | Role + Capability | 细粒度 | 声明式但未执行 |
| 插件状态存储 | wp_options 表 | DB 表 | 无 |
| 插件市场 | 有 | 有 | 无 |
| 多语言插件 | PHP only | JS only | **Rust/JS/Lua/WASM** |

**raisfast 的独特优势**是三引擎沙箱隔离，这在博客系统中非常少见。但 Host API 和生态成熟度差距明显。

---

## 5. 改进路线图

### Phase 1 — 补齐基础能力（1 周）

```
Week 1:
  ├── 权限执行模块（permissions.rs）
  ├── Host API: http_get/http_post（域名白名单）
  ├── Host API: get_post（文章只读访问）
  ├── Host API: KV store（plugin_storage 表 + get_data/set_data）
  ├── 插件管理 API（列表/详情/启用/禁用/重载）
  └── 集成测试覆盖新 Host API + 权限检查
```

### Phase 2 — 生命周期与健壮性（1 周）

```
Week 2:
  ├── 生命周期 Hook（on_load/on_unload/on_config_change）
  ├── 错误恢复（错误计数 + 自动禁用 + 健康状态）
  ├── 插件管理 API 增强（健康状态、错误日志）
  └── Host API: db_query（只读查询 + 表权限校验）
```

### Phase 3 — 平台化能力（按需）

```
  ├── EventBus 事件总线
  ├── 插件间通信机制
  ├── 性能指标采集（Prometheus 或内部 metrics）
  ├── 依赖管理与版本冲突检测
  └── 插件脚手架 CLI（cargo run -- generate-plugin）
```

---

## 6. 依赖新增预估

| 阶段 | 依赖 | 用途 | 体积影响 |
|------|------|------|---------|
| Phase 1 | `reqwest`（可能已有） | Host HTTP 请求 | 零增长（已有） |
| Phase 1 | 无 | KV store 用 SQLite | 零增长 |
| Phase 2 | 无 | 错误恢复纯内存 | 零增长 |
| Phase 3 | `prometheus`（可选） | 指标采集 | +~1MB |

Phase 1-2 几乎不增加外部依赖。

---

## 7. 总结

```
当前状态:
  ✅ 三引擎沙箱 — 业界领先
  ✅ 热重载 — 生产可用
  ✅ Hook 四类分派 — 架构合理
  ✅ 安全模型 — 沙箱限制到位
  ✅ 测试覆盖 — 质量有保障

  ❌ Host API — 仅 2 个函数，插件做不了实事
  ❌ 权限执行 — 声明式但未运行时检查
  ❌ 管理接口 — 无法运行时管理插件
  ❌ 生命周期 — 无初始化/清理回调
  ❌ 持久状态 — 插件无法存数据
```

**核心判断：** 架构骨架扎实，三引擎沙箱是独特优势。但当前能扩展的点太少（11 Hook + 2 Host API），实际可编写的插件非常有限。补齐 Host API、权限执行、管理接口后，才算真正可用的插件平台。
