# @raisfast/sdk 终极目标与实施路线

> 综合分析 PocketBase JS SDK 优秀设计 + RaisFast 后端实际能力，制定 SDK 发展路线。
> RaisFast 不做 PocketBase 的翻版，而是在通用 CMS SDK 的基础上，暴露自身独有的 AOP、多租户、插件、工作流等能力。

## 一、后端实际 API 全景

RaisFast 后端远比 PocketBase 复杂，分以下几个子系统：

### 1.1 Auth（认证）

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | `/auth/register` | 注册 |
| POST | `/auth/login` | 登录 |
| POST | `/auth/refresh` | Refresh token 换 access token |
| POST | `/auth/logout` | 登出 |
| POST | `/auth/forgot-password` | 发送重置密码邮件 |
| POST | `/auth/reset-password` | Token 重置密码 |
| POST | `/auth/set-password` | OAuth 用户首次设密码 |
| GET | `/auth/config` | 获取认证配置 |
| POST | `/auth/verify-email` | 验证邮箱 |
| POST | `/auth/resend-verification` | 重发验证邮件 |
| POST | `/auth/sms/send` | 发送短信验证码 |
| POST | `/auth/sms/verify` | 验证短信码 |
| POST | `/auth/phone/bind` | 绑定手机号 |

### 1.2 OAuth2

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/auth/oauth/{provider}` | 跳转 OAuth |
| GET | `/auth/oauth/{provider}/callback` | OAuth 回调 |
| GET | `/auth/oauth/providers` | 列出已配置 provider |
| GET | `/auth/oauth/bindings` | 用户已绑定的 OAuth |
| DELETE | `/auth/oauth/{provider}/unbind` | 解绑 |

### 1.3 Users

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/users/me` | 当前用户 |
| PUT | `/users/me` | 更新资料 |
| PUT | `/users/me/password` | 改密码 |
| GET | `/users/{id}` | 公开用户信息 |
| GET | `/users` | 用户列表（admin） |
| PUT | `/users/{id}/role` | 修改角色（admin） |

### 1.4 Content Type 动态路由

每个注册的 Content Type 自动获得：

**公开（collection 类型）**：
- `GET /cms/{plural}` — 列表（rule engine 过滤 + 缓存）
- `POST /cms/{plural}` — 创建（aspect dispatch）
- `GET /cms/{plural}/{id_or_slug}` — 读取
- `PUT /cms/{plural}/{id_or_slug}` — 更新
- `DELETE /cms/{plural}/{id_or_slug}` — 删除（支持 soft-delete）

**公开（singleton 类型）**：
- `GET /cms/{singular}` — 读取
- `PUT /cms/{singular}` — 更新

**Admin**：
- `GET /admin/cms/{plural}` — 管理列表（含未发布/私有）
- `GET /admin/cms/{plural}/{id_or_slug}` — 管理详情
- `GET /admin/cms/{singular}` — 管理 singleton

**版本（versionable 协议）**：
- `GET /admin/cms/{plural}/{id}/revisions` — 版本列表
- `GET /admin/cms/{plural}/{id}/revisions/{rev_id}` — 版本详情
- `POST /admin/cms/{plural}/{id}/revisions/{rev_id}/restore` — 恢复版本
- `GET /admin/cms/{plural}/{id}/revisions/{rev_a}/diff/{rev_b}` — 版本对比

### 1.5 内置 Blog 模块

Categories / Tags / Posts / Comments — 标准 CRUD + 管理端。

### 1.6 内置 Pages 模块

Pages CRUD + sitemap + reusable blocks（可复用区块）。

### 1.7 Media

上传 / 列表 / 统计 / 删除。

### 1.8 Realtime

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/events` | SSE 实时流 |
| GET | `/ws` | WebSocket（可选） |

### 1.9 Admin 子系统

| 模块 | 路径前缀 | 功能 |
|------|---------|------|
| Content Types | `/admin/content-types` | CT schema CRUD |
| Plugins | `/admin/plugins` | 插件管理 + 热重载 |
| Cron Jobs | `/admin/crons` | 定时任务 CRUD + 日志 |
| RBAC | `/admin/rbac` | 角色 + 权限管理 |
| Stats | `/admin/stats` | 统计概览 / 内容统计 / 趋势 |
| Options | `/admin/options` | 系统配置（KV） |
| Tenants | `/admin/tenants` | 多租户管理 |
| Audit Log | `/admin/audit` | 审计日志 |
| Webhooks | `/admin/webhooks` | Webhook CRUD |
| Workflows | `/admin/workflows` | 工作流定义 + 实例 + 步骤执行 |
| API Tokens | `/tokens` | API Token 管理 |

### 1.10 系统端点

| 路径 | 说明 |
|------|------|
| `/health` / `/healthz` / `/readyz` | 健康检查 |
| `/metrics` | Prometheus 指标 |
| `/feed.xml` | RSS 2.0 |
| `/api/v1/routes` | 路由注册表 |
| `/api/docs/openapi.json` | OpenAPI spec |
| `/api/docs` | Swagger UI |

### 1.11 GraphQL（可选）

| 路径 | 说明 |
|------|------|
| `GET /graphql` | GraphiQL IDE |
| `POST /graphql` | GraphQL 查询/变更 |

---

## 二、RaisFast 独有特性（PocketBase 没有）

这些是 RaisFast 的差异化能力，SDK 必须暴露：

### 2.1 AOP Protocol 系统

Content Type 通过 `implements` 声明协议，SDK 需要让开发者感知这些协议的存在：

| 协议 | 自动管理列 | SDK 影响 |
|------|-----------|---------|
| ownable | `created_by`, `updated_by` | 创建/更新时自动填充，用户只读 |
| timestampable | `created_at`, `updated_at` | 完全自动，用户不可设 |
| soft_deletable | `deleted_at`, `deleted_by` | delete 变为软删除，需要 restore 能力 |
| versionable | `version` + revisions 表 | 提供 revisions 浏览/恢复/对比 |
| cacheable | TTL 缓存 | 透明，但需要手动 invalidate 能力 |

**SDK 设计**：

```ts
// collection.ts — 版本管理
async listRevisions(id: string, options?: RequestOptions): Promise<PaginatedData<Revision>>;
async getRevision(id: string, revId: string, options?: RequestOptions): Promise<Revision>;
async restoreRevision(id: string, revId: string, options?: RequestOptions): Promise<T>;
async diffRevisions(id: string, revA: string, revB: string, options?: RequestOptions): Promise<Record<string, unknown>>;

// 软删除恢复
async restore(id: string, options?: RequestOptions): Promise<T>;
```

### 2.2 多租户

通过 `X-Tenant-ID` header 切换租户。后端已有完整的租户管理 API：

```ts
// admin 层
client.admin.listTenants();
client.admin.createTenant({ name: "Acme" });
client.admin.getTenant("id");
client.admin.updateTenant("id", { name: "New" });
client.admin.deleteTenant("id");

// 全局设置
client.setTenantId("acme");   // 切换租户上下文
client.setTenantId(null);      // 重置
```

### 2.3 插件系统

三语言运行时（WASM / JS / Lua）+ 热重载 + 指标。SDK 需暴露：

```ts
client.admin.listPlugins();           // 含健康状态、执行指标
client.admin.getPlugin("my-plugin");
client.admin.enablePlugin("my-plugin");
client.admin.disablePlugin("my-plugin");
client.admin.reloadPlugin("my-plugin");  // 热重载
client.admin.unloadPlugin("my-plugin");
```

### 2.4 工作流引擎

PocketBase 没有工作流。RaisFast 有完整的定义→实例→步骤执行系统：

```ts
// workflow 模块（新增）
client.admin.listWorkflows();
client.admin.createWorkflow(definition);
client.admin.getWorkflow("id");
client.admin.deleteWorkflow("id");
client.admin.startWorkflow("id", payload?);
client.admin.listInstances();
client.admin.getInstance("id");
client.admin.executeStep("instanceId", { step: 1, action: "approve", data: {} });
client.admin.cancelInstance("instanceId");
client.admin.getStepLogs("instanceId");
```

### 2.5 RBAC

```ts
client.admin.listRoles();
client.admin.createRole({ name: "editor", permissions: [...] });
client.admin.updateRole("id", { permissions: [...] });
client.admin.deleteRole("id");
client.admin.getPermissions("roleId");
client.admin.setPermissions("roleId", [...]);
```

### 2.6 系统配置（Options）

KV 键值对配置系统：

```ts
client.admin.listOptions();
client.admin.getOption("key");
client.admin.setOption("key", "value");
client.admin.deleteOption("key");
client.admin.batchUpdateOptions({ key1: "v1", key2: "v2" });

// 公开配置（无需认证）
client.getPublicOptions();
```

### 2.7 Webhook 系统

```ts
client.admin.listWebhooks();
client.admin.createWebhook({ url: "...", events: ["PostCreated"] });
client.admin.getWebhook("id");
client.admin.updateWebhook("id", { events: ["*"] });
client.admin.deleteWebhook("id");
```

### 2.8 审计日志

```ts
client.admin.listAuditLogs({ page: 1, page_size: 50 });
client.admin.getAuditLog("id");
```

### 2.9 定时任务

```ts
client.admin.listCrons();
client.admin.createCron({ name: "...", schedule: "0 * * * *", handler: "..." });
client.admin.getCron("id");
client.admin.updateCron("id", { schedule: "*/5 * * * *" });
client.admin.deleteCron("id");
client.admin.toggleCron("id");
client.admin.listCronLogs();
client.admin.cleanupCronLogs();
```

### 2.10 API Token

```ts
client.admin.listTokens();
client.admin.createToken({ name: "CI", expires_at: "..." });
client.admin.deleteToken("id");
```

### 2.11 GraphQL

```ts
// 直接用 send() 即可
const data = await client.send('/graphql', {
  method: 'POST',
  body: { query: '{ posts { items { id title } } }' },
});
```

### 2.12 Tauri 桌面模式

SDK 未来可以适配 Tauri IPC（不经过 HTTP），共享相同的服务层接口。

---

## 三、SDK 当前覆盖情况

### ✅ 已实现

| 模块 | 方法 | 对应后端 |
|------|------|---------|
| **Client** | `RaisFast()` 构造 + authStore + beforeSend/afterSend | — |
| **Auth** | login, register, refresh, logout, getMe, updateMe, changePassword | ✅ 全部对应 |
| **Auth** | requestPasswordReset, confirmPasswordReset | ✅ forgot-password / reset-password |
| **Auth** | listOAuth2Providers, authWithOAuth2 | ✅ oauth endpoints |
| **Collection** | getList, getFullList, getOne, getFirstListItem, create, update, delete | ✅ cms 动态路由 |
| **Admin Collection** | adminCollection + same CRUD | ✅ admin/cms 路由 |
| **Admin** | stats, statsContent, statsTrends | ✅ |
| **Admin** | listPlugins, getPlugin, enable/disable/reload/unloadPlugin | ✅ |
| **Admin** | list/create/get/update/deleteContentType | ✅ content-types |
| **Admin** | listRoutes | ✅ routes |
| **RaisFast** | single() helper | ✅ |

### ❌ 未实现（后端已就绪）

| 模块 | 缺失方法 | 对应后端 | 优先级 |
|------|---------|---------|--------|
| **Auth** | authConfig | `GET /auth/config` | P1 |
| **Auth** | verifyEmail, resendVerification | `POST /auth/verify-email`, `/resend-verification` | P1 |
| **Auth** | sendSmsCode, verifySms, bindPhone | `/auth/sms/*`, `/auth/phone/*` | P2 |
| **Auth** | setPassword | `POST /auth/set-password` | P2 |
| **Auth** | OAuth redirect/bindings/unbind | `/auth/oauth/*` | P1 |
| **Users** | getUser, listUsers, updateUserRole | `/users/*` | P1 |
| **Collection** | listRevisions, getRevision, restoreRevision, diffRevisions | `/admin/cms/{plural}/{id}/revisions/*` | P1 |
| **Collection** | restore（软删除恢复） | 后端待确认 | P2 |
| **Realtime** | SSE subscribe/unsubscribe | `GET /events` | **P0** |
| **Admin Tenants** | list/create/get/update/deleteTenant | `/admin/tenants` | P1 |
| **Admin RBAC** | list/create/update/deleteRole, get/setPermissions | `/admin/rbac/*` | P1 |
| **Admin Options** | list/get/set/deleteOption, batchUpdate, getPublicOptions | `/admin/options/*`, `/options/public` | P1 |
| **Admin Webhooks** | list/create/get/update/deleteWebhook | `/admin/webhooks` | P2 |
| **Admin Audit** | listAuditLogs, getAuditLog | `/admin/audit` | P2 |
| **Admin Crons** | list/create/get/update/delete/toggleCron, listCronLogs, cleanupCronLogs | `/admin/crons/*` | P2 |
| **Admin Tokens** | list/create/deleteToken | `/tokens` | P2 |
| **Admin Workflows** | 完整工作流 CRUD + 执行 | `/admin/workflows/*` | P2 |
| **Media** | upload, list, stats, delete | `/media/*` | P1 |
| **Blog** | categories, tags, posts, comments CRUD | `/categories`, `/tags`, `/posts`, `/comments` | P2 |
| **Pages** | pages CRUD + sitemap + reusable blocks | `/pages`, `/admin/pages/*` | P2 |
| **Health** | health, liveness, readiness | `/health`, `/healthz`, `/readyz` | P1 |
| **Client** | cancelRequest, cancelAllRequests | 纯前端 | P1 |
| **Client** | GraphQL send helper | `POST /graphql` | P3 |

### ❌ 未实现（后端也待开发）

| 功能 | 说明 | 优先级 |
|------|------|--------|
| expand 关联展开 | `?expand=author` 自动填充关联记录 | P2 |
| upsert | 存在则更新，不存在则创建 | P3 |
| batch API | 单事务批量操作 | P3 |
| 文件 URL 构建 + 缩略图 | `getFileURL(record, field, { thumb })` | P2 |

---

## 四、SDK 模块设计目标

### 4.1 模块结构

```
raisfast.ts          — 入口：RaisFast 类，组装所有子模块
client.ts            — HTTP 层：request/hooks/refresh/cancel
auth.ts              — AuthStore：BaseAuthStore + LocalAuthStore
auth-api.ts          — Auth API：login/register/oauth/sms/email/phone
collection.ts        — Collection：CRUD + revisions + subscribe
realtime.ts          — Realtime：SSE 客户端 + 自动重连
media.ts             — Media：上传/列表/统计
admin.ts             — Admin：拆分为子命名空间
  ├─ admin/plugins.ts
  ├─ admin/tenants.ts
  ├─ admin/rbac.ts
  ├─ admin/options.ts
  ├─ admin/webhooks.ts
  ├─ admin/audit.ts
  ├─ admin/crons.ts
  ├─ admin/workflows.ts
  └─ admin/tokens.ts
errors.ts            — SDKError
types.ts             — 所有类型定义
```

### 4.2 Admin 命名空间设计

Admin 功能太多，不宜平铺在一个类里。采用子命名空间：

```ts
const client = new RaisFast(baseUrl);

// 插件
client.admin.plugins.list();
client.admin.plugins.enable("my-plugin");

// 租户
client.admin.tenants.list();
client.admin.tenants.create({ name: "Acme" });

// RBAC
client.admin.rbac.listRoles();
client.admin.rbac.setPermissions("roleId", [...]);

// 配置
client.admin.options.get("site_name");
client.admin.options.set("site_name", "My Blog");
client.admin.options.batchUpdate({ ... });

// Webhooks
client.admin.webhooks.list();
client.admin.webhooks.create({ url: "...", events: ["*"] });

// 审计
client.admin.audit.list({ page: 1 });

// 定时任务
client.admin.crons.list();
client.admin.crons.toggle("id");

// 工作流
client.admin.workflows.list();
client.admin.workflows.start("id");
client.admin.workflows.executeStep("instanceId", { ... });

// API Token
client.admin.tokens.list();
client.admin.tokens.create({ name: "CI" });
```

### 4.3 Realtime 设计

后端已有 `GET /events` SSE endpoint 和 `GET /ws` WebSocket。SDK 需要同时支持：

```ts
const client = new RaisFast(baseUrl);

// SSE 模式（默认）
const unsub = await client.collection("posts").subscribe("*", (e) => {
  console.log(e.action);  // "create" | "update" | "delete"
  console.log(e.record);  // T 类型记录
});

// 订阅单条
const unsub2 = await client.collection("posts").subscribe("id123", (e) => { ... });

// 取消
unsub();
await client.collection("posts").unsubscribe();

// 全局 realtime 访问
client.realtime.isConnected;
client.realtime.connectionId;
```

### 4.4 请求取消设计

```ts
const client = new RaisFast(baseUrl);

// 自动取消相同 key 的上一个请求
const list = await client.collection("posts").getList(1, 25, {
  requestKey: "posts-list",
});

// 手动取消
client.cancelRequest("posts-list");
client.cancelAllRequests();
```

---

## 五、实施路线

### Phase 1：核心补齐（1-2 周）

后端已就绪，SDK 纯前端工作：

| 任务 | 工时 | 说明 |
|------|------|------|
| cancelRequest / cancelAllRequests | 4h | 纯前端 |
| Health check | 0.5h | `client.health.check()` |
| Auth 补齐：authConfig / verifyEmail / resendVerification / setPassword | 2h | 后端已实现 |
| OAuth 补齐：redirect URL 构建 / bindings / unbind | 1h | 后端已实现 |
| Users 补齐：getUser / listUsers / updateUserRole | 1h | 后端已实现 |
| Media 模块：upload / list / stats / delete | 2h | 后端已实现 |
| 文件 URL 构建 `getFileURL()` | 1h | 纯前端 URL 拼接 |
| Collection 版本管理：listRevisions / getRevision / restoreRevision / diffRevisions | 3h | 后端已实现 |

### Phase 2：Realtime + Admin 重构（2-3 周）

| 任务 | 工时 | 说明 |
|------|------|------|
| Realtime 模块（SSE 客户端） | 1d | 后端 `/events` 已实现 |
| Collection.subscribe / unsubscribe | 1d | 基于 Realtime 模块 |
| 自动重连 + 心跳 | 0.5d | — |
| Admin 命名空间拆分 | 2d | plugins/tenants/rbac/options/webhooks/audit/crons/tokens |
| Admin Workflows 模块 | 1d | 后端已实现 |
| 集成测试 | 1d | — |

### Phase 3：SMS / Auth 增强（1 周）

| 任务 | 工时 | 说明 |
|------|------|------|
| SMS 认证：sendSmsCode / verifySms / bindPhone | 1h | 后端已实现 |
| Blog 模块 SDK（categories / tags / posts / comments） | 1d | 后端已实现 |
| Pages 模块 SDK（pages + reusable blocks） | 1d | 后端已实现 |
| Auth onChange 优化（token 过期前自动刷新） | 0.5d | — |

### Phase 4：高级特性（按需）

| 任务 | 工时 | 说明 |
|------|------|------|
| expand 关联展开 | 2d | 需后端配合 |
| WebSocket realtime 备选 | 1d | 后端已实现 |
| upsert | 2h | 需后端支持 |
| Batch API | 1d | 需后端支持 |
| GraphQL helper | 0.5h | 纯前端封装 |
| Tauri IPC adapter | 2d | 桌面模式不走 HTTP |

---

## 六、SDK 质量目标

| 指标 | 目标 |
|------|------|
| Bundle size (gzip) | < 10KB（realtime 模块可 tree-shake） |
| 零运行时依赖 | ✅ |
| ESM + CJS + types | ✅（tsup 构建） |
| TypeScript 严格模式 | ✅ |
| 单元测试 | Phase 1 完成时 30+，最终 60+ |
| 运行环境 | 浏览器 + Node.js（Deno / Bun 待验证） |
| React Native | 目标支持（AuthStore 可定制） |
| 最低浏览器要求 | ES2020+（不需要 IE11） |

---

## 七、设计原则

1. **后端已实现 → SDK 必须暴露**：不遗漏任何已有 API
2. **后端未实现 → SDK 先设计接口**：types 定义先行，方法体 throw "Not implemented"
3. **PocketBase 好的设计要学**：requestKey、subscribe、expand、auto refresh
4. **RaisFast 特色要突出**：AOP 协议感知、多租户、工作流、RBAC、GraphQL
5. **Admin 按领域拆命名空间**：不做一个 1000 行的 God class
6. **Tree-shakable**：未使用的模块不进 bundle
