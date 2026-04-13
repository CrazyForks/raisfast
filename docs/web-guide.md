# 前端技术方案

## 1. 概述

博客系统的前端，基于 Next.js 16 (App Router) + Tailwind CSS v4 + shadcn/ui 构建。与 Rust 后端 (hello-axum) 前后端分离部署，开发时前端 dev server 直接请求后端 API。

项目位于 `web/` 目录。

---

## 2. 技术栈

| 类别 | 技术 | 版本 | 说明 |
|------|------|------|------|
| 框架 | Next.js (App Router) | 16.2 | RSC + Client Components |
| 语言 | TypeScript | 5.x | 严格模式 |
| 样式 | Tailwind CSS | 4.2 | v4，CSS-based 配置（无 `tailwind.config.ts`） |
| UI 组件库 | shadcn/ui (base-nova) | 4.2 | 按需复制组件到项目，可定制 |
| 基础 UI | @base-ui/react | 1.3 | shadcn v4 底层无样式原语 |
| 状态管理 | zustand | 5.0 | 轻量，存 auth 状态，persist 到 localStorage |
| 数据请求 | @tanstack/react-query | 5.97 | 缓存、自动刷新、loading/error 管理 |
| 表单 | react-hook-form + zod | 7.72 / 4.3 | 表单验证，和后端 validator 对齐 |
| 图标 | lucide-react | 1.8 | shadcn 默认图标库 |
| Toast | sonner | 2.0 | shadcn 推荐的 toast 方案 |
| i18n | next-intl | 4.9 | 国际化（复用后端 locale 逻辑） |
| 包管理器 | pnpm | 10.x | workspace 支持 |

---

## 3. 项目结构

```
web/
├── src/
│   ├── app/                        # Next.js App Router 页面
│   │   ├── layout.tsx              # 根 layout（Geist 字体 + Providers）
│   │   ├── page.tsx                # 首页
│   │   ├── globals.css             # Tailwind v4 + shadcn CSS 变量
│   │   ├── (public)/               # 公开页面组（Header + Footer 布局）
│   │   │   ├── layout.tsx
│   │   │   ├── posts/
│   │   │   │   ├── page.tsx        # 文章列表（分页、搜索、筛选）
│   │   │   │   └── [slug]/
│   │   │   │       └── page.tsx    # 文章详情（Markdown 渲染 + 评论）
│   │   │   └── categories/
│   │   │       └── [id]/
│   │   │           └── page.tsx    # 按分类浏览
│   │   ├── (admin)/                # 后台页面组（Sidebar 布局）
│   │   │   ├── layout.tsx          # AdminLayout：侧边栏 + 顶部导航
│   │   │   ├── dashboard/
│   │   │   │   └── page.tsx        # 仪表盘（统计卡片 + 最近评论）
│   │   │   ├── posts/
│   │   │   │   ├── page.tsx        # 文章管理 DataTable
│   │   │   │   ├── new/
│   │   │   │   │   └── page.tsx    # 新建文章（Markdown 编辑器）
│   │   │   │   └── [id]/edit/
│   │   │   │       └── page.tsx    # 编辑文章
│   │   │   ├── categories/
│   │   │   │   └── page.tsx        # 分类管理
│   │   │   ├── tags/
│   │   │   │   └── page.tsx        # 标签管理
│   │   │   ├── comments/
│   │   │   │   └── page.tsx        # 评论管理（审核 + 状态切换）
│   │   │   ├── media/
│   │   │   │   └── page.tsx        # 媒体管理（网格 + 拖拽上传）
│   │   │   └── users/
│   │   │       └── page.tsx        # 用户管理（仅 Admin）
│   │   ├── auth/
│   │   │   ├── login/
│   │   │   │   └── page.tsx        # 登录页
│   │   │   └── register/
│   │   │       └── page.tsx        # 注册页
│   │   └── profile/
│   │       └── page.tsx            # 个人资料设置
│   ├── components/
│   │   ├── ui/                     # shadcn 组件（自动生成）
│   │   ├── blog/                   # 博客专用组件
│   │   │   ├── post-card.tsx       # 文章卡片（列表展示）
│   │   │   ├── post-content.tsx    # 文章内容（HTML 渲染 + 代码高亮）
│   │   │   ├── post-toc.tsx        # 文章目录（TOC）
│   │   │   ├── comment-section.tsx # 评论区（树形嵌套）
│   │   │   ├── comment-form.tsx    # 评论表单（游客/登录）
│   │   │   ├── search-bar.tsx      # 搜索栏
│   │   │   └── tag-badge.tsx       # 标签徽章
│   │   ├── admin/                  # 后台专用组件
│   │   │   ├── post-form.tsx       # 文章表单（Markdown 编辑器 + 标签多选）
│   │   │   ├── media-grid.tsx      # 媒体网格（上传 + 管理器）
│   │   │   ├── comment-table.tsx   # 评论 DataTable
│   │   │   └── stats-card.tsx      # 统计卡片
│   │   ├── common/                 # 通用组件
│   │   │   ├── header.tsx          # 顶部导航栏
│   │   │   ├── footer.tsx          # 底部
│   │   │   ├── pagination.tsx      # 分页组件
│   │   │   └── user-menu.tsx       # 用户下拉菜单
│   │   └── providers.tsx           # 全局 Providers（React Query + Toaster）
│   ├── lib/
│   │   ├── api.ts                  # API 封装（自动 token / refresh / i18n）
│   │   └── utils.ts                # shadcn cn() 工具函数
│   ├── stores/
│   │   └── auth.ts                 # zustand auth store（localStorage 持久化）
│   └── hooks/
│       └── use-mobile.ts           # shadcn 移动端检测 hook
├── public/                         # 静态资源
├── .env.local                      # 环境变量
├── next.config.ts                  # Next.js 配置
├── postcss.config.mjs              # PostCSS（@tailwindcss/postcss）
├── components.json                 # shadcn 配置
├── tsconfig.json                   # TypeScript 配置（@/* 路径别名）
└── package.json
```

### 3.1 路由组说明

| 路由组 | 布局 | 认证要求 | 页面 |
|--------|------|----------|------|
| `(public)` | Header + Footer | 无 | 首页、文章列表、文章详情、分类 |
| `(admin)` | Sidebar + TopNav | 需登录（Author/Admin） | 仪表盘、文章管理、评论管理、媒体管理、用户管理 |
| `auth` | 居中卡片布局 | 无 | 登录、注册 |
| `profile` | Header + Footer | 需登录 | 个人资料 |

---

## 4. API 集成

### 4.1 后端 API 基地址

```env
# .env.local
NEXT_PUBLIC_API_URL=http://localhost:3000/api/v1
```

生产环境改为实际后端地址。

### 4.2 API 封装设计 (`src/lib/api.ts`)

```
请求流程：
  fetch → 自动加 Authorization → 自动加 Accept-Language
       → 401? → 尝试 refresh token → 成功? 重试原请求 → 失败? logout
       → 解析 { code, message, data } → code≠0? throw ApiError → 返回 data
```

封装提供以下方法：

| 方法 | 用途 | Content-Type |
|------|------|--------------|
| `api.get<T>(path)` | GET 请求 | application/json |
| `api.post<T>(path, body)` | POST JSON | application/json |
| `api.put<T>(path, body)` | PUT JSON | application/json |
| `api.delete<T>(path)` | DELETE | application/json |
| `api.upload<T>(path, file)` | POST 文件 | multipart/form-data |

### 4.3 Token 刷新机制

- Access Token 过期（后端默认 15 分钟）
- API 层自动拦截 401，调用 `/auth/refresh` 获取新 token
- 刷新成功：更新 zustand store，重试原请求
- 刷新失败：清除 store，跳转登录页
- Refresh Token 过期（后端默认 7 天）：需重新登录

### 4.4 后端 API 路由表

| 方法 | 路径 | 认证 | 说明 |
|------|------|------|------|
| GET | `/health` | 无 | 健康检查 |
| POST | `/auth/register` | 无 | 注册 |
| POST | `/auth/login` | 无 | 登录 |
| POST | `/auth/refresh` | 无 | 刷新 Token |
| POST | `/auth/logout` | 是 | 登出 |
| GET | `/users/me` | 是 | 当前用户信息 |
| PUT | `/users/me` | 是 | 更新用户资料 |
| PUT | `/users/me/password` | 是 | 修改密码 |
| GET | `/users/{id}` | 无 | 用户公开信息 |
| GET | `/users` | Admin | 用户列表 |
| GET | `/categories` | 无 | 分类列表 |
| POST | `/categories` | Author | 创建分类 |
| PUT | `/categories/{id}` | Author | 更新分类 |
| DELETE | `/categories/{id}` | Author | 删除分类 |
| GET | `/tags` | 无 | 标签列表 |
| POST | `/tags` | Author | 创建标签 |
| DELETE | `/tags/{id}` | Author | 删除标签 |
| GET | `/posts` | 无 | 文章列表（支持 ?page=&q=&category_id=&tag_id=） |
| POST | `/posts` | Author | 创建文章 |
| GET | `/posts/{slug}` | 无 | 文章详情 |
| PUT | `/posts/{slug}` | 是* | 更新文章（*作者或 Admin） |
| DELETE | `/posts/{slug}` | 是* | 删除文章（*作者或 Admin） |
| GET | `/posts/{slug}/comments` | 无 | 评论列表 |
| POST | `/posts/{slug}/comments` | 无 | 游客评论 |
| POST | `/posts/{slug}/comments/authed` | 是 | 登录用户评论 |
| DELETE | `/comments/{id}` | 是* | 删除评论（*作者或 Admin） |
| PUT | `/comments/{id}/status` | Admin | 审核评论 |
| POST | `/media/upload` | 是 | 上传文件 |
| GET | `/media` | 是 | 媒体列表 |
| DELETE | `/media/{id}` | 是* | 删除媒体（*所有者或 Admin） |
| GET | `/feed.xml` | 无 | RSS 订阅 |

### 4.5 后端响应格式

```json
// 成功
{ "code": 0, "message": "success", "data": { ... } }

// 分页
{ "code": 0, "message": "success", "data": { "items": [...], "total": 42, "page": 1, "page_size": 20 } }

// 错误
{ "code": 40100, "message": "unauthorized", "data": null }
```

错误码范围：

| 范围 | HTTP 状态 |
|------|-----------|
| 40000 | 400 Bad Request |
| 40100 | 401 Unauthorized |
| 40300 | 403 Forbidden |
| 40400 | 404 Not Found |
| 40900 | 409 Conflict |
| 42900 | 429 Too Many Requests |
| 50000 | 500 Internal Server Error |

### 4.6 国际化 (i18n) 对接

后端已支持通过 `Accept-Language` 头或 `?lang=` 参数切换语言（en / zh-CN）。

API 层自动附加 `Accept-Language: navigator.language`，前端无需额外处理。

如需前端 UI 也做国际化，使用 `next-intl`，翻译文件放在 `src/i18n/` 目录。

---

## 5. 状态管理

### 5.1 Auth Store (`src/stores/auth.ts`)

使用 zustand + persist 中间件，自动持久化到 `localStorage("auth-storage")`。

```
AuthState {
  user: User | null          // 用户信息
  accessToken: string | null // JWT access token
  refreshToken: string | null // JWT refresh token

  login(user, access, refresh) → 设置全部
  logout()                    → 清除全部
  isLoggedIn()                → accessToken !== null
  isAdmin()                   → user.role === "admin"
  isAuthor()                  → role in ["admin", "author"]
}
```

### 5.2 服务端数据 (React Query)

所有 API 数据通过 `@tanstack/react-query` 管理：

- 缓存：默认 `staleTime: 60s`
- 自动重试：1 次
- 使用 `useQuery` 读取、`useMutation` 写入
- Query Key 约定：`["posts", { page, q, category_id }]`、`["post", slug]`、`["comments", slug]`

---

## 6. 页面设计

### 6.1 公开页面

#### 首页 (`/`)

- Hero 区域：博客标题 + 简介
- 最新文章列表（6 篇）
- 快速链接：浏览全部、RSS

#### 文章列表 (`/posts`)

- 搜索栏 + 分类筛选 + 标签筛选
- 文章卡片网格（标题、摘要、标签、日期、浏览量）
- 分页组件

#### 文章详情 (`/posts/[slug]`)

- 文章标题、元信息（作者、日期、分类、标签）
- HTML 内容（后端已渲染，前端直接展示）
- 代码高亮（Shiki，客户端增强）
- 目录导航（TOC，从 h2/h3 标题提取）
- 评论区：树形嵌套展示 + 评论表单

#### 登录 (`/auth/login`)

- 邮箱 + 密码表单
- zod 校验（邮箱格式、密码长度）
- 登录成功后 redirect 到之前的页面

#### 注册 (`/auth/register`)

- 邮箱 + 用户名 + 密码表单
- zod 校验（邮箱格式、用户名 2-50 字符、密码 ≥ 8 字符）

### 6.2 后台页面

#### 仪表盘 (`/admin/dashboard`)

- 统计卡片：文章数、评论数、媒体数
- 最近待审核评论列表
- 最近文章列表

#### 文章管理 (`/admin/posts`)

- DataTable：标题、状态（draft/published）、分类、标签、日期
- 操作：编辑、删除
- 状态筛选
- 新建按钮 → `/admin/posts/new`

#### 文章编辑 (`/admin/posts/[id]/edit`)

- Markdown 编辑器（Milkdown 或 Novel）
- 标题、分类选择、标签多选
- 封面图上传
- 摘要编辑（自动从内容提取）
- 状态切换：草稿 / 发布

#### 评论管理 (`/admin/comments`)

- DataTable：评论内容、文章、作者、状态、日期
- 状态筛选：pending / approved / spam
- 快速操作：通过、标记垃圾、删除

#### 媒体管理 (`/admin/media`)

- 网格视图
- 拖拽上传
- 点击复制 URL
- 删除

#### 分类管理 (`/admin/categories`)

- 简单列表 + 新建/编辑/删除

#### 标签管理 (`/admin/tags`)

- 简单列表 + 新建/删除

#### 用户管理 (`/admin/users`)

- DataTable：用户名、邮箱、角色、注册时间
- 仅 Admin 可访问

---

## 7. 组件库

### 7.1 已安装的 shadcn 组件

| 组件 | 用途 |
|------|------|
| button | 按钮 |
| card | 卡片容器 |
| input | 输入框 |
| label | 表单标签 |
| textarea | 多行文本 |
| select | 下拉选择 |
| badge | 徽章标签 |
| avatar | 用户头像 |
| dropdown-menu | 下拉菜单 |
| dialog | 模态框 |
| sheet | 侧滑面板 |
| separator | 分隔线 |
| skeleton | 加载骨架屏 |
| sonner | Toast 通知 |
| tabs | 标签页 |
| table | 表格 |
| command | 命令面板 |
| sidebar | 侧边栏 |
| tooltip | 提示气泡 |

### 7.2 待按需添加

| 组件 | 用途 | 安装命令 |
|------|------|----------|
| form | react-hook-form 集成 | `pnpm dlx shadcn@latest add form` |
| pagination | 分页 | `pnpm dlx shadcn@latest add pagination` |
| calendar | 日期选择 | `pnpm dlx shadcn@latest add calendar` |
| popover | 弹出层 | `pnpm dlx shadcn@latest add popover` |
| switch | 开关 | `pnpm dlx shadcn@latest add switch` |
| alert-dialog | 确认对话框 | `pnpm dlx shadcn@latest add alert-dialog` |
| breadcrumb | 面包屑 | `pnpm dlx shadcn@latest add breadcrumb` |

---

## 8. 样式方案

### 8.1 Tailwind CSS v4

v4 不再使用 `tailwind.config.ts`，改为 CSS-based 配置：

```css
/* src/app/globals.css */
@import "tailwindcss";
@import "tw-animate-css";
@import "shadcn/tailwind.css";

/* CSS 变量定义主题色（oklch 色彩空间） */
:root { --primary: oklch(0.205 0 0); ... }
.dark { --primary: oklch(0.922 0 0); ... }
```

### 8.2 主题切换

已内置 light/dark 双主题 CSS 变量。可集成 `next-themes` 实现：

```tsx
import { ThemeProvider } from "next-themes";

// 在 Providers 中包裹
<ThemeProvider attribute="class" defaultTheme="system">
```

### 8.3 字体

- 正文字体：Geist Sans（Next.js 内置）
- 代码字体：Geist Mono

---

## 9. 开发流程

### 9.1 环境准备

```bash
cd web
pnpm install          # 安装依赖
cp .env.local .env    # 确认 API 地址
```

### 9.2 启动开发服务器

```bash
# 终端 1：启动后端
cd /path/to/hello-axum
cargo run

# 终端 2：启动前端
cd web
pnpm dev              # 默认 http://localhost:3001
```

### 9.3 添加 shadcn 组件

```bash
pnpm dlx shadcn@latest add <component-name>
```

### 9.4 构建

```bash
pnpm build            # 生产构建，输出到 .next/
pnpm start            # 启动生产服务器
```

### 9.5 代码规范

```bash
pnpm lint             # ESLint 检查
```

---

## 10. 部署方案

### 10.1 推荐：前后端分离部署

```
用户 → Nginx
         ├── /             → Next.js (Node.js) 或静态文件
         ├── /api/v1/*     → hello-axum (Rust)
         ├── /uploads/*    → hello-axum 静态文件
         └── /feed.xml     → hello-axum RSS
```

Nginx 配置示例：

```nginx
server {
    listen 80;
    server_name blog.example.com;

    # 前端
    location / {
        proxy_pass http://127.0.0.1:3001;
    }

    # 后端 API
    location /api/v1/ {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }

    # RSS
    location /feed.xml {
        proxy_pass http://127.0.0.1:3000;
    }

    # 上传文件
    location /uploads/ {
        proxy_pass http://127.0.0.1:3000;
        expires 30d;
    }
}
```

### 10.2 备选：前端静态导出

如果博客不需要 SSR，可以 `next export` 输出纯静态文件，用 Nginx 直接托管（无需 Node.js 进程）。

需要在 `next.config.ts` 中添加：

```ts
const nextConfig: NextConfig = {
  output: "export",
};
```

### 10.3 环境变量

| 变量 | 开发默认值 | 生产值 |
|------|-----------|--------|
| `NEXT_PUBLIC_API_URL` | `http://localhost:3000/api/v1` | `https://blog.example.com/api/v1` |

---

## 11. 安全注意事项

- **Token 存储**：Access Token 和 Refresh Token 存在 localStorage（仅限 HTTPS 环境使用）
- **XSS 防护**：React 自动转义，后端已用 ammonia 清洗 HTML
- **CSRF**：REST API 使用 Bearer Token 认证，无 Cookie，不受 CSRF 攻击
- **CORS**：后端已配置 `CORS_ORIGINS` 白名单，生产环境设置为前端域名
- **上传**：后端已限制文件类型（magic bytes 校验）和大小（5MB）
- **Rate Limiting**：后端已对登录、注册、评论接口限流
