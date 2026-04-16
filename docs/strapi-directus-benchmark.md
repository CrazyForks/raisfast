# Strapi & Directus 特性对标与借鉴分析

> 2026-04-16 · 基于当前系统架构，对比 Strapi v5、Directus 的核心特性，评估借鉴价值与实施路线。

---

## 目录

- [1. 现状总览](#1-现状总览)
- [2. 值得借鉴的特性（按优先级排序）](#2-值得借鉴的特性按优先级排序)
  - [2.1 第一梯队：高价值 + 可行性高](#21-第一梯队高价值--可行性高)
  - [2.2 第二梯队：中等价值](#22-第二梯队中等价值)
  - [2.3 第三梯队：架构级改进](#23-第三梯队架构级改进)
- [3. 优先级总结与实施路线](#3-优先级总结与实施路线)

---

## 1. 现状总览

### 当前系统与 Strapi/Directus 能力矩阵

| 能力 | 当前系统 | Strapi v5 | Directus |
|---|---|---|---|
| 多租户 | ✅ tenant_id 软隔离 | ❌ 单租户 | ❌ 单租户 |
| Auth/RBAC | ✅ JWT + 条件权限 + 通配符 | ✅ RBAC + 条件 | ✅ RBAC + 条件 |
| Content Type Engine | ✅ TOML schema → 自动 CRUD | ✅ Content Type Builder | ✅ Collection Builder |
| 关系类型 | ✅ 6 种 | ✅ 4 种 | ✅ 4 种 |
| 插件系统 | ✅ WASM/JS/Lua 三运行时 | ✅ Node.js 插件 | ✅ Extensions（同进程） |
| EventBus | ✅ broadcast + 3 订阅者 | ✅ Lifecycle hooks | ✅ Event hooks |
| 后台任务 | ✅ SQLite 队列 + Cron | ✅ Cron | ✅ Cron |
| 搜索 | ✅ Tantivy（中文支持） | ⚠️ 需插件 | ⚠️ 需扩展 |
| Webhook | ✅ HMAC-SHA256 | ✅ 内置 | ✅ Flow 内置 |
| 审计日志 | ✅ 持久化 | ✅ 内置 | ✅ Activity Log |
| 多数据库 | ✅ SQLite/PostgreSQL/MySQL | ✅ PostgreSQL/MySQL/SQLite | ✅ PostgreSQL/MySQL/SQLite/Oracle |
| 前端 Admin | ✅ 24 页面 | ✅ 完整 | ✅ Data Studio |
| Prometheus | ✅ Counter + Histogram | ❌ | ❌ |

### 当前系统缺失的核心特性

| 特性 | Strapi | Directus | 当前系统 |
|---|---|---|---|
| API Token 认证 | ✅ Read-only/Full/Custom | ✅ Static/Token | ❌ |
| Content Versioning | ✅ Content History | ⚠️ 通过 Revisions | ❌ |
| Document ID 概念 | ✅ v5 核心概念 | ❌ | ❌ |
| 可视化 Flow 引擎 | ❌ | ✅ Flows（杀手级） | ❌ |
| Content i18n | ✅ 按 CT/字段级别 | ✅ 按 Collection | ❌（仅 UI i18n） |
| Review Workflow | ✅ 多阶段审批 | ⚠️ 通过 Flow | ❌（仅 draft/published） |
| Release 管理 | ✅ 批量定时发布 | ⚠️ 通过 Flow | ❌ |
| Dashboard Panels | ❌ | ✅ Insights 模块 | ❌（仅 3 个统计数字） |
| GraphQL API | ✅ 自动生成 | ✅ 自动生成 | ❌ |
| OpenAPI/Swagger | ✅ 自动生成 | ❌ | ❌ |
| Extension Marketplace | ✅ Marketplace | ✅ Marketplace Beta | ❌ |
| Custom Field UI | ⚠️ Custom Fields | ✅ Interfaces | ❌ |
| SSO (OIDC/SAML) | ✅ Enterprise | ✅ 内置 | ❌ |
| Realtime (WebSocket) | ❌ | ✅ WebSocket + SSE | ⚠️ SSE 单向 |

---

## 2. 值得借鉴的特性（按优先级排序）

### 2.1 第一梯队：高价值 + 可行性高

#### 2.1.1 API Token 认证（参考 Strapi API Tokens）

**Strapi 的做法：**

API Token 是独立于用户账号的认证方式，用于程序化访问 REST/GraphQL API：

- **三种类型：** Read-only（只允许 find/findOne）、Full-access、Custom（按 content type + action 细分权限）
- **可配置过期时间：** 7 天 / 30 天 / 90 天 / 无限期
- **加密存储：** Token 值用 encryption key 加密，可在 Admin 面板查看；无 encryption key 则只显示一次
- **使用方式：** `Authorization: Bearer <api-token>`
- **Salt 隔离：** 更换 salt 会使所有现有 token 失效

**当前系统差距：**

只有 JWT 用户认证（access token + refresh token），无程序化 API 访问凭据。前端/移动端/第三方集成都需要共享用户账号。

**借鉴价值：** SaaS 必备。前端/移动端/CI/CD/第三方集成都需要独立凭据，不应暴露用户密码。

**实施方案：**

```
新增数据表：api_tokens
├── id (UUID v7)
├── tenant_id (TEXT)
├── name (TEXT, NOT NULL)
├── description (TEXT)
├── token_hash (TEXT, NOT NULL) — SHA-256 哈希
├── token_prefix (TEXT) — 前 8 字符用于识别
├── type (TEXT: "read-only" | "full-access" | "custom")
├── permissions (JSON) — custom 类型的细粒度权限
├── expires_at (TEXT, ISO 8601)
├── last_used_at (TEXT)
├── created_by (TEXT)
├── created_at (TEXT)
└── updated_at (TEXT)
```

```
新增中间件：ApiKeyUser
├── 从 Authorization: Bearer <token> 提取 token
├── SHA-256 哈希后查 api_tokens 表
├── 检查过期时间和权限
├── 构造 AuthContext 注入请求扩展
└── 更新 last_used_at
```

**工作量：2-3 天**

---

#### 2.1.2 OpenAPI 自动生成（参考 Strapi OpenAPI Spec）

**Strapi 的做法：**

基于 Content Type 注册自动生成 OpenAPI 3.0 spec，通过 `GET /api/docs` 访问，可接入 Swagger UI。

**当前系统差距：** 无任何 API 文档。前后端协作、第三方集成完全靠口头沟通或阅读代码。

**借鉴价值：** 开发体验和集成效率的倍增器。有文档 vs 无文档的协作效率差 3-5 倍。

**实施方案（两条路线选一）：**

**路线 A：utoipa crate（推荐）**
- 在 handler 函数上添加 `#[utoipa::path]` 注解
- Content Type Engine 根据 schema 动态生成 OpenAPI paths
- 暴露 `GET /api/docs/openapi.json` 端点
- 前端挂载 Swagger UI 或 Redoc

**路线 B：Content Type 动态生成**
- 在 `content_type/schema.rs` 中添加 `to_openapi_schema()` 方法
- 在 `server/mod.rs` 路由注册时收集所有 content type 的路径
- 合并静态 handler 文档 + 动态 content type 文档

**工作量：3-5 天**

---

#### 2.1.3 Content Versioning（参考 Strapi Content History）

**Strapi 的做法：**

每次内容变更保存一个快照，Content Manager 中可浏览所有历史版本，显示每个版本的创建时间、操作者、状态（Draft/Modified/Published），支持一键 Restore 回滚。

**当前系统差距：** Content Type Engine 只有 `updated_at`，更新直接覆盖，无法回溯。

**借鉴价值：** CMS/SaaS 的基础能力，防误操作、审计追溯、团队协作冲突恢复。

**实施方案：**

```
新增数据表：content_versions
├── id (UUID v7)
├── tenant_id (TEXT)
├── content_type (TEXT, NOT NULL) — 如 "product"
├── record_id (TEXT, NOT NULL) — 对应记录的主键
├── version (INTEGER, NOT NULL) — 自增版本号
├── snapshot (JSON, NOT NULL) — 完整字段快照
├── diff (JSON) — 与上一版本的差异（可选优化）
├── created_by (TEXT)
├── created_at (TEXT)
└── UNIQUE(content_type, record_id, version)
```

```
新增 API：
├── GET /admin/cms/{type}/{id}/versions — 版本列表（分页）
├── GET /admin/cms/{type}/{id}/versions/{version} — 版本详情
└── POST /admin/cms/{type}/{id}/versions/{version}/restore — 回滚
```

**关键实现点：**
- Content Type Repository 的 `update()` 方法中，先读取当前值 → 插入版本快照 → 再执行更新
- version 号通过 `SELECT MAX(version) + 1` 生成
- restore 操作本质是一次 update（将 snapshot 数据写入主表）
- 配合 Content Type Schema 的 `versioning: true` 配置项控制是否启用

**工作量：3-5 天**

---

#### 2.1.4 Document ID 概念（参考 Strapi v5 Document Model）

**Strapi v5 的做法：**

引入 `documentId` 作为内容的**逻辑标识符**（不变），而 `id` 是数据库物理主键。同一篇内容在 draft/published/多 locale 下共享一个 `documentId`，不同版本各有不同 `id`：

```
id=1  documentId=abc  status=draft   locale=en
id=2  documentId=abc  status=published locale=en
id=3  documentId=abc  status=draft   locale=zh-CN
```

API 使用 `documentId` 寻址：
- `GET /api/articles/{documentId}` → 返回 published 版本
- `GET /api/articles/{documentId}?status=draft` → 返回草稿
- `GET /api/articles/{documentId}?locale=zh-CN` → 返回中文版

**当前系统差距：** Content Type 使用 UUID v7 作为主键，draft/published 共用一行（通过 `status` 字段），没有文档级别的逻辑 ID。

**借鉴价值：** 这是 Draft/Publish 双版本、i18n 多语言、版本管理的**数据模型基础**。没有 Document ID，以上三项都难以优雅实现。

**实施方案：**

```
Content Type 数据表改造：
├── id (UUID v7) — 物理主键，每行唯一
├── document_id (UUID v7) — 逻辑文档 ID，不变
├── version (INTEGER) — 版本号
├── status (TEXT) — draft / published
├── locale (TEXT) — 语言标识（i18n 时使用）
├── published_at (TEXT)
├── ...业务字段...
└── UNIQUE(document_id, status, locale) — 每个文档每个状态每个语言最多一行
```

```
API 寻址改造：
├── GET /cms/{plural} — 列表默认返回 published
├── GET /cms/{plural}/{document_id} — 按 documentId 返回 published
├── GET /cms/{plural}/{document_id}?status=draft — 返回草稿
├── POST /cms/{plural} — 创建时同时生成 document_id
├── PUT /cms/{plural}/{document_id} — 更新 draft 版本
└── POST /cms/{plural}/{document_id}/publish — 发布（draft → published）
```

**注意事项：**
- 所有关联查询需改为通过 `document_id` 而非 `id`
- 现有数据需迁移（为已有记录生成 document_id = id）
- Content Type Schema 新增 `document_mode: bool` 配置项

**工作量：5-7 天**（涉及 Content Type Repository 核心重构）

---

#### 2.1.5 Visual Flow Engine（参考 Directus Flows）⭐

**Directus 的做法：**

这是 Directus 的**杀手级特性**。Flows 是可视化、事件驱动的自动化引擎：

**Trigger 类型（5 种）：**
- **Event Hook（阻塞/非阻塞）：** 数据变更时触发，阻塞模式可修改或取消事务
- **Webhook：** 接收外部 HTTP 请求触发
- **Schedule (CRON)：** 定时触发
- **Another Flow：** Flow 间调用，支持数组迭代（for-loop）
- **Manual：** Admin UI 按钮手动触发

**Operation 类型（14 种）：**
- Condition（条件分支）
- Run Script（JS/TypeScript 沙箱）
- Create / Read / Update / Delete Data（CRUD 操作）
- Webhook / Request URL（HTTP 请求）
- Send Email（邮件发送）
- Send Notification（站内通知）
- Transform Payload（数据转换）
- Trigger Flow（调用其他 Flow）
- Sleep（延迟）
- Log to Console（调试）
- Throw Error（错误中断）
- JWT（签名/验证）

**Data Chain：** 每个 operation 的输出自动追加到数据链上，后续 operation 可通过 `$trigger`、`$last`、`<operationKey>` 引用。

**当前系统差距：** EventBus + Webhook + Cron 三者割裂，无法组合编排。无法实现"订单创建→扣库存→发邮件→Webhook 通知"这种跨步骤业务流程。

**借鉴价值：** **让平台从"好用"变为"不可替代"。** 电商/CRM/SaaS 的业务规则都可以零代码配置，是企业客户选型时的关键决策因素。

**实施方案：**

```
新增数据表：flows
├── id (UUID v7)
├── tenant_id (TEXT)
├── name (TEXT, NOT NULL)
├── description (TEXT)
├── status (TEXT: "active" | "inactive")
├── icon (TEXT)
├── color (TEXT)
├── track_logs (BOOLEAN) — 是否记录执行日志
├── trigger_type (TEXT: "event_hook" | "webhook" | "schedule" | "manual" | "flow")
├── trigger_config (JSON) — 触发器配置
├── created_at (TEXT)
└── updated_at (TEXT)

新增数据表：flow_operations
├── id (UUID v7)
├── flow_id (TEXT, NOT NULL, FK → flows)
├── position (INTEGER) — 执行顺序
├── type (TEXT: "condition" | "script" | "crud" | "webhook" | "email" | "transform" | "sleep" | "log" | "trigger_flow")
├── name (TEXT) — operation key（Data Chain 引用键名）
├── config (JSON) — 操作配置
├── success_next_id (TEXT, FK → flow_operations) — 成功路径下一操作
├── failure_next_id (TEXT, FK → flow_operations) — 失败路径下一操作
├── created_at (TEXT)
└── updated_at (TEXT)

新增数据表：flow_logs
├── id (UUID v7)
├── flow_id (TEXT)
├── trigger_payload (JSON)
├── accountability (JSON) — 触发者信息
├── status (TEXT: "success" | "failed" | "running")
├── started_at (TEXT)
└── finished_at (TEXT)
```

```
Flow Engine 核心逻辑：
├── FlowTrigger — 监听 EventBus / Cron / Webhook，匹配 trigger_config
├── FlowExecutor — 按 DAG 拓扑序执行 operation 链
│   ├── DataChain — 维护 { $trigger, $accountability, $last, <op_key> } 数据链
│   ├── OperationDispatcher — 根据 type 分派到具体 handler
│   └── ConditionRouter — 评估条件，选择 success/failure 路径
├── ScriptSandbox — JS 执行沙箱（可复用 QuickJS 插件运行时）
└── FlowLogger — 记录执行日志到 flow_logs
```

**分阶段实施：**
1. Phase 1（1 周）：数据模型 + Event Hook Trigger + CRUD/Condition/Log Operation
2. Phase 2（1 周）：Script/Email/Webhook Operation + Admin CRUD API
3. Phase 3（1 周）：Admin UI（流程可视化编辑器）+ Schedule/Manual Trigger

**工作量：2-3 周**

---

### 2.2 第二梯队：中等价值

#### 2.2.1 Content i18n（参考 Strapi Internationalization）

**Strapi 的做法：**

- 按 Content Type 和字段级别启用国际化
- 每个内容可以有多个 locale 版本
- API 通过 `?locale=zh-CN` 查询特定语言
- 支持"从其他语言填充"功能（一键复制再翻译）
- 支持 AI 翻译（Growth 计划）

**当前系统差距：** 只有 UI 层 i18n（`rust_i18n` 翻译错误消息），内容数据本身无多语言支持。

**借鉴价值：** 出海 SaaS 必备。多语言内容管理是国际化项目的核心需求。

**实施方案（依赖 Document ID）：**

```
Content Type Schema 扩展：
├── localized: bool — 是否启用国际化
└── fields[].localized: bool — 字段是否需要翻译

数据表扩展（在 Document ID 基础上）：
├── locale (TEXT, DEFAULT 'zh-CN') — 内容语言
└── UNIQUE(document_id, status, locale)

API 扩展：
├── GET /cms/{plural}?locale=en — 指定语言查询
├── GET /cms/{plural}/{document_id}?locale=zh-CN — 指定语言获取
├── POST /cms/{plural}/{document_id}/locale — 创建新语言版本
└── POST /cms/{plural}/{document_id}/fill-from-locale — 从其他语言填充
```

**工作量：5-7 天**（依赖 Document ID）

---

#### 2.2.2 Review Workflow（参考 Strapi Review Workflows）

**Strapi 的做法：**

多阶段审批流：
- 每个 Content Type 可配置独立工作流
- 工作流由多个阶段组成（如 Draft → Review → Approved → Published）
- 每个阶段可配置"哪些角色可以推进"和"哪些角色可以退回"
- 支持指定审批人（assignee）
- 可复制阶段、调整顺序

**当前系统差距：** 只有简单的 draft/published 状态切换，无审批流程。

**借鉴价值：** 企业级 SaaS 必备。合同审批、内容发布流程、ERP 工单、请假审批等场景都需要。

**实施方案：**

```
新增数据表：workflows
├── id (UUID v7)
├── tenant_id (TEXT)
├── name (TEXT, NOT NULL)
├── content_type (TEXT) — 关联的 Content Type
├── created_at (TEXT)
└── updated_at (TEXT)

新增数据表：workflow_stages
├── id (UUID v7)
├── workflow_id (TEXT, FK → workflows)
├── name (TEXT)
├── position (INTEGER)
├── color (TEXT)
├── can_advance_roles (JSON) — 可推进到此阶段的角色列表
├── can_retreat_roles (JSON) — 可退回的角色列表
├── created_at (TEXT)
└── updated_at (TEXT)

Content Type 数据扩展：
└── workflow_stage (TEXT) — 当前所在阶段
```

**工作量：5-7 天**

---

#### 2.2.3 Release 管理（参考 Strapi Releases）

**Strapi 的做法：**

将多个内容条目（跨 Content Type）打包成一个 Release：
- 支持手动发布或定时发布（可配置时区）
- 条目可以是 publish 或 unpublish 操作
- 自动检测发布就绪状态（Ready / Blocked / Empty / Done）
- 支持批量添加条目到 Release

**当前系统差距：** 只能单条发布，无批量/定时发布能力。

**借鉴价值：** 内容运营效率提升。典型场景：营销活动上线（同时发布 Banner + 文章 + 活动 Page + 产品信息）。

**实施方案：**

```
新增数据表：releases
├── id (UUID v7)
├── tenant_id (TEXT)
├── name (TEXT, NOT NULL)
├── status (TEXT: "empty" | "blocked" | "ready" | "done")
├── scheduled_at (TEXT) — 定时发布时间
├── timezone (TEXT) — 发布时区
├── created_by (TEXT)
├── created_at (TEXT)
└── updated_at (TEXT)

新增数据表：release_items
├── id (UUID v7)
├── release_id (TEXT, FK → releases)
├── content_type (TEXT) — 如 "article"
├── record_id (TEXT) — 文档 ID
├── action (TEXT: "publish" | "unpublish")
├── status (TEXT: "ready" | "not_ready" | "already_done")
└── created_at (TEXT)

Cron 任务：每分钟检查 scheduled_at ≤ now() 的 Release 并执行
```

**工作量：3-5 天**

---

#### 2.2.4 Dashboard Panels / Insights（参考 Directus Insights）

**Directus 的做法：**

Insights 模块提供可视化仪表盘：
- 拖拽配置 Panel（图表、计数器、列表、自定义组件）
- 每个 Panel 可自定义数据查询
- 支持全局过滤器和日期范围
- Dashboard 可保存和分享

**当前系统差距：** Dashboard 只有 3 个静态统计数字（posts/users/comments），无图表。

**借鉴价值：** 所有管理后台都需要数据可视化。CRM 要看销售漏斗、电商要看 GMV 趋势、SaaS 要看用户增长。

**实施方案：**

```
新增数据表：dashboards
├── id (UUID v7)
├── tenant_id (TEXT)
├── name (TEXT)
├── icon (TEXT)
├── note (TEXT)
├── created_by (TEXT)
├── created_at (TEXT)
└── updated_at (TEXT)

新增数据表：panels
├── id (UUID v7)
├── dashboard_id (TEXT, FK → dashboards)
├── name (TEXT)
├── type (TEXT: "metric" | "time_series" | "bar" | "pie" | "table" | "custom")
├── query_config (JSON) — 数据查询配置
├── display_config (JSON) — 显示配置（颜色/轴标签等）
├── position_x (INTEGER)
├── position_y (INTEGER)
├── width (INTEGER)
├── height (INTEGER)
├── created_at (TEXT)
└── updated_at (TEXT)
```

前端集成 Recharts 或 ECharts 渲染图表。

**工作量：1-2 周**

---

### 2.3 第三梯队：架构级改进

#### 2.3.1 GraphQL API（参考 Strapi GraphQL）

**Strapi/Directus 的做法：** 基于 Content Type 自动生成 GraphQL Schema，支持查询/变更/订阅，前端可精确选择所需字段。

**当前系统差距：** 纯 REST API。

**评估：** Rust 生态有 `async-graphql` crate，质量不错。但 REST + `fields` 参数（Content Type 已支持 `select`）基本满足按需查询需求。GraphQL 的主要价值在于关联查询的灵活性（避免 N+1），但当前 Content Type 的 `include` 参数已支持 eager loading。

**建议：** 延后。REST API 已够用，等有明确的客户端需求时再加。

**工作量：2-3 周**

---

#### 2.3.2 Extension Marketplace（参考 Strapi/Directus Marketplace）

**Strapi/Directus 的做法：** 在线插件市场，搜索/安装/更新/卸载插件，类似 npm/vscode 扩展市场。

**当前系统差距：** 插件只能手动下载放置到 `plugin_dir`。

**评估：** 生态建设的中长期目标。需要：插件注册表（registry）API + 插件元数据（manifest）标准化 + 版本管理 + 依赖解析 + Admin UI。当前系统已有完善的插件 manifest 定义和 sandbox 隔离，基础架构是好的。

**建议：** 中长期目标，先做好插件开发者体验（CLI 工具 + 脚手架 + 文档）。

**工作量：1 月+**

---

#### 2.3.3 Custom Field UI（参考 Directus Interfaces）

**Directus 的做法：** 允许扩展字段的编辑器 UI 组件（如颜色选择器、地图、Markdown 编辑器、星级评分等），而不是局限于内置的 input/textarea/select。

**当前系统差距：** Content Type Admin UI 的字段编辑器是固定的（根据 field_type 映射到对应的 shadcn/ui 组件）。

**评估：** 锦上添花。可通过插件系统注册自定义前端组件（利用现有的 `admin_pages` 机制），但需要设计好组件接口协议。

**建议：** 当有实际定制需求时再实现。

**工作量：1-2 周**

---

#### 2.3.4 SSO / Social Login（参考 Strapi SSO）

**Strapi 的做法：** 支持 OIDC、SAML、LDAP 等企业级 SSO。Users & Permissions 插件支持自定义 Provider。

**当前系统差距：** 只有用户名/密码登录。

**评估：** 可通过插件系统实现 OAuth2 Provider（GitHub/Google/微信等），企业级 SSO 可通过 Flow Engine + Webhook 集成。不需要在核心中实现。

**建议：** 做一个 OAuth2 插件示例即可。

**工作量：5-7 天（单个 Provider 插件）**

---

## 3. 优先级总结与实施路线

### 优先级矩阵

| 优先级 | 特性 | 预估工作量 | ROI | 依赖 |
|---|---|---|---|---|
| **P0** | API Token 认证 | 2-3 天 | 极高 | 无 |
| **P0** | OpenAPI 自动生成 | 3-5 天 | 高 | 无 |
| **P1** | Document ID 概念 | 5-7 天 | 高 | 无（但后续特性依赖它） |
| **P1** | Content Versioning | 3-5 天 | 高 | Document ID |
| **P1** | Visual Flow Engine | 2-3 周 | 极高 | 无 |
| **P2** | Content i18n | 5-7 天 | 中 | Document ID |
| **P2** | Review Workflow | 5-7 天 | 中 | 无 |
| **P2** | Release 管理 | 3-5 天 | 中 | Document ID |
| **P2** | Dashboard Panels | 1-2 周 | 中 | 无 |
| **P3** | GraphQL API | 2-3 周 | 低 | 无 |
| **P3** | Extension Marketplace | 1 月+ | 低 | 无 |
| **P3** | Custom Field UI | 1-2 周 | 低 | 无 |
| **P3** | SSO / Social Login | 5-7 天/个 | 中 | 无 |

### 推荐实施路线

```
Phase 1 — 基础补全（2 周）
├── API Token 认证（2-3 天）
├── OpenAPI 自动生成（3-5 天）
└── Document ID 概念（5-7 天）

Phase 2 — 核心差异化（3-4 周）
├── Content Versioning（3-5 天）
├── Visual Flow Engine — Phase 1（1 周）
├── Visual Flow Engine — Phase 2（1 周）
└── Visual Flow Engine — Phase 3 + Admin UI（1 周）

Phase 3 — 功能完善（2-3 周）
├── Content i18n（5-7 天）
├── Review Workflow（5-7 天）
└── Release 管理（3-5 天）

Phase 4 — 体验提升（按需）
├── Dashboard Panels
├── Custom Field UI
├── SSO 插件
└── GraphQL API
```

### 完成后能力对比

Phase 1-3 完成后，系统在功能层面对标：

| 维度 | 对标产品 | 水平 |
|---|---|---|
| Content Type Engine | Strapi v5 / Directus | 同等（+ 多租户） |
| 插件系统 | 超越 | 多运行时（WASM/JS/Lua） |
| 自动化引擎 | Directus Flows | 同等 |
| 多租户 | 超越 | Strapi/Directus 无原生支持 |
| API 文档 | Strapi | 同等 |
| 审计/合规 | Directus | 同等 |
| 性能/安全 | 超越 | Rust + 编译时安全 |
| 生态/插件数 | 落后 | 需要长期建设 |
