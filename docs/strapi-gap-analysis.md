# Strapi 替代方案 — 差距分析与路线图

> 目标：高性能、高安全性、高扩展性的 Strapi 替代品

## 当前完成度：约 55%

| 类别 | 完成度 | 说明 |
|------|--------|------|
| **核心架构** | 95% | Handler→Service→Model、插件系统、CMS 引擎 |
| **认证/权限** | 80% | JWT + RBAC，缺密码重置 |
| **Content Type** | 90% | Builder UI + 动态 CRUD，缺组件/动态区块 |
| **Admin UI** | 70% | 20 个模块，缺 Relation Picker、批量编辑 |
| **媒体** | 50% | 上传/缩略图，缺裁剪/焦点/多格式 |
| **基础设施** | 75% | Worker/Cron/Webhook/搜索，缺 SMTP/重试日志 |
| **Strapi 标配功能** | 20% | 缺 GraphQL/内容 i18n/导入导出/主题 |

---

## 逐项差距分析

### 1. GraphQL API — 缺失 (0%)

**现状**：项目仅支持 REST API，所有路由注册在 `/api/v1/`。Cargo.toml 中无任何 GraphQL crate。

**Strapi**：提供完全自动生成的 GraphQL API（queries、mutations、subscriptions），支持过滤、分页、排序、关联自动解析。

**差距**：
- 无 GraphQL schema 生成
- 无 GraphQL 查询引擎
- 无 subscription（实时数据推送）
- 无 GraphQL playground

---

### 2. Content-Type Builder UI — 完成 (~90%)

**现状**：
- 完整的 Content-Type Builder 页面（1013 行），支持创建/编辑 content type
- 字段类型：text, richtext, email, password, uid, integer, bigint, decimal, float, date, datetime, time, boolean, enum, json, media, relation
- 字段配置：required, unique, private, immutable, label, description, default, max_length, min/max, enum_values, relation config, media config
- 字段上下移动排序、删除字段
- draft/publish, timestamps, soft_delete, slug_field 开关

**Strapi 额外能力**：
- 拖拽排序（本项目用上下箭头）
- 组件/可组合类型（Component，可复用字段组）
- 动态区块（Dynamic Zone，运行时选择组件类型）
- Schema 实时预览

---

### 3. 媒体库 — 部分 (~50%)

**现状**：
- 上传/下载/删除，MIME 校验 + magic-byte 验证
- 网格/列表视图、搜索、按类型过滤、排序
- 详情面板（类型、大小、尺寸、URL、日期）
- 自动缩略图生成（WebP，后台 Worker）
- 存储抽象（Local + S3）
- 统计端点

**Strapi 额外能力**：
- 裁剪/焦点编辑 UI
- 多格式/响应式图片变体（srcset）
- 焦点选择（smart cropping）
- 文件夹/相册组织
- 多文件上传进度条

---

### 4. 内容 i18n（每条记录多语言）— 缺失 (0%)

**现状**：仅有 **UI 层面** 的 i18n（rust_i18n + next-intl，支持 en/zh-CN）。posts 表无 locale 列，无 translation_of 外键，无记录级翻译管理。

**Strapi**：每条内容可有 en/zh/fr 等多语言版本，管理员在编辑界面切换语言，支持回退 locale。

**差距**：
- 无 per-record 多语言模型
- 无语言切换 UI
- 无 fallback locale 逻辑
- 无翻译同步机制

---

### 5. Admin 关联管理 — 部分 (~30%)

**现状**：Content-Type Builder 支持定义 relation 字段，后端有完整的 relation 解析（批量 ManyToOne/OneToOne/OneToMany/ManyToMany）。但 **前端 relation 字段渲染为普通文本 Input**，用户必须手动输入关联记录 ID。

**Strapi**：完整的 Relation Picker，支持搜索、分页、多选（ManyToMany）、从 Picker 内直接创建关联记录。

**差距**：
- 无下拉/combobox 浏览选择关联记录
- 无搜索联想
- 无分页浏览
- 无内联创建关联记录
- 不显示关联记录的 title/name（仅显示原始 ID）

---

### 6. 用户/角色管理 UI — 完成 (~80%)

**现状**：
- 用户管理页（列表、创建、改角色、分页）
- RBAC 页（角色 CRUD、权限管理、系统角色保护）
- 后端完整的 role/permission 数据库操作

**Strapi 额外能力**：
- 管理员编辑其他用户的资料（email/username/bio）
- 用户封禁/停用/暂停
- 用户头像管理
- 更细粒度的字段级权限控制
- 可视化权限矩阵编辑器

---

### 7. Webhook 投递重试 — 部分 (~60%)

**现状**：
- Worker 框架级指数退避重试（base × 2^attempt + jitter）
- 最大重试次数（默认 3），死信队列
- 死信任务可手动重试
- HMAC-SHA256 签名
- Webhook handler 非 2xx 返回 error 触发重试

**Strapi 额外能力**：
- 每个订阅独立配置重试策略
- 投递日志/历史（每次尝试的响应码、响应体）
- Webhook 调试 UI
- 自动熔断/禁用失败 webhook

---

### 8. 邮件发送 (SMTP) — 缺失 (0%，仅有 stub)

**现状**：`SendWelcomeEmailHandler` 仅 log 到控制台，有注释掉的 SMTP 代码。AppConfig 无 SMTP 字段，Cargo.toml 无 lettre 依赖。

**Strapi**：完整的邮件 Provider 系统，支持 SendGrid、Mailgun、Amazon SES、自定义 SMTP。

---

### 9. 密码重置流程 — 缺失 (0%)

**现状**：Auth handler 支持 register/login/refresh/logout/change_password。无 forgot-password/reset-password endpoint，无 reset token，无相关 UI。

**Strapi**：完整的 forgot-password → 邮件 token → reset-password 流程。

---

### 10. API 版本管理 — 部分 (~20%)

**现状**：所有 API 路由挂在 `/api/v1/`，仅为 URL 前缀。

**Strapi**：支持多版本并行、版本协商、废弃通知。

---

### 11. 插件市场 — 部分 (~50%)

**现状**：
- 插件管理 UI（列表、启用/禁用/重载/删除）
- 完整插件系统（WASM/JS/Lua 三引擎）
- 插件热重载、健康监控、VFS、权限、存储
- 扩展安装/启用/禁用/卸载

**Strapi 额外能力**：在线 Marketplace，浏览/搜索/一键安装社区插件。

---

### 12. 主题系统 — 极少 (~10%)

**现状**：仅有 dark/light CSS 切换（next-themes + Tailwind dark mode）。

**Strapi**：管理面板主题引擎（自定义颜色/logo/字体）+ 公开主题市场 + 模板引擎。

---

### 13. 导入/导出 — 缺失 (0%)

**现状**：无任何导入导出功能。

**Strapi**：WordPress 导入器、CSV/JSON 导入、全库导出/导入、实例间迁移。

---

### 14. 内容审核 — 部分 (~60%)

**现状**：
- 评论审核队列 UI（状态过滤、审批/拒绝/删除、批量操作）
- 评论创建默认 pending，仅 approved 公开显示
- 插件 hook `on_comment_creating`
- 嵌套评论深度验证（最多 3 层）

**Strapi 额外能力**：垃圾评论检测/过滤、自动审批规则、AI 内容审核。

---

### 15. 定时发布 — 部分 (~40%)

**现状**：
- `ScheduledPublishHandler` 完整实现并通过测试
- 已注册到 handler registry
- **但没有任何代码触发它**：`JobEnqueuer` 不处理 `ScheduledPublish`，DTO 无 `publish_at` 字段，无 UI 设置发布时间

**Strapi**：编辑器设置发布日期，系统自动在指定时间发布。

---

## 路线图

### Phase 1 — 补齐基础体验（2-3 周）

目标：**用户能完整使用，不卡在基本流程上**

| # | 任务 | 优先级 | 工作量 |
|---|------|--------|--------|
| 1.1 | SMTP 邮件发送（lettre + 配置） | P0 | 2天 |
| 1.2 | 密码重置流程（forgot/reset endpoint + 邮件） | P0 | 1天 |
| 1.3 | 定时发布对接（handler 已有，加 glue code） | P0 | 0.5天 |
| 1.4 | Relation Picker UI（下拉搜索选择关联记录） | P0 | 2天 |
| 1.5 | Webhook 投递日志（delivery history + 重试 UI） | P1 | 1天 |

### Phase 2 — Admin UI 打磨（2-3 周）

目标：**Admin 体验不输 Strapi**

| # | 任务 | 工作量 |
|---|------|--------|
| 2.1 | Content-Type Builder 拖拽排序（dnd-kit 替代箭头） | 1天 |
| 2.2 | 批量操作（列表页多选 + 批量删除/发布/移动） | 2天 |
| 2.3 | 媒体库增强（文件夹组织 + 多文件上传进度条） | 2天 |
| 2.4 | 内联编辑器（列表页直接编辑字段，不用进详情） | 2天 |
| 2.5 | Admin 全局搜索（Cmd+K 弹窗，搜索所有资源） | 1天 |

### Phase 3 — 核心差异化功能（3-4 周）

目标：**补齐 Strapi 标配 + 发挥 Rust 优势**

| # | 任务 | 工作量 |
|---|------|--------|
| 3.1 | GraphQL API（async-graphql + 自动 schema 生成） | 5天 |
| 3.2 | 内容 i18n（locale 列 + translation_of FK + Admin 切换） | 3天 |
| 3.3 | 导入导出（JSON/CSV + WordPress 导入器） | 3天 |
| 3.4 | 实时 API（SSE → WebSocket 升级 + 订阅过滤） | 2天 |
| 3.5 | Dashboard 仪表盘（统计图表 + 最近活动 + 快捷操作） | 2天 |

### Phase 4 — 开发者体验（2-3 周）

目标：**让其他开发者愿意用**

| # | 任务 | 工作量 |
|---|------|--------|
| 4.1 | 文档站（Getting Started + API Reference + Plugin 开发指南） | 5天 |
| 4.2 | Demo 站点（在线体验，预装示例数据） | 2天 |
| 4.3 | 一键部署（`docker compose up` 3 分钟跑起来） | 2天 |
| 4.4 | Plugin CLI（`rblog plugin new/create/publish`） | 3天 |
| 4.5 | TypeScript SDK 生成（从 OpenAPI spec 自动生成） | 2天 |

### Phase 5 — 插件生态（3-4 周）

目标：**有可用的官方插件**

| # | 任务 | 工作量 |
|---|------|--------|
| 5.1 | 插件市场 UI（浏览/搜索/一键安装） | 3天 |
| 5.2 | 插件注册中心（简单的 registry API + 存储） | 3天 |
| 5.3 | 官方插件：Stripe 支付 | 3天 |
| 5.4 | 官方插件：Algolia/Meilisearch 搜索 | 2天 |
| 5.5 | 官方插件：邮件营销 | 3天 |
| 5.6 | 官方插件：AI 内容生成 | 3天 |

### Phase 6 — 企业级功能（4-6 周）

目标：**能卖给企业**

| # | 任务 | 工作量 |
|---|------|--------|
| 6.1 | SSO/SAML | 5天 |
| 6.2 | 审计日志增强（导出 CSV、合规报告） | 2天 |
| 6.3 | 内容审批工作流（多级审批、评论批注） | 5天 |
| 6.4 | 主题引擎（模板系统 + 主题市场） | 5天 |
| 6.5 | PostgreSQL 生产支持 | 5天 |
| 6.6 | 多数据库连接（读写分离） | 3天 |

---

## 总时间估算

| 阶段 | 时间 | 里程碑 |
|------|------|--------|
| Phase 1 | 2-3 周 | 可自用的完整 CMS |
| Phase 2 | 2-3 周 | Admin 体验达标 |
| Phase 3 | 3-4 周 | 功能对齐 Strapi |
| Phase 4 | 2-3 周 | 开源发布就绪 |
| Phase 5 | 3-4 周 | 有插件生态 |
| Phase 6 | 4-6 周 | 企业可用 |
| **总计** | **16-23 周** | Strapi 替代 |

---

## 核心竞争优势

### vs Strapi

| 维度 | rust-blog | Strapi |
|------|-----------|--------|
| **性能** | 单实例 10x QPS，内存 1/10 | Node.js 需要集群才能到千级 QPS |
| **部署** | 单二进制 30MB，零依赖 | Node.js 18+ + npm + PostgreSQL，200MB+ |
| **插件安全** | WASM 沙箱隔离，插件崩溃不影响主进程 | JS 插件直接 require()，一个 crash 全挂 |
| **冷启动** | <100ms | 3-5 秒 |
| **二进制安全** | 编译型，源码不可逆 | JavaScript 源码可直接阅读 |
| **GC** | 无 GC 暂停 | V8 GC 导致 P99 延迟毛刺 |
| **单机密度** | 一台 1C1G VPS 跑几十个站点（SQLite） | 每个站点需要独立进程 + 数据库连接 |

### 独特卖点

1. **WASM 插件沙箱** — Strapi 完全没有的安全隔离机制，是构建可信插件市场的前提
2. **极致轻量部署** — 30MB 单文件替代 Node.js 全家桶，适合嵌入式、边缘、IoT 场景
3. **SQLite 原生架构** — 多租户托管成本极低（每站点一个 .db 文件），不依赖外部数据库
4. **三引擎插件** — WASM + JS + Lua，开发者可按需选择性能/生态平衡点
