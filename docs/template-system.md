# 模板系统设计

> 借鉴 WordPress 的模板层级 + Gutenberg 块编辑器，适配无头 CMS 架构。
> 后端存储模板配置和 Block 结构化数据，前端负责渲染。

## 1. 核心概念

| 概念 | 说明 | 类比 WordPress |
|------|------|---------------|
| **Block** | 最小内容单元（段落、标题、图片、代码等），以 JSON 存储 | Gutenberg Block |
| **Block 字段** | Content Type 的 `content` 字段类型为 `blocks`，存储 Block 数组 | `post_content`（富文本） |
| **Layout** | 页面骨架，定义有哪些区域（regions） | `header.php` + `footer.php` + `sidebar.php` |
| **Page Template** | 将 Layout 绑定到特定 Content Type 或单篇文章 | `single-post.php` / `page.php` |
| **Region** | Layout 中的一个插槽，可配置 Block 列表 | `get_sidebar()` / 动态 Widget 区域 |
| **模板层级** | 前端根据路由自动匹配最具体的模板 | WordPress Template Hierarchy |

## 2. 架构总览

```
┌─ 后端 Rust ──────────────────────────────────────────────┐
│                                                           │
│  content_types/post.toml                                  │
│    fields.content.type = "blocks"   ← 块编辑器字段类型     │
│                                                           │
│  layouts 表                                               │
│    id | name    | regions                                 │
│    L1 | default | ["header","nav","content","sidebar",    │
│    |         |    "footer"]                                │
│    L2 | full   | ["header","content","footer"]             │
│                                                           │
│  page_templates 表                                        │
│    id | name          | layout | content_type             │
│    T1 | Post Detail   | L1     | post                     │
│    T2 | Post Full     | L2     | post                     │
│    T3 | Page Landing  | L2     | page                     │
│                                                           │
│  template_regions 表 (每个区域放哪些 block)                │
│    template=T1, region="sidebar"                          │
│      → [{"type":"recent_posts","limit":5}]                │
│                                                           │
└───────────────────────────────────────────────────────────┘

┌─ 前端 Next.js ────────────────────────────────────────────┐
│                                                           │
│  TemplateProvider                                         │
│    ├─ 查询当前路由 → 匹配 page_template                   │
│    ├─ 加载 layout → 渲染 regions                          │
│    └─ 每个 region → 渲染配置的 blocks                     │
│                                                           │
│  BlockRegistry (前端)                                     │
│    "paragraph"    → <ParagraphBlock />                    │
│    "heading"      → <HeadingBlock />                      │
│    "image"        → <ImageBlock />                        │
│    "code"         → <CodeBlock />                         │
│    "gallery"      → <GalleryBlock />                      │
│    "recent_posts" → <RecentPostsBlock />                  │
│    "newsletter"   → <NewsletterBlock />                   │
│    ...插件可注册新 block...                                │
│                                                           │
│  LayoutRegistry (前端)                                    │
│    "default" → Header + Nav + Content + Sidebar + Footer  │
│    "full"    → Header + Content + Footer                  │
│    "landing" → 全自定义                                    │
│                                                           │
└───────────────────────────────────────────────────────────┘
```

## 3. Block 编辑器

### 3.1 Block 数据结构

内容不再是纯 Markdown 文本，而是结构化的 Block 数组。

`content` 字段存储格式：

```jsonc
[
  { "type": "heading", "attrs": { "level": 2 }, "content": "为什么选择我们" },
  { "type": "paragraph", "content": "我们提供最优质的服务..." },
  {
    "type": "image",
    "attrs": { "src": "/uploads/team.jpg", "alt": "团队照片", "width": 800 }
  },
  {
    "type": "columns",
    "attrs": { "count": 3 },
    "children": [
      [{ "type": "card", "attrs": { "title": "快速", "icon": "zap" } }],
      [{ "type": "card", "attrs": { "title": "安全", "icon": "shield" } }],
      [{ "type": "card", "attrs": { "title": "可靠", "icon": "check" } }]
    ]
  },
  { "type": "code", "attrs": { "language": "rust" }, "content": "fn main() {}" },
  { "type": "newsletter" }   // 自定义 block（插件注册）
]
```

### 3.2 Block 类型定义

每个 Block 统一结构：

```typescript
interface Block {
  type: string;                    // block 类型标识
  content?: string;                // 文本内容（paragraph / heading / code）
  attrs?: Record<string, unknown>; // 属性（src / level / language 等）
  children?: Block[][];            // 嵌套子 block（columns 等）
}
```

### 3.3 内置 Block 类型

| Block 类型 | attrs | 说明 |
|-----------|-------|------|
| `paragraph` | — | 段落文本，支持内联标记（粗体、链接等） |
| `heading` | `level: 1-6` | 标题 |
| `image` | `src, alt, width, caption?` | 图片 |
| `image_gallery` | `images: [{src, alt}]` | 图片画廊 |
| `code` | `language` | 代码块 |
| `quote` | `author?, source?` | 引用 |
| `list` | `ordered: bool` | 有序/无序列表 |
| `columns` | `count: 2-4` | 多栏布局，children 是二维数组 |
| `divider` | — | 分隔线 |
| `embed` | `url, provider, width, height` | 第三方嵌入（YouTube、Tweet 等） |
| `table` | `rows, headers: bool` | 表格 |
| `callout` | `variant: info/warning/danger/tip` | 提示框 |
| `card` | `title, icon?, image?` | 卡片（常用于 columns 内） |
| `raw_html` | — | 原始 HTML（仅管理员可用） |

### 3.4 前端 Block 渲染

```tsx
// components/BlockRenderer.tsx

interface BlockProps {
  type: string;
  content?: string;
  attrs?: Record<string, unknown>;
  children?: Block[][];
}

const blockRegistry: Record<string, React.ComponentType<BlockProps>> = {
  heading: HeadingBlock,
  paragraph: ParagraphBlock,
  image: ImageBlock,
  columns: ColumnsBlock,
  code: CodeBlock,
  gallery: GalleryBlock,
  newsletter: NewsletterBlock,
  // 插件运行时动态注册更多...
};

export function BlockRenderer({ blocks }: { blocks: Block[] }) {
  return (
    <>
      {blocks.map((block, i) => {
        const Component = blockRegistry[block.type];
        if (!Component) return <UnknownBlock key={i} type={block.type} />;
        return <Component key={i} {...block} />;
      })}
    </>
  );
}
```

插件可注册自定义 Block：

```tsx
// 插件注册 block
BlockRegistry.register("newsletter", NewsletterBlock);
BlockRegistry.register("pricing_table", PricingTableBlock);
```

### 3.5 后端兼容

现有 `content` 字段是 Markdown 文本。迁移策略：

1. 新增字段类型 `blocks`，新 Content Type 使用
2. 现有 `richtext` 字段保持 Markdown 存储不变
3. 前端 `BlockRenderer` 遇到 `string` 类型 content 自动走 Markdown 渲染
4. Admin 编辑器：`richtext` 用 Markdown 编辑器，`blocks` 用 Block 编辑器

## 4. Layout 与 Page Template

### 4.1 模板匹配规则（Template Resolution）

模仿 WordPress 模板层级，前端根据路由自动匹配模板：

```
请求 /posts/hello-world
  1. 查找 page_template: content_type=post, slug="hello-world" → 没找到
  2. 查找 page_template: content_type=post, slug=null  → 找到 "Post Full"
  3. 加载 layout → Header + Content + Footer
  4. Content 区域渲染 post.content 的 blocks

请求 /pages/about
  1. 查找 page_template: content_type=page, slug="about" → 找到 "Page Landing"
  2. 加载对应 layout
  3. 渲染 region 配置的 blocks + 文章 content

请求 / (首页)
  1. 查找 page_template: content_type=null, slug=null → 找到 "Homepage"
  2. 加载 landing layout
  3. 所有 region 均由 template region_config 定义
```

匹配优先级（从高到低）：

| 优先级 | 匹配条件 | 示例 |
|--------|---------|------|
| 1 | `content_type` + `slug` 精确匹配 | `post` + `hello-world` |
| 2 | `content_type` + `slug=null` 默认模板 | `post` + null |
| 3 | 全局默认 | null + null |

### 4.2 前端 TemplateProvider

```tsx
// components/TemplateProvider.tsx

async function resolveTemplate(
  contentType: string | null,
  slug: string | null,
): Promise<PageTemplate | null> {
  const templates = await api.get<Template[]>("/templates");
  
  // 优先级 1: content_type + slug 精确匹配
  let matched = templates.find(
    (t) => t.content_type === contentType && t.slug === slug,
  );
  
  // 优先级 2: content_type 默认模板
  if (!matched) {
    matched = templates.find(
      (t) => t.content_type === contentType && !t.slug,
    );
  }
  
  // 优先级 3: 全局默认
  if (!matched) {
    matched = templates.find((t) => !t.content_type && !t.slug);
  }
  
  return matched ?? null;
}

export function TemplateProvider({
  template,
  data,
  children,
}: {
  template: PageTemplate;
  data?: Record<string, unknown>;
  children?: React.ReactNode;
}) {
  const layout = layouts[template.layout_id];
  
  return (
    <layout.Component>
      {layout.regions.map((region) => (
        <Region key={region} name={region}>
          {region === "content" && children}
          {template.region_config[region]?.map((block, i) => (
            <BlockRenderer key={i} blocks={[block]} />
          ))}
        </Region>
      ))}
    </layout.Component>
  );
}
```

## 5. 数据库设计

### 5.1 layouts 表

```sql
CREATE TABLE layouts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    regions TEXT NOT NULL,           -- JSON: ["header","nav","content","sidebar","footer"]
    preview TEXT,                    -- 预览图 URL（可选）
    is_system BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### 5.2 page_templates 表

```sql
CREATE TABLE page_templates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    layout_id TEXT NOT NULL REFERENCES layouts(id) ON DELETE CASCADE,
    content_type TEXT,               -- null = 全局/首页
    slug TEXT,                       -- null = 该 content_type 的默认模板
    region_config TEXT NOT NULL DEFAULT '{}', -- JSON: region → block 配置数组
    is_system BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(content_type, slug)
);
```

### 5.3 预置数据

```sql
INSERT INTO layouts (id, name, regions, is_system, created_at, updated_at) VALUES
    ('layout-default', 'Default',  '["header","nav","content","sidebar","footer"]', 1, datetime('now'), datetime('now')),
    ('layout-full',    'Full Width', '["header","content","footer"]',                1, datetime('now'), datetime('now')),
    ('layout-landing', 'Landing',    '["content"]',                                  1, datetime('now'), datetime('now'));

INSERT INTO page_templates (id, name, layout_id, content_type, slug, region_config, is_system, created_at, updated_at) VALUES
    ('tpl-post', 'Post Detail', 'layout-default', 'post', NULL,
     '{"sidebar":[{"type":"recent_posts","attrs":{"limit":5}},{"type":"tags"}]}',
     1, datetime('now'), datetime('now')),
    ('tpl-page', 'Page', 'layout-full', 'page', NULL, '{}',
     1, datetime('now'), datetime('now')),
    ('tpl-home', 'Homepage', 'layout-landing', NULL, NULL,
     '{"content":[{"type":"hero"},{"type":"featured_posts","attrs":{"limit":3}},{"type":"newsletter"}]}',
     1, datetime('now'), datetime('now'));
```

## 6. API 设计

### 6.1 Layout CRUD

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/admin/layouts` | 列出所有布局 |
| GET | `/api/v1/admin/layouts/:id` | 获取单个布局 |
| POST | `/api/v1/admin/layouts` | 创建布局 |
| PUT | `/api/v1/admin/layouts/:id` | 更新布局 |
| DELETE | `/api/v1/admin/layouts/:id` | 删除布局（非 system） |

### 6.2 Page Template CRUD

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/admin/templates` | 列出所有模板 |
| GET | `/api/v1/admin/templates/:id` | 获取单个模板 |
| POST | `/api/v1/admin/templates` | 创建模板 |
| PUT | `/api/v1/admin/templates/:id` | 更新模板（改布局、配 region） |
| DELETE | `/api/v1/admin/templates/:id` | 删除模板（非 system） |

### 6.3 模板解析（前端用）

| Method | Path | 说明 |
|--------|------|------|
| GET | `/api/v1/templates/resolve?content_type=post&slug=hello` | 解析匹配的模板 |
| GET | `/api/v1/templates/resolve` | 首页模板 |

响应示例：

```json
{
  "template": {
    "id": "tpl-post",
    "name": "Post Detail",
    "layout": {
      "id": "layout-default",
      "name": "Default",
      "regions": ["header", "nav", "content", "sidebar", "footer"]
    },
    "region_config": {
      "sidebar": [
        { "type": "recent_posts", "attrs": { "limit": 5 } },
        { "type": "tags" }
      ]
    }
  }
}
```

## 7. Content Type Schema 扩展

`blocks` 字段类型加到 Content Type 定义中：

```toml
# content_types/post.toml

[fields.content]
type = "blocks"           # 新字段类型
label = "正文"
required = true

[fields.excerpt]
type = "text"
max_length = 500
label = "摘要"
```

`blocks` 字段在数据库中存储为 `TEXT`（JSON 数组），与 `json` 字段类型存储方式相同，
区别在于前端使用 Block 编辑器而非 JSON 文本框。

## 8. 前端 Admin 页面

### 8.1 Block 编辑器组件

核心编辑组件，用于 `blocks` 类型字段的输入：

```
components/admin/block-editor/
├── BlockEditor.tsx          # 编辑器主体
├── BlockToolbar.tsx         # 顶部工具栏（插入 block / 拖拽排序）
├── blocks/                  # 每个 block 类型的编辑态组件
│   ├── ParagraphEdit.tsx
│   ├── HeadingEdit.tsx
│   ├── ImageEdit.tsx
│   ├── CodeEdit.tsx
│   ├── ColumnsEdit.tsx
│   └── ...
└── BlockSelector.tsx        # "+" 按钮弹出的 block 选择面板
```

### 8.2 模板管理页面

```
web/src/app/admin/templates/
├── page.tsx                 # 模板列表 + Layout 列表
├── [id]/page.tsx            # 模板编辑：选 Layout + 配置每个 Region 的 blocks
└── layouts/
    └── page.tsx             # Layout 管理（增删 regions）
```

### 8.3 Admin 侧边栏

layout.tsx 的 menuItems 增加：

```ts
{ label: "Templates", href: "/admin/templates", icon: LayoutTemplate },
```

## 9. 与现有系统集成

### 9.1 Content Type 字段类型扩展

`FieldType` 枚举新增 `Blocks`：

```rust
// src/content_type/schema.rs
pub enum FieldType {
    Text,
    RichText,
    // ... 现有类型
    Blocks,   // 新增
}
```

`blocks` 字段在数据库中存储为 `TEXT`（JSON 数组），与 `json` 字段类型的 migration 逻辑相同。

### 9.2 与插件系统结合

插件可通过 manifest 注册自定义 Block：

```toml
# plugins/newsletter/plugin.toml

[[blocks]]
type = "newsletter"
label = "Newsletter Signup"
icon = "mail"
component = "blocks/NewsletterBlock"   # 前端组件路径
```

插件加载时，前端通过 API 获取插件注册的 block 列表，动态加入 `BlockRegistry`。

### 9.3 与多租户结合

每个租户可有独立的 page_templates 配置（`tenant_id` 列过滤），
不同租户可使用不同的布局和 region block 配置。

## 10. 实施步骤

| 步骤 | 内容 | 优先级 |
|------|------|--------|
| 1 | 数据库 migration（layouts + page_templates 表 + 预置数据） | P0 |
| 2 | 后端 CRUD API（layout + template） | P0 |
| 3 | 后端模板解析 API（`/templates/resolve`） | P0 |
| 4 | Content Type 新增 `blocks` 字段类型 | P0 |
| 5 | 前端 BlockRegistry + BlockRenderer（只渲染，不编辑） | P0 |
| 6 | 前端 TemplateProvider + 模板匹配 | P0 |
| 7 | 前端 Admin 模板管理页面 | P1 |
| 8 | 前端 Block 编辑器（可视化编辑 block） | P1 |
| 9 | 插件注册自定义 Block 机制 | P2 |
| 10 | 拖拽排序 block | P2 |
