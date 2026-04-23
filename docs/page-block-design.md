# Page + Block 设计方案

> 版本：v1.0 · 日期：2026-04-23

## 一、目标

在现有博客系统基础上，增加 **页面（Page）** 和 **块（Block）** 支持，使其能够构建绝大部分企业官网场景：

- 首页（Hero + 统计 + 服务介绍 + 客户评价 + CTA）
- 关于我们（公司介绍 + 发展历程 + 团队成员）
- 产品/服务（特性展示 + 定价方案）
- 联系我们（表单 + 地图）
- 案例/作品集（画廊 + 评价）
- FAQ、博客、新闻等

### 设计参考

| CMS | 块系统 | 借鉴点 |
|---|---|---|
| WordPress Gutenberg | 块编辑器 + 可复用块 | 块类型体系、拖拽排序 |
| Strapi Dynamic Zones | 组件化 JSON 字段 | `tagged union` 数据结构 |
| Payload CMS Blocks Layout | 自定义块 + 布局 | 嵌套栏目（Columns） |
| Notion | 分栏 + 嵌套 | 多栏布局递归嵌套 |

## 二、架构概览

```
┌───────────────────────────────────────────┐
│                  Page                      │
│                                           │
│  title / slug / status / template / SEO   │
│                                           │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐  │
│  │ Block 0  │ │ Block 1  │ │ Block N  │  │
│  │  (hero)  │ │ (text)   │ │ (gallery)│  │
│  │ sort=0   │ │ sort=1   │ │ sort=N   │  │
│  └──────────┘ └──────────┘ └──────────┘  │
│                                           │
│  blocks: JSON ARRAY（有序）               │
└───────────────────────────────────────────┘

┌───────────────────────────────────────────┐
│            Reusable Block                 │
│                                           │
│  name / type / content(JSON) / global     │
│                                           │
│  → 可被任意 Page 引用：                   │
│    { "type": "reusable", "ref_id": "..." }│
└───────────────────────────────────────────┘
```

### 为什么原生实现（而非 Content Type）

| 维度 | Content Type 方案 | 原生方案（推荐） |
|---|---|---|
| 公开路由 | 需要额外插件/中间件拦截 slug | 直接注册 `/pages/{slug}` 路由 |
| Block 嵌套 | JSON 字段无类型校验 | Rust enum 强类型校验 |
| SEO 元数据 | 需自定义字段 | 内建 meta_title / meta_description / og_image |
| 模板系统 | 无法在 Rust 层控制 | 可注册自定义模板渲染器 |
| 版本控制 | content_revisions 表已支持 | 复用已有 revision 系统 |
| 层级页面 | 需手动实现 parent_id | 内建 hierarchy |
| 排序控制 | 无内建支持 | sort_order 字段 |

## 三、数据库设计

### Migration `023_create_pages.sql`

```sql
-- ============================================================
-- 页面表
-- ============================================================
CREATE TABLE IF NOT EXISTS pages (
    id               TEXT PRIMARY KEY,           -- UUID v7
    tenant_id        TEXT NOT NULL DEFAULT 'default',
    title            TEXT NOT NULL,
    slug             TEXT NOT NULL,

    -- 内容模式（二选一，或并存）
    content          TEXT,                       -- 简单模式：纯 markdown
    blocks           TEXT,                       -- 块模式：JSON array

    -- SEO
    meta_title       TEXT,
    meta_description TEXT,
    og_image         TEXT,

    -- 布局
    template         TEXT NOT NULL DEFAULT 'default',  -- default / full / landing / contact / ...

    -- 层级（支持树形页面）
    parent_id        TEXT REFERENCES pages(id) ON DELETE SET NULL,
    sort_order       INTEGER NOT NULL DEFAULT 0,

    -- 状态
    status           TEXT NOT NULL DEFAULT 'draft',    -- draft / published / archived
    author_id        TEXT NOT NULL REFERENCES users(id),

    -- 封面图
    cover_image      TEXT,

    -- 时间戳
    published_at     TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now')),

    UNIQUE(tenant_id, slug)
);

CREATE INDEX idx_pages_slug      ON pages(tenant_id, slug);
CREATE INDEX idx_pages_status    ON pages(tenant_id, status);
CREATE INDEX idx_pages_parent    ON pages(tenant_id, parent_id);
CREATE INDEX idx_pages_author    ON pages(author_id);

-- ============================================================
-- 可复用块（全局组件，跨页面引用）
-- ============================================================
CREATE TABLE IF NOT EXISTS reusable_blocks (
    id          TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL DEFAULT 'default',
    name        TEXT NOT NULL,          -- 显示名称，如 "公司介绍"
    type        TEXT NOT NULL,          -- hero / text / gallery / ...
    content     TEXT NOT NULL,          -- JSON：该类型块的数据
    description TEXT,                   -- 备注
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_reusable_blocks_tenant ON reusable_blocks(tenant_id);
```

### 表关系

```
pages.author_id → users.id
pages.parent_id → pages.id (自引用，支持树形)

reusable_blocks 独立表，通过 blocks JSON 中的 { type: "reusable", ref_id } 引用
```

## 四、Block 类型系统（Rust Enum）

### 核心设计

每个 Block 是一个 `tagged union`（`serde` 的 `#[serde(tag = "type")]`），序列化为 JSON 时自动携带 `type` 字段：

```json
[
  { "type": "hero", "title": "...", "subtitle": "..." },
  { "type": "richtext", "content": "# Hello" },
  { "type": "stats", "items": [...] },
  { "type": "reusable", "ref_id": "..." }
]
```

### 类型定义

```rust
// src/models/page.rs

use serde::{Deserialize, Serialize};

/// 页面中的一个块
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PageBlock {
    // ── 内容类 ──

    /// 大图横幅（首页 Hero）
    Hero {
        title: String,
        subtitle: Option<String>,
        background_image: Option<String>,
        cta_text: Option<String>,      // 按钮文字
        cta_url: Option<String>,       // 按钮链接
        alignment: Option<String>,     // left / center / right
        overlay: Option<bool>,         // 暗色遮罩
        height: Option<String>,        // sm / md / lg / full
    },

    /// 富文本（Markdown）
    Richtext {
        content: String,
    },

    /// 单张图片
    Image {
        url: String,
        alt: Option<String>,
        caption: Option<String>,
        link: Option<String>,
        width: Option<String>,         // full / half / third / quarter
    },

    /// 图片画廊
    Gallery {
        images: Vec<GalleryImage>,
        columns: Option<u32>,          // 2 / 3 / 4
        gap: Option<String>,           // sm / md / lg
    },

    /// 视频
    Video {
        url: String,
        provider: Option<String>,      // youtube / bilibili / vimeo / custom
        title: Option<String>,
        autoplay: Option<bool>,
    },

    // ── 转化类 ──

    /// 行动号召（Call to Action）
    Cta {
        title: String,
        description: Option<String>,
        button_text: String,
        button_url: String,
        style: Option<String>,         // primary / outline / banner
        background_image: Option<String>,
    },

    /// 客户评价
    Testimonial {
        items: Vec<TestimonialItem>,
        layout: Option<String>,        // carousel / grid / masonry
    },

    // ── 信息展示类 ──

    /// 常见问题
    Faq {
        items: Vec<FaqItem>,
    },

    /// 数据统计
    Stats {
        items: Vec<StatItem>,
        background: Option<String>,    // light / dark / primary / image
    },

    /// 时间线
    Timeline {
        items: Vec<TimelineItem>,
    },

    /// 团队成员
    Team {
        members: Vec<TeamMember>,
        columns: Option<u32>,          // 2 / 3 / 4
    },

    /// 价格表
    Pricing {
        plans: Vec<PricingPlan>,
        highlight_index: Option<usize>,  // 高亮哪个方案
    },

    // ── 交互类 ──

    /// 联系表单
    ContactForm {
        email_to: Option<String>,
        fields: Option<Vec<FormFieldDef>>,
        submit_text: Option<String>,
        success_message: Option<String>,
    },

    /// 地图
    Map {
        address: Option<String>,
        lat: Option<f64>,
        lng: Option<f64>,
        zoom: Option<u32>,
    },

    // ── 排版类 ──

    /// 代码块
    Code {
        code: String,
        language: Option<String>,
        show_line_numbers: Option<bool>,
    },

    /// 引用
    Quote {
        content: String,
        author: Option<String>,
        source: Option<String>,
    },

    /// 分隔线
    Divider {
        style: Option<String>,         // solid / dashed / dotted / space
    },

    /// 间距
    Spacer {
        height: Option<String>,        // sm / md / lg / xl 或 "48px"
    },

    // ── 布局类 ──

    /// 多栏布局（可递归嵌套子块）
    Columns {
        columns: Vec<ColumnDef>,
        gap: Option<String>,
    },

    /// 自定义 HTML（仅管理员可用）
    Html {
        content: String,
    },

    /// 引用可复用块
    Reusable {
        ref_id: String,
    },
}
```

### 辅助结构体

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GalleryImage {
    pub url: String,
    pub alt: Option<String>,
    pub caption: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestimonialItem {
    pub quote: String,
    pub author: String,
    pub company: Option<String>,
    pub avatar: Option<String>,
    pub rating: Option<u32>,           // 1-5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaqItem {
    pub question: String,
    pub answer: String,
    pub is_open: Option<bool>,         // 默认展开
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatItem {
    pub label: String,
    pub value: String,
    pub suffix: Option<String>,        // + / 万 / % / +
    pub icon: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineItem {
    pub date: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamMember {
    pub name: String,
    pub role: Option<String>,
    pub avatar: Option<String>,
    pub bio: Option<String>,
    pub social_links: Option<Vec<SocialLink>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialLink {
    pub platform: String,              // twitter / github / linkedin / email / website
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingPlan {
    pub name: String,
    pub price: String,
    pub period: Option<String>,        // 月 / 年 / 次
    pub description: Option<String>,
    pub features: Vec<String>,
    pub button_text: Option<String>,
    pub button_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormFieldDef {
    pub name: String,
    pub label: String,
    pub field_type: String,            // text / email / phone / textarea / select
    pub required: Option<bool>,
    pub options: Option<Vec<String>>,  // select 类型的选项
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub width: Option<String>,         // "1/3" "1/2" "2/3" "1/4" "3/4"
    pub blocks: Vec<PageBlock>,        // 递归嵌套
}
```

## 五、API 设计

### 公开 API

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/pages` | 页面列表（仅 published） | 无 |
| GET | `/pages/{slug}` | 按 slug 获取页面（含解析后的 blocks） | 无 |
| GET | `/pages/sitemap` | 站点地图数据（所有已发布页 slug + 更新时间） | 无 |

### 管理 API

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/admin/pages` | 列表（全部状态，支持筛选） | Author |
| GET | `/admin/pages/{id}` | 详情 | Author |
| POST | `/admin/pages` | 创建 | Author |
| PUT | `/admin/pages/{id}` | 更新 | Author |
| DELETE | `/admin/pages/{id}` | 删除 | Author |
| PUT | `/admin/pages/{id}/status` | 发布 / 下线 / 归档 | Author |
| PUT | `/admin/pages/reorder` | 批量调整排序 | Author |

### 可复用块

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/admin/reusable-blocks` | 列表 | Author |
| POST | `/admin/reusable-blocks` | 创建 | Author |
| PUT | `/admin/reusable-blocks/{id}` | 更新 | Author |
| DELETE | `/admin/reusable-blocks/{id}` | 删除 | Author |

### 块元数据

| 方法 | 路径 | 说明 | 认证 |
|---|---|---|---|
| GET | `/admin/block-templates` | 返回所有可用块类型定义 | Author |

### API 响应格式

遵循项目约定（`code: 0` 表示成功）：

```json
// GET /pages/about
{
  "code": 0,
  "message": "ok",
  "data": {
    "id": "01923abc...",
    "title": "关于我们",
    "slug": "about",
    "status": "published",
    "template": "default",
    "content": null,
    "blocks": [
      { "type": "hero", "title": "关于我们", "subtitle": "...", "height": "lg" },
      { "type": "richtext", "content": "公司成立于..." },
      { "type": "stats", "items": [...] },
      { "type": "timeline", "items": [...] }
    ],
    "meta_title": "关于我们 - XX公司",
    "meta_description": "...",
    "og_image": "/uploads/...",
    "parent_id": null,
    "sort_order": 0,
    "author_id": "...",
    "cover_image": null,
    "published_at": "2026-04-20T08:00:00Z",
    "created_at": "2026-04-19T10:00:00Z",
    "updated_at": "2026-04-20T08:00:00Z"
  }
}
```

```json
// GET /pages?page=1&page_size=10
{
  "code": 0,
  "message": "ok",
  "data": {
    "items": [...],
    "total": 15,
    "page": 1,
    "page_size": 10
  }
}
```

## 六、前端结构

### Admin 页面

```
web/src/app/admin/pages/
  page.tsx                        — 页面列表（表格 + 状态筛选 + 搜索）
  new/page.tsx                    — 新建页面
  [id]/edit/page.tsx              — 编辑页面

web/src/app/admin/reusable-blocks/
  page.tsx                        — 可复用块列表

web/src/components/admin/
  block-editor/
    index.tsx                     — 块编辑器主组件（排序 + 添加 + 删除）
    block-wrapper.tsx             — 单个块的外壳（拖拽手柄 + 删除 + 排序按钮）
    add-block-menu.tsx            — 添加块的弹出菜单
    preview/
      index.tsx                   — 块预览渲染器
  blocks/
    hero-editor.tsx               — Hero 块编辑表单
    richtext-editor.tsx           — 富文本块编辑（复用 MarkdownEditor）
    image-editor.tsx              — 图片块编辑（复用媒体上传）
    gallery-editor.tsx            — 画廊块编辑
    video-editor.tsx              — 视频块编辑
    cta-editor.tsx                — CTA 块编辑
    testimonial-editor.tsx        — 评价块编辑
    faq-editor.tsx                — FAQ 块编辑
    stats-editor.tsx              — 统计块编辑
    timeline-editor.tsx           — 时间线块编辑
    team-editor.tsx               — 团队块编辑
    pricing-editor.tsx            — 价格块编辑
    contact-form-editor.tsx       — 联系表单块编辑
    map-editor.tsx                — 地图块编辑
    code-editor.tsx               — 代码块编辑
    quote-editor.tsx              — 引用块编辑
    divider-editor.tsx            — 分隔线块编辑
    spacer-editor.tsx             — 间距块编辑
    columns-editor.tsx            — 多栏布局块编辑
    html-editor.tsx               — 自定义 HTML 块编辑
    reusable-editor.tsx           — 可复用块引用编辑
```

### 公开页面

```
web/src/app/(public)/pages/
  [slug]/page.tsx                 — 公开页面渲染

web/src/components/public/
  block-renderer.tsx              — 块渲染调度器（根据 type 渲染对应组件）
  blocks/
    hero-block.tsx                — Hero 渲染
    richtext-block.tsx            — 富文本渲染
    image-block.tsx               — 图片渲染
    gallery-block.tsx             — 画廊渲染
    video-block.tsx               — 视频渲染
    cta-block.tsx                 — CTA 渲染
    testimonial-block.tsx         — 评价渲染
    faq-block.tsx                 — FAQ 渲染
    stats-block.tsx               — 统计渲染
    timeline-block.tsx            — 时间线渲染
    team-block.tsx                — 团队渲染
    pricing-block.tsx             — 价格渲染
    contact-form-block.tsx        — 联系表单渲染
    map-block.tsx                 — 地图渲染
    code-block.tsx                — 代码渲染
    quote-block.tsx               — 引用渲染
    divider-block.tsx             — 分隔线渲染
    spacer-block.tsx              — 间距渲染
    columns-block.tsx             — 多栏布局渲染（递归）
    html-block.tsx                — 自定义 HTML 渲染
```

### 块编辑器交互

```
┌─ Page Editor ───────────────────────────────────────┐
│                                                     │
│  Title:  [关于我们                              ]   │
│  Slug:   [about-us                             ]   │
│  Template: [default ▾]    Status: [draft ▾]         │
│                                                     │
│  ── SEO ─────────────────────────────────────────── │
│  Meta Title: [关于我们 - XX公司                ]    │
│  Meta Desc:  [专业XX服务，值得信赖...            ]  │
│  OG Image:   [上传图片]                             │
│                                                     │
│  ── Blocks ──────────────────────────────────────── │
│                                                     │
│  ┌─ Hero ──────────────────────────── [↑][↓][✕] ┐  │
│  │  Title:    [欢迎来到我们的公司]              │  │
│  │  Subtitle: [专业服务，值得信赖]              │  │
│  │  BG Image: [📷 hero-bg.jpg]                 │  │
│  │  CTA:      [了解更多] → [/contact]           │  │
│  │  Height:   [lg ▾]  Overlay: [✓]              │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  ┌─ Richtext ──────────────────────── [↑][↓][✕] ┐  │
│  │  [富文本编辑器 / Markdown 编辑器...]         │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  ┌─ Stats ─────────────────────────── [↑][↓][✕] ┐  │
│  │  [10年经验] [500+客户] [99%满意] [24/7支持]  │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  ┌─ Team ──────────────────────────── [↑][↓][✕] ┐  │
│  │  👤 张三 CEO     👤 李四 CTO                  │  │
│  │  👤 王五 设计    [+ 添加成员]                 │  │
│  └──────────────────────────────────────────────┘  │
│                                                     │
│  [+ 添加块 ▾]                                       │
│  ┌─────────────────────────────────────────┐        │
│  │  🖼 Hero      📝 文本      📷 图片      │        │
│  │  🎬 视频      📢 CTA       ⭐ 评价      │        │
│  │  ❓ FAQ       📊 统计      ⏱ 时间线     │        │
│  │  👥 团队      💰 价格      📮 表单      │        │
│  │  📍 地图      📋 代码      💬 引用      │        │
│  │  ➖ 分隔线    ↕️ 间距      📦 栏目布局   │        │
│  │  🔄 可复用    🌐 自定义HTML              │        │
│  └─────────────────────────────────────────┘        │
│                                                     │
│  ── 侧栏 ─────────────────────────────────────────  │
│  Parent:  [无（顶级页面） ▾]                        │
│  Sort:    [0]                                       │
│  Cover:   [上传封面图]                               │
│                                                     │
│            [保存草稿]    [发布]                       │
└─────────────────────────────────────────────────────┘
```

### 公开页面渲染流程

```
用户访问 /about
       ↓
Next.js [slug] page.tsx
       ↓
GET /api/v1/pages/about
       ↓
返回 { title, template, blocks: [...], meta_title, ... }
       ↓
设置 <title> meta 信息
       ↓
BlockRenderer 遍历 blocks 数组
       ↓
根据 block.type 匹配渲染组件
       ↓
若 type = "reusable" → 请求 GET /api/v1/admin/reusable-blocks/{ref_id}
       ↓
若 type = "columns" → 递归渲染子块
       ↓
组合渲染完整页面
```

## 七、20 种块类型 vs 企业网站场景

| 块类型 | 典型页面 | 用途 |
|---|---|---|
| Hero | 首页、产品页 | 大图横幅，核心标语 + CTA 按钮 |
| Richtext | 所有页面 | 通用富文本内容 |
| Image | 产品介绍、案例 | 单张图片展示 |
| Gallery | 作品集、案例 | 图片画廊/灯箱效果 |
| Video | 首页、产品演示 | 视频 embed |
| Cta | 首页、服务页 | "立即咨询""免费试用" |
| Testimonial | 首页、关于我们 | 客户评价/好评 |
| Faq | FAQ 页 | 常见问题折叠面板 |
| Stats | 首页、关于我们 | 数据亮点（年限、客户数） |
| Timeline | 关于我们 | 公司发展历程 |
| Team | 关于我们 | 团队成员卡片 |
| Pricing | 产品/服务 | 定价方案对比 |
| ContactForm | 联系我们 | 表单提交 |
| Map | 联系我们 | 公司地址 |
| Code | 技术文档 | 代码示例 |
| Quote | 关于我们、首页 | 客户语录 |
| Divider | 任意页面 | 视觉分隔 |
| Spacer | 任意页面 | 间距控制 |
| Columns | 任意页面 | 多栏布局（图文并排等） |
| Html | 任意页面 | 第三方组件嵌入 |
| Reusable | 任意页面 | 跨页面复用组件（页脚公告等） |

## 八、后端文件结构

```
src/
  models/
    page.rs                  — Page / PageBlock / 辅助结构体 + 查询函数
  services/
    page.rs                  — 页面业务逻辑（slug生成、状态管理、block校验）
  handlers/
    page.rs                  — API handlers（CRUD + 状态变更 + 排序）
  server.rs                  — 注册路由（追加 /pages 相关路由）
```

### 路由注册

```rust
// src/server.rs — 在 posts 路由之后添加

.route("/pages", get(page::list).post(page::create))
.route("/pages/sitemap", get(page::sitemap))
.route("/pages/{slug}", get(page::get_by_slug))
.route("/admin/pages", get(page::admin_list))
.route("/admin/pages/{id}", get(page::admin_get).put(page::update).delete(page::delete))
.route("/admin/pages/{id}/status", put(page::update_status))
.route("/admin/pages/reorder", put(page::reorder))
.route("/admin/reusable-blocks", get(page::list_reusable).post(page::create_reusable))
.route("/admin/reusable-blocks/{id}", put(page::update_reusable).delete(page::delete_reusable))
.route("/admin/block-templates", get(page::block_templates))
```

## 九、实现步骤

| 步骤 | 内容 | 预估 |
|---|---|---|
| 1 | Migration `023_create_pages.sql` | 0.5 天 |
| 2 | `src/models/page.rs` — 模型 + 查询函数 | 1 天 |
| 3 | `src/services/page.rs` — 业务逻辑 | 0.5 天 |
| 4 | `src/handlers/page.rs` — API handlers | 0.5 天 |
| 5 | `src/server.rs` — 注册路由 | 0.5 天 |
| 6 | 后端测试（clippy + fmt + test） | 0.5 天 |
| 7 | 前端 admin 页面编辑器（块编辑器 + 各块编辑组件） | 2-3 天 |
| 8 | 前端公开页面渲染器（各块渲染组件） | 1 天 |
| 9 | i18n + admin sidebar 更新 | 0.5 天 |
| **总计** | | **约 7 天** |

## 十、扩展方向（后续迭代）

- **主题系统**：不同 template 对应不同的块渲染样式
- **块模板市场**：预设行业模板（科技公司/餐饮/教育...）
- **拖拽排序**：前端块编辑器支持 drag & drop
- **块动画**：进入动画（fade-in / slide-up）配置
- **A/B 测试**：同一页面多版本 blocks 对比
- **定时发布**：scheduled_at 字段 + Cron 自动切换状态
- **多语言页面**：关联不同 locale 的 page 记录
