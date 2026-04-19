# 资源管理器设计文档

## 概述

通用的文件资源管理器，支持上传、在线预览、分类管理、搜索筛选。作为后台管理核心模块，同时服务于编辑器（插入图片/视频/附件）和独立管理场景。

## 功能清单

### 上传

| 功能 | 说明 |
|---|---|
| 拖拽上传 | 拖文件到页面直接上传，支持多文件批量 |
| 粘贴上传 | Ctrl+V 粘贴剪贴板图片（截图直接上传） |
| 按钮上传 | 点击按钮选择文件，支持多选 |
| 上传进度 | 每个文件显示独立进度条，失败可重试 |
| 格式校验 | 前后端双重校验 MIME 类型 + magic bytes |

### 文件分类

| 分类 | MIME 类型 | 图标 |
|---|---|---|
| 图片 | image/jpeg, image/png, image/gif, image/webp, image/svg+xml | `Image` |
| 视频 | video/mp4, video/webm, video/quicktime | `Video` |
| 音频 | audio/mpeg, audio/ogg, audio/wav, audio/aac | `Music` |
| 文档 | application/pdf, application/msword, application/vnd.* | `FileText` |
| 表格 | application/vnd.ms-excel, application/vnd.openxmlformats-* | `Sheet` |
| 压缩包 | application/zip, application/x-tar, application/gzip | `Archive` |
| 其他 | 以上未覆盖的类型 | `File` |

### 浏览

| 功能 | 说明 |
|---|---|
| 网格视图 | 缩略图卡片，显示文件名、大小、类型图标 |
| 列表视图 | 表格形式，显示文件名、类型、大小、上传时间、操作按钮 |
| 分类筛选 | 左侧侧边栏按文件类型分类，点击切换 |
| 搜索 | 按文件名模糊搜索 |
| 排序 | 按上传时间（默认）、文件名、文件大小排序 |
| 分页 | 每页 20/40/60 条，支持翻页 |

### 预览

| 文件类型 | 预览方式 |
|---|---|
| 图片 | 灯箱放大，支持缩放、左右切换 |
| 视频 | 内嵌播放器（`<video>`），播放/暂停/进度条 |
| 音频 | 内嵌播放器（`<audio>`），播放/暂停/进度条 |
| PDF | `<iframe>` 或 `<embed>` 内嵌预览 |
| Word/Excel | 提示下载（浏览器无法直接预览） |
| 其他 | 显示文件信息，提供下载链接 |

### 文件操作

| 操作 | 说明 |
|---|---|
| 重命名 | 弹出输入框修改文件名 |
| 复制 URL | 一键复制文件公开访问地址到剪贴板 |
| 下载 | 直接下载文件 |
| 删除 | 确认后删除（仅所有者或管理员） |
| 批量选择 | 网格视图支持多选，批量删除 |

### 编辑器集成

| 触发方式 | 行为 |
|---|---|
| 编辑器点击"插入图片" | 弹出资源管理器 → 选图 → 插入 `![](url)` |
| 编辑器点击"插入视频" | 弹出资源管理器 → 选视频 → 插入 `<video>` |
| 编辑器点击"插入链接" | 弹出资源管理器 → 选文件 → 插入 `[text](url)` |
| 编辑器粘贴/拖拽图片 | 直接上传并插入 |

## 页面布局

```
┌─────────────────────────────────────────────────────────┐
│  资源管理器    已用 128MB / 1GB            [网格] [列表]  │
├──────────┬──────────────────────────────────────────────┤
│          │  [上传文件 ▼]  🔍 搜索...  排序: 最新 ↑       │
│ 全部 (42) │──────────────────────────────────────────────│
│          │                                              │
│ 🖼 图片(15)│  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐     │
│ 🎬 视频(3) │  │  🖼   │ │  🖼   │ │  🎬   │ │  📄   │     │
│ 🎵 音频(5) │  │cover  │ │photo  │ │demo   │ │report │     │
│ 📄 文档(8) │  │.jpg   │ │.png   │ │.mp4   │ │.pdf   │     │
│ 📊 表格(4) │  │ 2.1MB │ │ 450KB │ │ 25MB  │ │ 1.2MB │     │
│ 📦 压缩(2) │  └──────┘ └──────┘ └──────┘ └──────┘     │
│ 📎 其他(5) │  ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐     │
│          │  │  📊   │ │  🎵   │ │  📦   │ │      │     │
│          │  │sales  │ │bgm    │ │backup │ │      │     │
│          │  │.xlsx  │ │.mp3   │ │.zip   │ │      │     │
│          │  │ 3.5MB │ │ 4.1MB │ │ 12MB  │ │      │     │
│          │  └──────┘ └──────┘ └──────┘ └──────┘     │
│          │                                              │
│          │  ← 1  2  3 →              每页 20 条         │
└──────────┴──────────────────────────────────────────────┘
```

### 详情面板（选中文件时右侧展开）

```
┌────────────────────────┐
│  🖼 cover.jpg          │
│  ┌──────────────────┐  │
│  │                  │  │
│  │   [图片预览]      │  │
│  │                  │  │
│  └──────────────────┘  │
│                        │
│  文件名  cover.jpg     │
│  类型    image/jpeg    │
│  大小    2.1 MB        │
│  上传者  admin         │
│  上传时间 2026-04-19   │
│                        │
│  URL                   │
│  ┌──────────────────┐  │
│  │ http://.../cover │📋│
│  └──────────────────┘  │
│                        │
│  [重命名] [下载] [删除]  │
└────────────────────────┘
```

## API 设计

### 现有接口调整

| 接口 | 变更 |
|---|---|
| `POST /api/v1/media/upload` | 放宽 MIME 白名单，支持所有类型 |

### 新增接口

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/v1/media/stats` | 存储统计（已用空间、文件数量按类型分组） |

#### GET /api/v1/media/stats 响应

```json
{
  "code": 0,
  "data": {
    "total_size": 134217728,
    "total_files": 42,
    "by_type": {
      "image": { "count": 15, "size": 31457280 },
      "video": { "count": 3, "size": 78643200 },
      "audio": { "count": 5, "size": 20971520 },
      "document": { "count": 8, "size": 2097152 },
      "spreadsheet": { "count": 4, "size": 1048576 },
      "archive": { "count": 2, "size": 12582912 },
      "other": { "count": 5, "size": 1048576 }
    }
  }
}
```

## 前端文件结构

```
web/src/app/admin/media/
  page.tsx                         ← 资源管理器主页面

web/src/components/admin/media/
  media-grid.tsx                   ← 网格视图
  media-list.tsx                   ← 列表视图
  media-upload.tsx                 ← 上传区域（拖拽 + 粘贴 + 按钮）
  media-upload-item.tsx            ← 单个上传进度条
  media-preview.tsx                ← 预览弹窗（图片/视频/音频/PDF）
  media-sidebar.tsx                ← 左侧分类筛选栏
  media-detail-panel.tsx           ← 右侧文件详情面板
  media-actions.tsx                ← 文件操作菜单（重命名/复制/下载/删除）
  media-selector.tsx               ← 编辑器内嵌选择器（选择并插入）
```

### 组件关系

```
page.tsx
├── media-sidebar.tsx          分类筛选
├── media-upload.tsx           上传区域
│   └── media-upload-item.tsx  进度条 × N
├── media-grid.tsx             网格视图
│   └── media-actions.tsx      每个文件的操作按钮
├── media-list.tsx             列表视图
│   └── media-actions.tsx
├── media-detail-panel.tsx     详情 + 预览
│   └── media-preview.tsx      预览组件
└── media-selector.tsx         编辑器调用时使用
    └── media-grid.tsx
```

## 后端改动

### 1. 放宽 MIME 白名单

```rust
// src/services/media.rs
const ALLOWED_TYPES: &[&str] = &[
    // 图片
    "image/jpeg", "image/png", "image/gif", "image/webp", "image/svg+xml",
    // 视频
    "video/mp4", "video/webm", "video/quicktime",
    // 音频
    "audio/mpeg", "audio/ogg", "audio/wav", "audio/aac",
    // 文档
    "application/pdf",
    "application/msword",
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "application/vnd.ms-excel",
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "application/vnd.ms-powerpoint",
    "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    // 压缩
    "application/zip", "application/x-tar", "application/gzip", "application/x-rar-compressed",
    // 文本
    "text/plain", "text/csv", "text/markdown",
];
```

### 2. 扩展 magic bytes

增加 ZIP、PDF 等通用格式的签名校验，对无法校验的类型（纯文本等）跳过 magic bytes 检查。

### 3. 新增 stats handler

```rust
// src/handlers/media.rs
pub async fn stats(
    State(state): State<crate::AppState>,
    auth_user: AuthUser,
    tenant: ResolvedTenant,
) -> AppResult<ApiResponse<MediaStatsResponse>>
```

### 4. 缩略图生成（可选 Phase 2）

利用已有的 `image` crate，上传图片时自动生成缩略图（200x200），存储为 `{key}.thumb.jpg`，用于网格视图快速加载。

## 上传大小限制

| 场景 | 建议值 |
|---|---|
| 图片 | 10MB |
| 视频 | 100MB |
| 其他 | 50MB |
| 总上限 | `MAX_UPLOAD_SIZE` 环境变量控制，默认 100MB |

前端按类型做预校验，超出直接提示。

## 编辑器集成方式

### 调用流程

```
用户点击编辑器"插入图片"按钮
    ↓
打开 Dialog 内嵌 media-selector.tsx
    ↓
media-selector 只显示图片类型，隐藏侧边栏
    ↓
用户选择一张图片（或上传新图）
    ↓
点击"插入"按钮
    ↓
返回 { url, alt } 给编辑器
    ↓
编辑器插入 ![alt](url)
```

### media-selector.tsx Props

```typescript
interface MediaSelectorProps {
  /** 过滤显示的文件类型 */
  filterType?: "image" | "video" | "audio" | "document" | "all";
  /** 是否允许多选 */
  multiple?: boolean;
  /** 选择确认回调 */
  onSelect: (files: MediaFile[]) => void;
  /** 取消回调 */
  onCancel: () => void;
}
```

## 实施计划

| 阶段 | 内容 | 预估 |
|---|---|---|
| **P1** | 放宽后端 MIME + 前端网格/列表视图 + 上传 + 搜索排序 | 1-2 天 |
| **P2** | 在线预览（图片灯箱 + 视频/音频播放 + PDF） | 0.5 天 |
| **P3** | 详情面板 + 文件操作（重命名/复制 URL/删除） | 0.5 天 |
| **P4** | 编辑器集成（media-selector） | 0.5 天 |
| **P5** | 存储统计 + 缩略图生成 | 1 天 |
