# Admin API 规格补全计划

> 现状：部分模块的写操作（create/update/delete）混在 public `/api/v1/xxx` 路由中，
> admin 只有 list/get，缺少完整的 CRUD 和批量操作。
> 目标：admin 拥有独立、完整的 CRUD + 批量操作路由。

---

## 路由分层原则

| 层 | 路径前缀 | 说明 |
|---|---|---|
| **Public** | `/api/v1/xxx` | 面向前端读者，读为主，写需 auth（普通用户可发评论、管理自己的内容） |
| **Admin** | `/api/v1/admin/xxx` | 面向管理后台，全部 CRUD，需 admin/author 权限 |

Admin 路由与 Public 路由**可能共用底层 service 函数**，但 handler 层独立，允许：
- Admin 绕过某些校验（如草稿直接发布）
- Admin 修改他人内容
- Admin 操作不触发某些副作用（如通知）
- Admin 有额外的审计日志

---

## 批量操作统一格式

所有模块的批量操作统一使用 `POST /admin/xxx/batch`，请求体：

```json
{
  "action": "delete",
  "ids": ["uuid1", "uuid2", "uuid3"]
}
```

各模块支持的 action：

| 模块 | 支持的 action |
|---|---|
| posts | `delete`, `publish`, `unpublish` |
| comments | `delete`, `approve`, `reject`, `spam` |
| media | `delete` |
| users | `disable`, `enable`, `change_role`（需额外 `role` 字段） |
| tags | `delete` |
| categories | `delete` |
| pages | `delete`, `publish`, `unpublish` |

`change_role` 批量操作扩展格式：
```json
{
  "action": "change_role",
  "ids": ["uuid1", "uuid2"],
  "role": "author"
}
```

---

## 模块级改动清单

### 1. posts（改动最大）

**现有 public 路由（保留）：**
- `GET /posts` — 文章列表（公开，支持分页/搜索/过滤）
- `GET /posts/{slug}` — 文章详情（公开）
- `POST /posts` — 创建文章（auth，普通用户投稿）
- `PUT /posts/{slug}` — 编辑文章（auth，只能改自己的）
- `DELETE /posts/{slug}` — 删除文章（auth，只能删自己的）

**需要新增 admin 路由：**
- `POST /admin/posts` — 创建文章（admin，可指定作者、强制发布）
- `PUT /admin/posts/{id}` — 编辑任意文章（admin，可改作者/状态）
- `DELETE /admin/posts/{id}` — 删除任意文章（admin）
- `POST /admin/posts/batch` — 批量操作（delete / publish / unpublish）

**已有 admin 路由（保留）：**
- `GET /admin/posts` — admin 列表（含草稿、所有作者）
- `GET /admin/posts/{slug}` — admin 详情

**SDK 变更：**
- `client.posts.create()` → `POST /admin/posts`（admin UI）
- `client.posts.update(slug)` → `PUT /admin/posts/{id}`（admin UI）
- `client.posts.delete(slug)` → `DELETE /admin/posts/{id}`（admin UI）
- 新增 `client.posts.batch(action, ids)` → `POST /admin/posts/batch`

---

### 2. categories

**现有 public 路由（保留）：**
- `GET /categories` — 分类列表
- `POST /categories` — 创建（auth，author+）
- `PUT /categories/{id}` — 更新（auth，author+）
- `DELETE /categories/{id}` — 删除（auth，author+）

**需要新增 admin 路由：**
- `POST /admin/categories` — 创建
- `PUT /admin/categories/{id}` — 更新
- `DELETE /admin/categories/{id}` — 删除
- `POST /admin/categories/batch` — 批量删除

**已有 admin 路由：** 无（全在 public 下）

---

### 3. tags

**现有 public 路由（保留）：**
- `GET /tags` — 标签列表
- `POST /tags` — 创建（auth，author+）
- `PUT /tags/{id}` — 更新（auth，author+）
- `DELETE /tags/{id}` — 删除（auth，author+）

**需要新增 admin 路由：**
- `POST /admin/tags` — 创建
- `PUT /admin/tags/{id}` — 更新
- `DELETE /admin/tags/{id}` — 删除
- `POST /admin/tags/batch` — 批量删除

---

### 4. media

**现有 public 路由（保留）：**
- `POST /media/upload` — 上传（auth）
- `GET /media` — 列表
- `GET /media/stats` — 统计
- `DELETE /media/{id}` — 删除（auth，自己的）

**需要新增 admin 路由：**
- `POST /admin/media/upload` — 上传（admin）
- `GET /admin/media` — admin 列表（含所有用户的媒体）
- `DELETE /admin/media/{id}` — 删除任意媒体（admin）
- `POST /admin/media/batch` — 批量删除

---

### 5. comments

**现有 public 路由（保留）：**
- `GET /posts/{slug}/comments` — 评论列表
- `POST /posts/{slug}/comments` — 游客评论
- `POST /posts/{slug}/comments/authed` — 登录用户评论
- `GET /comments` — 全部评论列表
- `DELETE /comments/{id}` — 删除（auth，自己的或 admin）
- `PUT /comments/{id}/status` — 更新状态（auth，author+）

**需要新增 admin 路由：**
- `GET /admin/comments` — admin 评论列表（支持按状态/文章/作者过滤）
- `PUT /admin/comments/{id}/status` — 审核评论（approve / reject / spam）
- `DELETE /admin/comments/{id}` — 删除任意评论
- `POST /admin/comments/batch` — 批量操作（delete / approve / reject / spam）

---

### 6. users

**现有 public 路由（保留）：**
- `GET /users/me` — 当前用户信息
- `PUT /users/me` — 更新自己信息
- `PUT /users/me/password` — 改密码
- `GET /users` — 用户列表
- `GET /users/{id}` — 用户详情
- `PUT /users/{id}/role` — 改角色（admin）

**需要新增 admin 路由：**
- `GET /admin/users` — admin 用户列表（含禁用用户、更多字段）
- `GET /admin/users/{id}` — admin 用户详情
- `POST /admin/users` — 创建用户（admin 指定角色、跳过注册流程）
- `PUT /admin/users/{id}` — 编辑任意用户（改角色/状态/信息）
- `DELETE /admin/users/{id}` — 禁用/删除用户
- `POST /admin/users/batch` — 批量操作（disable / enable / change_role）

---

### 7. pages（已有完整 admin CRUD，只需加批量）

**已有 admin 路由：**
- `GET /admin/pages`, `GET /admin/pages/{id}`, `PUT /admin/pages/{id}`, `DELETE /admin/pages/{id}`
- `PUT /admin/pages/{id}/status`, `PUT /admin/pages/reorder`

**需要新增：**
- `POST /admin/pages` — 创建页面（当前创建走 `POST /pages`）
- `POST /admin/pages/batch` — 批量操作（delete / publish / unpublish）

---

### 8. reusable_blocks（已有完整 admin CRUD，只需加批量）

**需要新增：**
- `POST /admin/reusable-blocks/batch` — 批量删除

---

## 不需要改动的模块

以下模块已有完整的 admin CRUD，无需调整：

| 模块 | 路由 | 状态 |
|---|---|---|
| cron | `/admin/crons` 全套 | ✅ 完整（需加 batch） |
| plugin | `/admin/plugins` 全套 | ✅ 完整（需加 batch） |
| rbac | `/admin/rbac/roles` 全套 | ✅ 完整（需加 batch） |
| stats | `/admin/stats` | ✅ 只读，无需 CRUD |
| options | `/admin/options` 全套 | ✅ 完整 |
| tenant | `/admin/tenants` 全套 | ✅ 完整（需加 batch） |
| audit | `/admin/audit` | ✅ 只读 |
| webhook | `/admin/webhooks` 全套 | ✅ 完整（需加 batch） |
| workflow | `/admin/workflows` 全套 | ✅ 完整 |
| reusable_block | `/admin/reusable-blocks` 全套 | ✅ 完整（需加 batch） |

---

## 9. content_type / CMS 动态内容

Content Type 模块比其他模块复杂，有**两层路由**需要补全。

### 9.1 Schema 管理（`/admin/content-types`）

**现有 admin 路由（完整 CRUD）：**
- `GET /admin/content-types` — 列出所有 schema
- `POST /admin/content-types` — 创建 schema
- `GET /admin/content-types/{singular}` — 获取 schema
- `PUT /admin/content-types/{singular}` — 更新 schema
- `DELETE /admin/content-types/{singular}` — 删除 schema

**需要新增：**
- `POST /admin/content-types/batch` — 批量操作（delete / export / duplicate）
- `POST /admin/content-types/{singular}/fields` — 给 schema 增加字段（避免每次 PUT 整个 schema）
- `DELETE /admin/content-types/{singular}/fields/{field_name}` — 删除单个字段

### 9.2 CMS 动态内容（`/cms/{*path}` 和 `/admin/cms/{*path}`）

**现有路由：**
- `ANY /cms/{*path}` — public catch-all（读 + 写混在一起）
- `ANY /admin/cms/{*path}` — admin catch-all

**问题：**
1. Public `/cms/xxx` 没有读写分离——游客/普通用户理论上能触发写操作
2. 没有独立的 admin 内容 CRUD 路由（只能通过 catch-all）
3. 没有批量操作
4. 每个 content type 的路由是动态生成的，没有出现在路由注册表中

**需要新增（每个 content type 动态注册）：**

| 路由 | 方法 | 说明 |
|---|---|---|
| `/admin/cms/{plural}/batch` | POST | 批量操作（delete / publish / unpublish / archive） |
| `/admin/cms/{plural}/export` | GET | 导出为 JSON/CSV |
| `/admin/cms/{plural}/import` | POST | 从 JSON 导入 |

**Public vs Admin CMS 路由分离规则：**

| 操作 | Public `/cms/{plural}` | Admin `/admin/cms/{plural}` |
|---|---|---|
| 列表 | `GET`（仅 published） | `GET`（含 draft/archived） |
| 详情 | `GET /{id}`（仅 published） | `GET /{id}`（任意状态） |
| 创建 | `POST`（auth，draft） | `POST`（admin，可指定状态/作者） |
| 更新 | `PUT /{id}`（auth，自己的） | `PUT /{id}`（任意内容） |
| 删除 | 无 | `DELETE /{id}` |
| 批量 | 无 | `POST /batch` |
| 导出 | 无 | `GET /export` |
| 导入 | 无 | `POST /import` |

---

## 全模块批量操作汇总

所有模块统一使用 `POST /admin/xxx/batch`，请求体 `{action, ids}`。

| 模块 | 批量路由 | 支持的 action |
|---|---|---|
| posts | `POST /admin/posts/batch` | `delete`, `publish`, `unpublish` |
| categories | `POST /admin/categories/batch` | `delete` |
| tags | `POST /admin/tags/batch` | `delete` |
| comments | `POST /admin/comments/batch` | `delete`, `approve`, `reject`, `spam` |
| media | `POST /admin/media/batch` | `delete` |
| users | `POST /admin/users/batch` | `disable`, `enable`, `change_role` |
| pages | `POST /admin/pages/batch` | `delete`, `publish`, `unpublish` |
| reusable_blocks | `POST /admin/reusable-blocks/batch` | `delete` |
| content_type schema | `POST /admin/content-types/batch` | `delete`, `export`, `duplicate` |
| CMS content (per type) | `POST /admin/cms/{plural}/batch` | `delete`, `publish`, `unpublish`, `archive` |
| cron | `POST /admin/crons/batch` | `delete`, `enable`, `disable` |
| plugin | `POST /admin/plugins/batch` | `enable`, `disable` |
| rbac roles | `POST /admin/rbac/roles/batch` | `delete` |
| tenant | `POST /admin/tenants/batch` | `delete`, `suspend`, `activate` |
| webhook | `POST /admin/webhooks/batch` | `delete`, `enable`, `disable` |

---

## 实施顺序

```
Phase 1 — 数据模型
  ├── 定义 BatchRequest<T> 通用请求体
  ├── 定义各模块的 BatchAction enum
  ├── 定义 AdminCreateXxx / AdminUpdateXxx DTO（与 public 版本的区别）
  └── CMS 动态内容的 batch 路由注册机制

Phase 2 — 核心模块 Admin CRUD
  ├── posts — admin create/update/delete
  ├── categories — admin create/update/delete
  ├── tags — admin create/update/delete
  ├── comments — admin list/update_status/delete
  ├── CMS content — public/admin 读写分离
  └── content_type schema — 字段级 CRUD

Phase 3 — 批量操作
  ├── posts — batch delete/publish/unpublish
  ├── comments — batch approve/reject/spam/delete
  ├── media — batch delete
  ├── CMS content — batch delete/publish/unpublish/archive
  ├── content_type schema — batch delete/export/duplicate
  └── users — batch disable/enable/change_role

Phase 4 — 其余模块 + SDK 更新
  ├── media — admin CRUD
  ├── users — admin CRUD
  ├── pages — 补 create + batch
  ├── cron/plugin/rbac/tenant/webhook/reusable_block — 补 batch
  └── SDK: 所有 admin UI 调用从 /xxx 切到 /admin/xxx

Phase 5 — tests
  └── 每个 admin 路由补集成测试（CRUD + 权限校验 + 批量操作）
```

---

## Admin vs Public Handler 差异示例（posts）

| 行为 | Public `POST /posts` | Admin `POST /admin/posts` |
|---|---|---|
| 权限 | auth（author+） | auth（admin） |
| 作者 | 自动取当前用户 | 可指定任意用户（`author_id`） |
| 状态 | 默认 draft | 可直接 published |
| slug 冲突 | 报错 | 可强制覆盖 |
| 分类/标签 | 只能用已有的 | 可同时创建新分类/标签 |
| 审计日志 | 不记录 | 记录操作者 |
