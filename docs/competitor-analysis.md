# 竞争对手分析与平台演进路线

> raisfast 定位：**Rust 单体嵌入式后端 + 插件系统 + Headless CMS + HTTP/Tauri 双模式部署**

---

## 1. Headless CMS 竞品

| 项目 | 语言 | 数据库 | 插件/扩展 | Admin UI | 部署模式 | Stars |
|---|---|---|---|---|---|---|
| **Strapi** | Node.js | PostgreSQL/MySQL/SQLite | ✅ 插件市场 | ✅ React | 独立服务器 | 65k+ |
| **Directus** | Node.js/TypeScript | PostgreSQL/MySQL/SQLite/Oracle | ✅ 扩展 | ✅ Vue | 独立服务器 | 29k+ |
| **Payload** | TypeScript | MongoDB/Postgres | ✅ 插件 | ✅ React (Next.js) | 独立服务器/Serverless | 25k+ |
| **Sanity** | TypeScript/JS | 专有云存储 | ✅ 插件 | ✅ React (Studio) | SaaS only | 32k+ |
| **Ghost** | Node.js | MySQL | 有限（主题+Hook） | ✅ 自带 | 独立服务器 | 49k+ |
| **Keystatic** | TypeScript | 文件系统/任意 CMS | ❌ | ✅ React | 嵌入 Next.js | 5k+ |
| **TinaCMS** | TypeScript | 文件系统/Git | ❌ | ✅ React | 嵌入前端 | 12k+ |

### Strapi（最强劲的 Headless CMS 竞品）

**优势：**
- 成熟的插件生态系统（300+ 社区插件）
- 完善的 Admin UI（React + Design System）
- Content-Type Builder 可视化建表
- REST + GraphQL 双 API
- 角色权限系统（RBAC）精细控制
- i18n / 多语言支持
- Cloud 版本（托管服务）
- 企业客户背书（Toyota、IBM、NASA）

**劣势：**
- Node.js 单线程，高并发性能有限
- 插件热加载不可靠，开发体验差
- SQLite 支持不完善（官方推荐 PG/MySQL）
- 部署依赖 Node.js 运行时（~200MB+）
- v4 → v5 迁移频繁破坏性变更
- 无桌面应用模式
- 无多语言插件沙箱（只支持 JS 插件）
- 内存占用较高（空闲 ~150MB）

### Directus

**优势：**
- 可连接已有数据库（不锁定）
- 支持多种数据库（含 Oracle、CockroachDB 等）
- 实时订阅（WebSocket）
- Dashboard / Insights 可视化
- 文件转换（图片裁剪、视频转码）

**劣势：**
- Node.js 性能瓶颈同 Strapi
- Docker 部署复杂（多容器）
- 无桌面模式
- 插件系统不如 Strapi 成熟
- SQLite 支持有限

### Payload CMS

**优势：**
- 深度集成 Next.js，SSR/SSG 天然支持
- TypeScript 类型安全（端到端）
- 代码优先（code-first），Schema 即代码
- 原生支持 Edge / Serverless 部署

**劣势：**
- 必须用 Next.js 生态
- 无插件市场
- 社区规模较小
- 无桌面模式

---

## 2. Self-hosted BaaS 竞品

| 项目 | 语言 | 数据库 | 插件 | Auth | Storage | Admin UI | 部署 |
|---|---|---|---|---|---|---|---|
| **PocketBase** | Go | SQLite | ❌ | ✅ | ✅ | ✅ | 单二进制 |
| **Supabase** | TypeScript | PostgreSQL | Edge Functions | ✅ | ✅ | ✅ | Docker 多容器 |
| **Appwrite** | TypeScript | MariaDB | ✅ | ✅ | ✅ | ✅ | Docker 多容器 |
| **Nhost** | TypeScript | PostgreSQL | ✅ | ✅ | ✅ | ✅ | Docker + Hasura |
| **Casdoor** | Go | PostgreSQL/MySQL | ✅ | ✅（专注Auth） | ❌ | ✅ | 独立/Docker |
| **Logto** | TypeScript | PostgreSQL | ❌ | ✅（专注Auth） | ❌ | ✅ | Docker |

### PocketBase（最直接的竞争对手）

**优势：**
- 单二进制文件，零依赖部署（~30MB）
- 嵌入式 SQLite，开箱即用
- 内置 Admin Dashboard（React）
- Auth（OAuth2、Email/Password）
- 实时订阅
- 文件存储
- Go 语言性能优秀
- 极简 API 设计
- 活跃社区（40k+ Stars）

**劣势：**
- **无插件系统**（这是最大的差距）
- 无 Headless CMS 能力（无 Content-Type Builder）
- 无桌面应用模式
- Go 生态不如 JS/Rust 丰富
- RBAC 相对简单
- 无多租户支持
- 扩展只能通过 Go 嵌入（需要重新编译）

### Supabase

**优势：**
- PostgreSQL 全能力（RLS、JSONB、全文搜索）
- 实时订阅（WebSocket）
- Edge Functions（Deno）
- 完善的客户端 SDK（JS/Dart/Swift/Kotlin）
- 存储带 CDN
- 向量搜索（pgvector，AI 场景）
- 大厂背书（GitHub、Vercel 合作）

**劣势：**
- 必须使用 PostgreSQL
- 部署复杂（Docker Compose 10+ 容器）
- 无桌面模式
- Edge Functions 冷启动延迟
- 免费版限制多
- 不适合嵌入式/离线场景

### Appwrite

**优势：**
- 功能最全的 Self-hosted BaaS
- 多语言 SDK（14 种语言）
- 内置 Functions（多 runtime）
- 实时订阅
- 关系型数据库抽象

**劣势：**
- Docker 部署复杂
- 依赖 MariaDB
- 资源占用大（最低 2GB RAM）
- 无桌面模式
- 无 Headless CMS 能力

---

## 3. Rust 生态竞品

| 项目 | 定位 | 特点 | 与我们的差距 |
|---|---|---|---|
| **Loco** | Rust 全栈框架（类 Rails） | Axum + SeaORM，CLI 脚手架 | 无 CMS、无插件、无桌面 |
| **Salvo** | Rust Web 框架 | 高性能 HTTP，WebSocket | 纯框架，无业务能力 |
| **Actix-web** | Rust HTTP 框架 | 最快的 Web 框架之一 | 纯框架，无 CMS/BaaS |
| **Spin** (Fermyon) | WASM Serverless | 组件模型，多语言 | 偏 Serverless，不做 BaaS |
| **Shuttle** | Rust 部署平台 | 一键部署 Rust 服务 | 纯部署工具 |
| **Deno** | JS/TS Runtime | 内置 TS、Web API、FFI | 偏 Runtime，非框架 |

**Rust 生态目前没有 BaaS / Headless CMS 产品。** 这是一个明显的市场空白。

---

## 4. 桌面 + 后端双模式竞品

| 项目 | 技术栈 | 特点 |
|---|---|---|
| **Electron** | Chromium + Node.js | 主流方案，但体积大（~150MB）、内存高 |
| **Tauri** | Rust + WebView | 轻量（~10MB），但无人做 BaaS 后端 |
| **Wails** | Go + WebView | Go 生态的 Tauri，无 BaaS |
| **Neutralinojs** | C++ + WebView | 极轻量，但功能有限 |

**全球范围内，目前没有人做「Tauri + CMS/BaaS 后端双模式」**。

现有的桌面应用后端方案都是 Electron + 内嵌 Express/NestJS，体积和性能都无法与 Rust 方案相比。

---

## 5. 竞争力矩阵

| 能力 | raisfast | PocketBase | Strapi | Supabase | Appwrite |
|---|---|---|---|---|---|
| 单二进制部署 | ✅ | ✅ | ❌ | ❌ | ❌ |
| 嵌入式 SQLite | ✅ | ✅ | ⚠️ | ❌ | ❌ |
| Headless CMS | ✅ | ❌ | ✅ | ❌ | ❌ |
| Content-Type Builder | ✅ | ❌ | ✅ | ❌ | ❌ |
| 插件系统 | ✅ | ❌ | ✅ | ✅ | ✅ |
| 多语言插件沙箱 | ✅ (JS/Lua/WASM) | ❌ | JS only | Deno only | 多 runtime |
| 桌面应用模式 | ✅ (Tauri) | ❌ | ❌ | ❌ | ❌ |
| Admin UI | ✅ | ✅ | ✅ | ✅ | ✅ |
| Auth + RBAC | ✅ | ✅ | ✅ | ✅ | ✅ |
| 实时订阅 | ✅ (WS/SSE) | ✅ | ❌ | ✅ | ✅ |
| API（REST） | ✅ | ✅ | ✅ | ✅ | ✅ |
| API（GraphQL） | ❌ | ❌ | ✅ | ✅ | ❌ |
| 全文搜索 | ✅ (Tantivy) | ❌ | ❌ | ✅ | ❌ |
| 多租户 | ✅ | ❌ | ❌ | ✅ | ❌ |
| 零依赖运行 | ✅ | ✅ | ❌ | ❌ | ❌ |
| WASM 支持 | ✅ | ❌ | ❌ | ❌ | ❌ |
| 二进制体积 | ~20MB | ~30MB | ~200MB+ | N/A | N/A |
| 空闲内存 | ~15MB | ~20MB | ~150MB+ | ~500MB+ | ~300MB+ |
| 语言安全 | ✅ (deny unsafe) | ✅ | ⚠️ | ⚠️ | ⚠️ |
| 性能（RPS） | 100k+ | 80k+ | 10k-30k | 30k-50k | 20k-40k |

---

## 6. 我们的独特定位

综合分析，raisfast 在全球范围内的独特定位是：

> **Rust 单体嵌入式后端 + 插件系统 + Headless CMS + 同时支持 HTTP 服务器和 Tauri 桌面应用**

这个定位目前**没有直接竞品**，它填补了以下空白：

1. **PocketBase 没有**的：插件系统、CMS、多租户
2. **Strapi 没有**的：单二进制、桌面模式、多语言插件沙箱、高性能
3. **Supabase 没有**的：单二进制、嵌入式 SQLite、桌面模式、离线运行
4. **Tauri 生态没有**的：完整的后端 BaaS/CMS 能力

### 核心卖点

- **一个二进制，两种部署**：同一套 Rust 代码，既可运行 HTTP 服务器，也可编译为 Tauri 桌面应用
- **多语言插件沙箱**：JS（QuickJS ES2024）/ Lua / WASM，热加载，权限隔离
- **嵌入式运行**：SQLite 内嵌，零外部依赖，适合边缘计算和离线场景
- **Rust 安全 + 性能**：`deny(unsafe_code)`，内存安全，100k+ RPS
- **Content-Type Builder**：TOML 定义 Schema，自动 CRUD API + 数据库迁移
- **企业级特性**：RBAC、多租户、审计日志、API Token、全文搜索

### 市场切入点建议

1. **PocketBase 用户升级路径**：需要插件/CMS/多租户的 PocketBase 用户
2. **Electron 开发者迁移**：需要轻量桌面 + 后端的团队
3. **嵌入式/边缘场景**：需要离线运行的 IoT、POS、Kiosk 应用
4. **Rust 生态填充**：Rust 社区目前缺少 BaaS 产品
5. **Strapi 轻量替代**：不需要 Node.js 运行时的 Headless CMS 需求

---

## 7. 平台化演进分析

### 7.1 什么是「综合平台」

从单功能工具演进为平台，需要三层能力：

```
┌─────────────────────────────────────────────────────┐
│              前端模板市场 (Theme/Template Store)       │
│   Blog / E-commerce / Forum / SaaS Starter / Docs   │
├─────────────────────────────────────────────────────┤
│              插件生态 (Plugin Marketplace)             │
│   支付 / 邮件 / OAuth / AI / 搜索 / 通知 / 分析      │
├─────────────────────────────────────────────────────┤
│              核心引擎 (raisfast)                     │
│   Auth / CMS / RBAC / 多租户 / 插件沙箱 / API        │
├─────────────────────────────────────────────────────┤
│              双模式运行时                              │
│   HTTP Server (Web/SaaS)  │  Tauri (桌面/离线)       │
├─────────────────────────────────────────────────────┤
│              数据层                                   │
│   SQLite (嵌入式)  │  PostgreSQL (生产)               │
└─────────────────────────────────────────────────────┘
```

### 7.2 前端模板生态

#### 竞品模板生态现状

| 平台 | 模板数量 | 模板类型 | 定价模式 |
|---|---|---|---|
| **Strapi** | 10+ starters | Next/Nuxt/Gatsby | 免费开源 |
| **Payload** | 5+ templates | Next.js | 免费开源 |
| **Directus** | 10+ starters | Vue/Nuxt/Next | 免费开源 |
| **Supabase** | 20+ examples | React/Svelte/Flutter | 免费开源 |
| **Refine** (dev) | 30+ templates | React Admin | 免费 + 付费 |
| **TemplateMonster** | 10000+ | WordPress/Shopify | 付费市场 |
| **ThemeForest** | 50000+ | WordPress/HTML/CMS | 付费市场 |

#### 我们的差异化机会

**现有 Admin UI 已有基础**（29 个管理页面），可以拆分为：

```
web/                          # 当前 Next.js 前端
├── admin/                    # Admin UI（已有）
├── templates/                # 前端模板
│   ├── blog-starter/         # 博客模板
│   ├── e-commerce/           # 电商模板（对接 ecommerce 插件）
│   ├── forum/                # 论坛模板（对接 forum 插件）
│   ├── saas-dashboard/       # SaaS 后台模板
│   ├── docs-site/            # 文档站模板
│   ├── portfolio/            # 作品集模板
│   └── landing-page/         # 落地页模板
└── adapters/                 # 前端 adapter 层
    ├── http.ts               # fetch() 调用（Web 模式）
    └── tauri.ts              # invoke() 调用（桌面模式）
```

**关键技术点**：前端 adapter 层，同一套 React 组件切换数据源：

```typescript
// adapters/http.ts — Web 模式
export const api = {
  getPosts: () => fetch('/api/v1/posts').then(r => r.json()),
  createPost: (data) => fetch('/api/v1/posts', { method: 'POST', body: JSON.stringify(data) }),
};

// adapters/tauri.ts — 桌面模式
import { invoke } from '@tauri-apps/api/core';
export const api = {
  getPosts: () => invoke('list_posts', { page: 1, pageSize: 20 }),
  createPost: (data) => invoke('create_post', { data }),
};
```

#### 模板生态建设路线

| 阶段 | 目标 | 交付物 |
|---|---|---|
| **Phase 1**（1-2 月） | Admin UI + 前端 adapter | adapter 层 + 桌面应用打通 |
| **Phase 2**（2-3 月） | 3 个官方模板 | Blog / E-commerce / Forum |
| **Phase 3**（3-6 月） | 模板 CLI + 模板规范 | `ext template create` 命令 |
| **Phase 4**（6-12 月） | 社区模板 | 模板市场网站 + 提交规范 |

### 7.3 插件生态

#### 竞品插件生态对比

| 平台 | 插件数量 | 插件语言 | 审核机制 | 变现模式 |
|---|---|---|---|---|
| **Strapi** | 300+ | JS/Node.js | 官方审核 | 免费为主 |
| **WordPress** | 60000+ | PHP | 自动 + 审核 | 免费 + 付费市场 |
| **Shopify** | 8000+ | Ruby/JS | 严格审核 | 付费为主（分成） |
| **Grafana** | 100+ | Go/JS/TS | 社区 + 审核 | 免费为主 |
| **PocketBase** | 0 | Go（需编译） | 无 | 无 |
| **Supabase** | ~50 Edge Functions | Deno/TS | 无 | 无 |

#### 我们的插件系统优势

当前插件架构已具备：

| 能力 | 状态 | 说明 |
|---|---|---|
| JS 插件 (QuickJS) | ✅ | ES2024 语法，Host API 20+ |
| Lua 插件 | ✅ | Lua 5.4 沙箱 |
| WASM 插件 | ✅ | WASI 组件模型 |
| 参数化查询 | ✅ | 防 SQL 注入 |
| 事务支持 | ✅ | begin/commit/rollback |
| 事件系统 | ✅ | 插件间通信 + WS 推送 |
| 文件系统 (VFS) | ✅ | 沙箱内安全文件操作 |
| 热加载 | ✅ | `PLUGIN_HOT_RELOAD=true` |
| 权限控制 | ✅ | DB/HTTP/FS 独立授权 |
| 版本迁移 | ✅ | `migrations/{version}.sql` |
| HTTP 路由注册 | ✅ | Plugin 自定义 REST API |
| Hook 系统 | ✅ | content/post/comment 全生命周期 |

#### 可建设的官方插件

| 插件 | 优先级 | 说明 |
|---|---|---|
| **auth-oauth** | P0 | GitHub/Google/Apple OAuth2 |
| **storage-s3** | P0 | S3/MinIO 文件存储 |
| **email-smtp** | P0 | 邮件发送（注册验证/通知） |
| **payment-stripe** | P1 | Stripe 支付集成 |
| **search-meilisearch** | P1 | Meilisearch 全文搜索 |
| **ai-embeddings** | P1 | 向量嵌入 + 语义搜索 |
| **notification** | P1 | 多渠道通知（邮件/WebSocket/Push） |
| **import-export** | P1 | 数据导入导出（CSV/JSON/SQL） |
| **seo** | P2 | Sitemap/robots.txt/Open Graph |
| **analytics** | P2 | 访问统计（轻量 Piwik 替代） |
| **backup-cloud** | P2 | 云端自动备份 |
| **cron** | P2 | 定时任务调度（已有基础设施） |
| **webhook** | P2 | 出站 Webhook（已有基础设施） |
| **media-processor** | P2 | 图片裁剪/压缩/水印 |
| **cache-redis** | P3 | Redis 缓存适配器 |
| **i18n** | P3 | 多语言内容管理 |
| **workflow** | P3 | 简单工作流引擎（审批/发布流程） |

### 7.4 平台化可行性评估

#### 优势（为什么能成）

1. **技术壁垒高**：Rust + 多语言插件沙箱 + 双模式部署，全球唯一
2. **性能碾压**：单二进制 20MB、空闲 15MB 内存、100k+ RPS，Node.js 方案无法竞争
3. **桌面蓝海**：Tauri + 后端双模式，没有竞品，需求真实（离线 POS、Kiosk、桌面工具）
4. **Rust 生态窗口**：Rust 社区快速增长（连续 9 年 Stack Overflow 最受喜爱语言），缺少 BaaS 产品
5. **嵌入式/边缘趋势**：IoT、边缘计算需要轻量级嵌入式后端，SQLite 单文件正好
6. **已有基础扎实**：111+ API、29 页 Admin、3 个示例扩展、插件沙箱完整

#### 风险（为什么可能失败）

1. **生态冷启动**：没有社区 = 没有插件 = 没有用户 = 没有社区（死亡螺旋）
2. **一人开发瓶颈**：Strapi 有 100+ 员工、PocketBase 有商业公司，平台级产品需要团队
3. **文档和教程缺口**：平台成功靠文档，不是代码。Strapi 的成功 50% 归功于文档
4. **前端生态劣势**：React/Next.js 生态选型正确，但模板制作需要大量设计工作
5. **GraphQL 缺失**：部分企业用户要求 GraphQL API

#### 成功概率评估

| 场景 | 概率 | 条件 |
|---|---|---|
| **Rust 社区明星项目**（3k+ Stars） | 60% | 文档完善 + 3 个以上官方插件 + 好的 README |
| **小众但盈利的工具**（独立开发者付费） | 40% | 桌面模式差异化 + 商业授权 |
| **中等规模平台**（10k+ Stars） | 20% | 需要团队 + 社区运营 + 模板市场 |
| **大规模平台**（对标 Strapi） | 5% | 需要融资 + 全职团队 + 2-3 年持续投入 |

### 7.5 推荐演进路线

```
Phase 0 (当前)          Phase 1               Phase 2              Phase 3
─────────────    ──────────────────    ──────────────────   ──────────────────
后端引擎完成  →   杀手场景做透       →   生态基础建设      →   平台化运营
111+ API          ↓                    ↓                    ↓
插件沙箱          桌面应用 Demo        前端 adapter          模板市场网站
Admin UI          3 个官方模板         5 个官方插件          插件市场网站
Tauri 适配        完整文档 + 教程      模板 CLI 规范         社区贡献机制
                  GitHub Stars 推广    插件审核流程          商业模式（付费插件）
```

#### Phase 0 → Phase 1 的关键动作（建议未来 1-2 月）

1. **做一个「Wow Demo」**：桌面应用（Tauri），内嵌 SQLite，启动即用，零配置
2. **3 个前端模板**：Blog（展示型）、E-commerce（交易型）、Forum（社区型）
3. **杀手文档**：
   - 5 分钟快速开始
   - 30 分钟完整教程（从零到部署）
   - API 参考（自动生成 OpenAPI）
4. **3 个核心插件**：OAuth 登录、邮件发送、S3 存储
5. **开源推广**：Reddit r/rust、Hacker News、Rust 中文社区

#### 判断是否继续投入的标准

| 指标 | 3 个月后 | 6 个月后 | 判断 |
|---|---|---|---|
| GitHub Stars | 500+ | 2000+ | 继续投入 |
| 活跃贡献者 | 3+ | 10+ | 生态启动 |
| 社区插件 | 2+ | 10+ | 平台化可行 |
| 付费用户 | 0 | 10+ | 商业可行 |
| 周下载量 | 100+ | 500+ | 市场验证 |

**如果 6 个月后 Stars < 500、无外部贡献者，建议调整为「个人工具/内部框架」定位，不再投入平台化。**

### 7.6 商业模式参考

| 模式 | 代表 | 可行性 | 说明 |
|---|---|---|---|
| **开源 + 托管云** | Supabase/Strapi Cloud | ⭐⭐ | 需要 DevOps 投入 |
| **开源 + 付费插件** | Grafana/Tailwind UI | ⭐⭐⭐ | 最轻量，适合个人 |
| **开源 + 商业授权** | PocketBase/Electron | ⭐⭐⭐ | MIT 个人免费，企业付费 |
| **纯开源 + 赞助** | Directus/Ghost | ⭐⭐ | 需要 Star 规模支撑 |
| **Open Core** | GitLab/Supabase | ⭐ | 需要明确免费/付费边界 |

推荐路径：**开源 + 付费插件 + 商业授权**（先做影响力，再变现）

### 7.7 最终竞争力全景（目标状态）

补齐 Content-Type Builder 可视化后，系统将达到无短板状态：

```
┌──────────┬──────┬──────────────────────────────────────┐
│ 维度      │ 评级  │ 说明                                  │
├──────────┼──────┼──────────────────────────────────────┤
│ 性能      │ ★★★★★ │ Rust + Axum + SQLite，100k+ RPS       │
│ 扩展性    │ ★★★★★ │ JS/Lua/WASM 沙箱 + 热加载 + 权限隔离   │
│ 开发速度  │ ★★★★★ │ 可视化 Builder + 脚手架 + 热加载       │
│ 部署灵活  │ ★★★★★ │ 单二进制 + HTTP/Tauri 双模式           │
│ 安全性    │ ★★★★★ │ deny(unsafe) + 参数化查询 + RBAC       │
│ 生态潜力  │ ★★★★☆ │ 插件市场 + 模板市场（待建设）           │
└──────────┴──────┴──────────────────────────────────────┘
```

在 Self-hosted BaaS / Headless CMS 赛道上，全球范围内没有第二个产品能同时达到这个水平：

- **PocketBase**：性能强，但无插件、无 CMS、无桌面
- **Strapi**：开发快，但性能差、二进制大、无桌面
- **Supabase**：功能全，但部署重、无桌面、无嵌入式
- **raisfast**：性能最强 + 扩展性最强 + 开发速度最快（补齐 Builder 后） + 部署最灵活

**核心竞争力：用 Rust 的性能和安全性，做到 Node.js 方案的开发体验。**

---

## 8. 值得借鉴的设计

### 8.1 Directus — 反向连接已有数据库

Directus 不建表，而是读取现有数据库 schema，自动生成 Admin UI 和 API。用户可以用 SQL 直接建表，然后 Directus 自动识别。

**借鉴方向：** 未来支持 `--connect-mode`，连已有 SQLite/PostgreSQL 自动生成 TOML 定义。

**优先级：** P2 — 差异化功能，非 MVP 必需。

### 8.2 Payload CMS — 字段级 Hook + 粒度 Access Control

```typescript
fields: [
  {
    name: 'price',
    type: 'number',
    access: {
      read: ({ req: { user } }) => user.role === 'admin',
      update: () => false,  // 创建后不可改
    },
    hooks: {
      beforeValidate: [validatePrice],
      afterChange: [syncToInventory],
    },
  }
]
```

**借鉴方向：**

- **字段级 Access Control（P0）** — 当前 API Rule 只到 endpoint 级别，企业场景需要字段级权限（如 `price` 字段仅 admin 可读、`author_id` 创建后不可改）。可在 TOML 字段定义中扩展 `access` 配置。
- **字段级 Hook（P1）** — 当前 Hook 粒度是 content type 级别（ContentCreating/ContentUpdated），没有字段级。字段级 Hook 和插件系统天然配合，如 `price` 变更时自动触发库存同步。

**优先级：** P0（字段级 Access Control）/ P1（字段级 Hook）

### 8.3 Supabase — RLS（行级安全）

```sql
CREATE POLICY "users_can_read_own" ON posts
  FOR SELECT USING (author_id = auth.uid());
```

直接在数据库层做行级权限，API 层完全不用管。

**借鉴方向：** 我们的 API Rule 已经在应用层实现了等价功能（`filter = 'author_id = @request.auth.id'`），不需要数据库层 RLS（SQLite 也不支持）。当前方案已经足够。

**优先级：** 已实现（等价方案）

### 8.4 Sanity — 内容版本树

```
v1 ── v2 ── v3（draft）
      └── v2a（另一个分支）
```

我们的 `content_revisions` 是线性历史列表。Sanity 做的是分支版本树，支持协作场景下多人同时编辑不同分支。

**借鉴方向：** 目前不需要。未来如果做多人协作编辑（类似 Google Docs），可以考虑分支版本树。当前线性 revision 已经满足审计和回滚需求。

**优先级：** P3

### 8.5 PocketBase — Realtime 订阅

```javascript
pb.collection('posts').subscribe('*', (e) => {
  console.log(e.action, e.record);
});
```

PocketBase 通过 SSE 长连接实现 Collection 级别的数据变更实时推送。

**借鉴方向：** 我们的 SSE 基础设施已就绪（`/api/v1/sse`），可以新增 Collection 级别的变更订阅：

```
GET /api/v1/cms/{plural}/subscribe
```

Content Type 的 create/update/delete 事件自动推送给订阅者。

**优先级：** P1 — 差异化卖点，实现成本低

### 8.6 Strapi — Content-Type Builder 可视化

Strapi 的 Admin UI 提供 Content-Type Builder，支持在线：
- 创建/编辑/删除 Content Type
- 添加/删除字段（可视化表单）
- 配置字段属性（类型、校验、关系）
- 即时生效（无需重启）

**借鉴方向：** 这是当前最优先需要补齐的能力。后端 API 已完备（`POST/PUT/DELETE /admin/content-types`），需要 Admin UI 前端对接。

**优先级：** P0 — 没有这个不能叫产品

### 8.7 借鉴优先级总结

| 借鉴点 | 来源 | 优先级 | 理由 |
|--------|------|--------|------|
| Content-Type Builder 可视化 | Strapi | **P0** | 没有这个不能叫产品 |
| 字段级 Access Control | Payload | **P0** | 企业场景刚需，API Rule 只到 endpoint 不够 |
| Admin 多视图（Board/Calendar） | Notion | **P0** | Content Type Admin UI 差异化 |
| Realtime SSE 订阅 | PocketBase | **P1** | 差异化卖点，基础设施已就绪 |
| 字段级 Hook | Payload | **P1** | 和插件系统天然配合 |
| 字段级 auto_generate（slug 等） | Keystone | **P1** | 减少硬编码，Content Type 更通用 |
| 触发器 → 动作链（Flow） | Directus | **P1** | 整合现有 Webhook/Worker/Event |
| 反向连接已有 DB | Directus | **P2** | 未来差异化 |
| 付费会员系统 | Ghost | **P2** | 创作者经济场景 |
| 服务端模板引擎（Tera） | Shopify | **P2** | 降低非开发者的自定义门槛 |
| 分支版本树 | Sanity | **P3** | 多人协作才需要 |
| Shortcode 动态组件 | WordPress | **P3** | 和 Block 系统重叠 |
| Section + Entry Type | Craft | **P3** | 和 Dynamic Zone 类似 |

---

## 9. 补充借鉴设计

### 9.1 WordPress — 短代码（Shortcode）

在内容中嵌入动态组件：

```
[contact-form email="admin@example.com"]
[gallery ids="1,2,3" columns="3"]
[latest-posts count="5" category="tech"]
```

**借鉴方向：** 可以在 PageBlock 里加 `Shortcode` 类型，或让 Richtext 块支持 `[plugin.xxx]` 语法，渲染时调用插件返回 HTML。

**优先级：** P3 — 和现有 Block 系统功能重叠

### 9.2 Ghost — 会员 + 付费订阅

Ghost 内置完整的付费会员系统：

```
免费会员 → 查看公开文章
付费会员（$5/月）→ 查看付费文章 + Newsletter
VIP（$15/月）→ 全部内容 + 社区
```

**借鉴方向：** 创作者经济最核心的功能。可通过 Content Type + 插件实现，但内置更方便。Stripe 集成 + 内容付费墙 + Newsletter 发送，是独立博客/创作者的刚需。

**优先级：** P2

### 9.3 Shopify Liquid — 服务端模板引擎

```liquid
{% for product in collection.products %}
  <h2>{{ product.title }}</h2>
  <span>{{ product.price | money }}</span>
{% endfor %}
```

**借鉴方向：** 当前前端完全是 Next.js（React），不支持服务端模板渲染。如果加轻量模板引擎（Rust Tera），可以让非开发者通过模板文件自定义页面，不需要 React 开发能力。Tauri 桌面模式下尤其有价值 — 用户可以离线编辑模板。

**优先级：** P2

### 9.4 Notion — 多视图（Board/Calendar/Gallery）

Notion 的"数据库"本质就是 Content Type，但它有多种视图：

```
Table View    → 表格（和当前 Admin UI 一样）
Board View    → 看板（按 status 拖拽排序）
Gallery View  → 卡片（按封面图展示）
Calendar View → 日历（按日期排列）
Timeline View → 甘特图
```

**借鉴方向：** 当前 Admin UI 只有 Table View。加 Board View 成本很低（前端拖拽组件 + 按 status 分组），但对用户体验提升巨大 — 拖拽改状态（draft → published）。Calendar View 适合按 `published_at` 排列内容。

**优先级：** P0

### 9.5 Directus — Flow 自动化

Directus Flow 是可视化的自动化规则引擎：

```
当文章状态变为 published
  → 调用 Webhook（通知搜索引擎）
  → 发送邮件（通知作者）
  → 更新缓存
```

**借鉴方向：** 已有 Webhook + Worker + Event 系统，但分散。整合成"触发器 → 动作链"的配置化界面：

```toml
[[flow]]
trigger = "content_type.posts.after_update"
condition = "status == 'published'"
actions = [
  { type = "webhook", url = "https://..." },
  { type = "email", to = "author", template = "published" },
  { type = "cache_purge", key = "cms:posts:*" },
]
```

用户不需要写代码就能编排自动化流程。

**优先级：** P1

### 9.6 Craft CMS — Section + Entry Type

Craft CMS 的内容模型更灵活：

```
Section（频道）
  ├── Entry Type A（文章：标题+正文+标签）
  ├── Entry Type B（视频：标题+视频URL+时长）
  └── Entry Type C（链接：标题+URL+描述）
```

同一个 Section 下可以有多种 Entry Type，比 Strapi 的单一 Content Type 更灵活。

**借鉴方向：** 类似 Strapi 的 Dynamic Zone。可以在 Content Type 里加字段类型 `entry_types`，允许同一条路由下渲染不同类型的条目。

**优先级：** P3 — 和 Dynamic Zone 功能类似

### 9.7 Keystone.js — 字段级 auto_generate

```typescript
fields: {
  slug: {
    type: 'text',
    hooks: {
      resolveInput: ({ resolvedData }) => slugify(resolvedData.title),
    }
  }
}
```

**借鉴方向：** 当前 slug 生成在 service 层硬编码。如果做成字段级配置，Content Type 就更通用：

```toml
[fields.slug]
type = "text"
auto_generate = "slug"      # 从 title 字段自动生成
auto_source = "title"

[fields.published_at]
type = "text"
auto_generate = "timestamp"  # 状态变为 published 时自动填充
auto_condition = "status == 'published'"
```

**优先级：** P1
