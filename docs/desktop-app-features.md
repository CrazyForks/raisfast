# 桌面应用功能清单

> raisfast Desktop — **All in One，开箱即用**
>
> 参考产品：VS Code（编辑器）、TablePlus（数据库管理）、Postman（API 调试）、
> Strapi Admin（CMS 管理）、Figma（协作）、PocketBase Admin（BaaS 管理）、
> Obsidian（笔记管理）、Docker Desktop（服务管理）

---

## 设计理念

| 原则 | 说明 |
|---|---|
| **All in One** | 一个应用覆盖后端开发全流程，无需安装其他工具 |
| **开箱即用** | 下载 → 打开 → 创建项目 → 立即开发，零配置 |
| **本地优先** | 所有数据和服务运行在本地，无需联网 |
| **一键发布** | 本地开发完成，一键导出为生产服务器 |

---

## 1. 项目管理

> 参考：VS Code Workspace、Xcode Project、Docker Desktop

### 1.1 项目仪表盘

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 项目列表 | 卡片式展示所有项目，显示状态/数据量/最后编辑时间 | VS Code Recent | P0 |
| 新建项目 | 向导式创建，选择模板（Blog/E-commerce/Forum/空白） | Xcode New Project | P0 |
| 打开项目 | 打开已有项目目录，自动识别 `extension.toml` | VS Code Open Folder | P0 |
| 项目概览 | 显示 API 数量、Content Type 数量、插件数量、数据库大小 | Docker Desktop Dashboard | P1 |
| 项目设置 | 端口、数据库路径、密钥、邮件配置等 | VS Code Settings | P1 |
| 项目收藏 | 收藏常用项目，快速访问 | — | P2 |
| 最近打开 | 最近打开的项目列表 | VS Code | P2 |

### 1.2 项目模板

| 功能 | 说明 | 优先级 |
|---|---|---|
| 博客模板 | Blog + 文章/分类/标签/评论，内置 Admin UI | P0 |
| 电商模板 | 产品/购物车/订单/支付，对接 Stripe | P0 |
| 论坛模板 | 板块/话题/回复/投票/投票，对接 Forum 插件 | P0 |
| SaaS 模板 | 多租户 + RBAC + 订阅计费 | P1 |
| 文档站模板 | Markdown 文档 + 全文搜索 + 版本管理 | P1 |
| API 服务模板 | 纯 API 后端，无前端 | P1 |
| 空白模板 | 零预设，从零开始 | P0 |
| 社区模板 | 从模板市场下载社区贡献的模板 | P2 |

### 1.3 项目导入/导出

| 功能 | 说明 | 优先级 |
|---|---|---|
| 导出为 HTTP 服务器 | 生成独立二进制文件，可直接部署到 Linux/Mac/Windows | P0 |
| 导出为 Docker 镜像 | 生成 Dockerfile + 构建镜像 | P1 |
| 导出为静态站点 | 如果项目是纯 CMS，可导出为静态 HTML | P2 |
| 导入 Strapi 项目 | 解析 Strapi schema，转换为 raisfast Content Type | P2 |
| 导入 PocketBase 数据 | 导入 PocketBase 的 SQLite 数据 | P2 |
| 项目备份/恢复 | 导出完整项目快照（数据库 + 配置 + 插件） | P1 |

---

## 2. Content-Type Builder（可视化）

> 参考：Strapi Content-Type Builder、Directus Insights、Airtable

### 2.1 Schema 设计器

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 可视化建表 | 拖拽添加字段，选择类型，配置属性 | Strapi CT Builder | P0 |
| 字段类型面板 | Text/Number/Boolean/Date/Enum/Relation/File/JSON/Rich Text | Strapi | P0 |
| 字段属性编辑 | 必填/默认值/验证规则/索引/唯一约束 | Directus | P0 |
| 关系设计 | 一对一/一对多/多对多，可视化连线 | Prisma Studio | P0 |
| 枚举编辑器 | 可视化编辑枚举值和颜色标签 | — | P0 |
| API Rule 配置 | 可视化配置每种操作的访问权限（Public/Member/Admin/None） | PocketBase | P0 |
| 草稿/发布开关 | 是否启用 draft/publish 工作流 | Strapi | P1 |
| 时间戳开关 | 是否自动管理 created_at/updated_at | — | P0 |
| 软删除开关 | 是否启用逻辑删除 | — | P1 |
| 缓存开关 | 是否启用查询缓存 | — | P1 |
| Schema 预览 | 实时预览生成的 TOML 和 SQL DDL | — | P0 |
| Schema 版本历史 | 查看字段变更历史，支持回滚 | Prisma Migrate | P2 |
| 批量导入字段 | 从 JSON/CSV/SQL 导入字段定义 | — | P2 |
| Schema 模板 | 预置常用 Schema（地址/商品/用户资料/文章） | — | P1 |

### 2.2 数据浏览器

> 参考：TablePlus、DBeaver、Airtable、NocoDB

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 表格视图 | 类电子表格展示数据，支持排序/筛选/分页 | TablePlus | P0 |
| 表单视图 | 单条记录编辑表单，字段类型对应合适的输入控件 | NocoDB | P0 |
| 看板视图 | 按状态字段分组展示（类 Trello） | Airtable | P2 |
| 画廊视图 | 图片/卡片式展示 | Airtable | P2 |
| 内联编辑 | 直接在表格中双击编辑单元格 | TablePlus | P0 |
| 批量操作 | 多选 → 批量删除/修改/导出 | DBeaver | P1 |
| 高级筛选 | 组合条件筛选（AND/OR/NOT），支持关联字段 | Airtable | P0 |
| 全文搜索 | 跨字段全文搜索，高亮匹配 | — | P1 |
| 关联数据查看 | 点击关联字段直接跳转查看关联记录 | TablePlus | P1 |
| 数据导入 | CSV/JSON/SQL 导入数据 | DBeaver | P1 |
| 数据导出 | 导出为 CSV/JSON/SQL/Excel | TablePlus | P1 |
| SQL 查询面板 | 直接写 SQL 查询，显示结果表格 | TablePlus | P1 |
| 数据可视化 | 简单图表（柱状图/饼图/折线图），查看字段分布 | Directus Insights | P2 |

---

## 3. 插件开发环境

> 参考：VS Code（代码编辑）、Chrome DevTools（调试）、Node-RED（可视化编程）

### 3.1 代码编辑器

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| JS 编辑器 | 语法高亮、智能提示（Host API 补全）、括号匹配 | VS Code | P0 |
| Lua 编辑器 | 语法高亮、Host API 补全 | VS Code + Lua 插件 | P0 |
| TOML 编辑器 | 语法高亮、Schema 校验 | VS Code | P0 |
| SQL 编辑器 | 语法高亮、表名/列名自动补全 | VS Code + SQL 插件 | P1 |
| 多标签编辑 | 同时编辑多个文件，标签页切换 | VS Code | P0 |
| 文件树 | 项目目录树，点击打开文件 | VS Code Explorer | P0 |
| 快捷键 | 支持常用编辑快捷键（复制/粘贴/撤销/搜索/替换） | VS Code | P0 |
| 代码片段 | 预置常用代码模板（CRUD/Hook/Route/事务） | VS Code Snippets | P1 |
| Mini Map | 代码缩略图导航 | VS Code | P2 |
| 多光标编辑 | Alt+Click 多位置同时编辑 | VS Code | P2 |
| 查找替换 | 当前文件查找替换，支持正则 | VS Code | P0 |
| 全局搜索 | 跨文件搜索 | VS Code | P1 |

### 3.2 调试与预览

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 即时预览 | 保存后立即在右侧面板看到 Hook/Route 执行结果 | Chrome DevTools | P0 |
| 控制台日志 | 查看 `Host.log()` 输出，按级别筛选 | Chrome Console | P0 |
| API 测试面板 | 构造请求 → 发送到插件路由 → 查看响应 | Postman | P0 |
| 断点调试 | 在 JS/Lua 代码中设置断点，逐步执行 | Chrome DevTools | P2（技术难度高） |
| 变量检查器 | 查看当前作用域变量值 | Chrome DevTools | P2 |
| 网络面板 | 查看 Host.httpGet/httpPost 请求和响应 | Chrome Network | P1 |
| SQL 日志 | 查看 Host.dbQuery/dbExecute 执行的 SQL 和耗时 | Django Debug Toolbar | P0 |
| 错误面板 | 显示语法错误和运行时错误，点击跳转到代码行 | VS Code Problems | P0 |
| 性能面板 | 显示 Hook/Route 执行耗时、内存使用 | Chrome Performance | P2 |
| 热加载指示 | 显示插件热加载状态和重载历史 | — | P1 |

### 3.3 插件管理

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 插件列表 | 显示已安装插件，启用/禁用/卸载 | VS Code Extensions | P0 |
| 插件配置 | 可视化编辑插件配置项（对应 `permissions`） | VS Code Settings | P0 |
| 插件模板 | 一键生成 JS/Lua/WASM 插件脚手架 | `ext new` | P0 |
| 插件市场 | 浏览和安装社区插件（离线 + 在线） | VS Code Marketplace | P2 |
| 插件权限 | 可视化配置 DB/HTTP/FS 权限 | — | P0 |
| 插件日志 | 查看插件运行日志，按插件筛选 | Docker Logs | P1 |
| 插件依赖 | 管理插件之间的依赖关系 | npm/cargo | P2 |
| 插件版本 | 查看/回滚插件版本 | — | P2 |

### 3.4 Host API 文档

| 功能 | 说明 | 优先级 |
|---|---|---|
| API 侧边栏 | 侧边栏展示所有 Host API，点击插入代码片段 | P0 |
| 参数提示 | 输入 `Host.` 后弹出 API 列表和参数说明 | P0 |
| 内置文档 | 不需要联网，所有 API 文档内置 | P0 |
| 示例代码 | 每个 Host API 提供 2-3 个使用示例 | P1 |
| 快速跳转 | Ctrl+Click Host API 跳转到文档 | P2 |

---

## 4. Auth & 用户管理

> 参考：Auth0 Dashboard、PocketBase Admin、Keycloak

### 4.1 用户管理

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 用户列表 | 表格展示所有用户，支持搜索/筛选/排序 | PocketBase Admin | P0 |
| 用户详情 | 查看用户信息、角色、登录历史、关联数据 | Auth0 | P0 |
| 创建用户 | 手动创建用户（设置用户名/邮箱/密码/角色） | PocketBase | P0 |
| 编辑用户 | 修改用户信息、重置密码、禁用/启用 | PocketBase | P0 |
| 删除用户 | 单个/批量删除用户，可选关联数据处理 | PocketBase | P1 |
| 角色分配 | 给用户分配角色，可视化选择权限 | Keycloak | P0 |

### 4.2 角色权限 (RBAC)

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 角色列表 | 管理所有角色 | Keycloak | P0 |
| 权限矩阵 | 表格展示 角色 × 资源 的权限配置 | Keycloak | P0 |
| 自定义角色 | 创建自定义角色，勾选权限 | Strapi | P0 |
| 权限模板 | 预置常用角色（Admin/Editor/Viewer/Member） | — | P0 |
| 集合级权限 | 按 Content Type 配置 CRUD 权限 | Strapi | P0 |
| 字段级权限 | 控制字段级别的读写权限 | Strapi | P2 |

### 4.3 认证配置

| 功能 | 说明 | 优先级 |
|---|---|---|
| JWT 配置 | Token 过期时间、密钥轮换 | P0 |
| OAuth2 配置 | GitHub/Google/Apple/Facebook 登录，可视化配置 Client ID/Secret | P1 |
| 邮箱验证 | 开关 + 邮件模板编辑 | P1 |
| 密码策略 | 最小长度/复杂度要求 | P1 |
| 登录限流 | 可视化配置登录频率限制 | P2 |
| API Token 管理 | 创建/撤销 API Token，设置权限和过期时间 | P0 |

---

## 5. API 开发与调试

> 参考：Postman、Insomnia、Hoppscotch

### 5.1 API 调试器

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 请求构建器 | Method/URL/Headers/Body/Params 编辑 | Postman | P0 |
| 认证支持 | 自动携带 JWT Token，支持 OAuth2 流程 | Postman | P0 |
| 响应查看器 | JSON 格式化、Raw/Preview/Header 切换 | Postman | P0 |
| 请求历史 | 记录所有请求，快速重发 | Postman | P0 |
| 环境变量 | 多环境切换（开发/测试/生产） | Postman | P1 |
| 集合管理 | API 按目录分组管理 | Postman | P1 |
| 批量测试 | 对一组 API 执行测试脚本，显示通过/失败 | Postman | P2 |
| 自动生成 | 根据 Content Type 自动生成 CRUD API 集合 | — | P0 |
| cURL 导出 | 将请求导出为 cURL/HTTPie/fetch 命令 | Postman | P1 |
| Mock 服务 | 根据 Schema 生成 Mock 数据 | Postman | P2 |

### 5.2 API 文档

| 功能 | 说明 | 优先级 |
|---|---|---|
| 自动生成 | 根据代码自动生成 API 文档（OpenAPI/Swagger） | P1 |
| 内置 Swagger UI | 桌面应用内嵌 Swagger UI | P1 |
| 在线文档导出 | 导出为静态 HTML 文档站点 | P2 |
| SDK 生成 | 根据开放 API 生成 TS/Go/Python 客户端 SDK | P2 |

---

## 6. 数据库管理

> 参考：TablePlus、DBeaver、DB Browser for SQLite、Prisma Studio

### 6.1 数据库浏览器

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 表列表 | 显示所有表，点击查看结构和数据 | TablePlus | P0 |
| 表结构查看 | 显示列名/类型/约束/索引/默认值 | DB Browser for SQLite | P0 |
| 原生 SQL 编辑 | 自由执行 SQL，支持语法高亮和格式化 | TablePlus | P0 |
| 查询历史 | 记录执行的 SQL 和结果 | DBeaver | P1 |
| 可视化 ER 图 | 自动生成表关系图，显示外键连线 | DBeaver | P2 |
| 索引管理 | 查看/创建/删除索引 | TablePlus | P1 |
| 触发器管理 | 查看/创建/删除触发器 | DBeaver | P2 |
| 数据库统计 | 表大小/行数/索引大小/碎片率 | TablePlus | P1 |

### 6.2 迁移管理

| 功能 | 说明 | 优先级 |
|---|---|---|
| 迁移列表 | 显示所有已执行的迁移文件和状态 | P0 |
| 执行迁移 | 一键执行待运行的迁移 | P0 |
| 创建迁移 | 根据 Schema 变更自动生成迁移 SQL | P1 |
| 回滚迁移 | 回滚到指定版本 | P1 |
| 迁移预览 | 执行前预览 SQL，确认后再执行 | P1 |

---

## 7. 文件管理

> 参考：Cloudflare R2 Dashboard、AWS S3 Console、PocketBase Admin

### 6.1 媒体库

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 文件上传 | 拖拽上传文件，支持批量上传 | PocketBase Admin | P0 |
| 文件浏览 | 网格/列表视图浏览已上传文件 | Cloudflare R2 | P0 |
| 图片预览 | 点击图片全屏预览，支持缩放 | — | P0 |
| 文件详情 | 文件名/大小/类型/上传时间/URL | PocketBase | P0 |
| 文件夹管理 | 创建文件夹，拖拽移动文件 | AWS S3 | P1 |
| 图片编辑 | 基础裁剪/旋转/压缩（客户端处理） | — | P2 |
| 存储配置 | 本地存储/S3/MinIO 切换配置 | — | P1 |

---

## 8. 实时监控

> 参考：Docker Desktop、Grafana、PocketBase Admin

### 8.1 服务状态

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 服务启停 | 一键启动/停止/重启后端服务 | Docker Desktop | P0 |
| 状态指示 | 实时显示服务运行状态（运行中/已停止/错误） | Docker Desktop | P0 |
| 端口监控 | 显示服务监听端口，快速打开浏览器 | Docker Desktop | P0 |
| 资源监控 | CPU/内存/请求数/响应时间实时图表 | Docker Desktop | P1 |
| 请求日志 | 实时滚动显示 HTTP 请求日志（类 tail -f） | Docker Desktop | P0 |
| 事件流 | 显示系统事件（用户注册/数据变更/插件事件） | — | P1 |
| WebSocket 监控 | 显示实时 WebSocket 连接和消息 | — | P2 |

### 8.2 日志查看器

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 日志流 | 实时滚动显示日志 | Docker Desktop Logs | P0 |
| 级别筛选 | 按 DEBUG/INFO/WARN/ERROR 筛选 | Chrome Console | P0 |
| 来源筛选 | 按模块筛选（Auth/CMS/Plugin/DB/HTTP） | — | P0 |
| 时间范围 | 按时间段筛选日志 | Grafana | P1 |
| 关键词搜索 | 全文搜索日志内容 | — | P0 |
| 日志导出 | 导出为文件 | Docker Desktop | P1 |

---

## 9. 发布与部署

> 参考：Docker Desktop、Vercel CLI、Netlify CLI、PocketBase

### 9.1 一键发布

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 发布为二进制 | 编译为 Linux/Mac/Windows 可执行文件 | Go build | P0 |
| 发布为 Docker | 生成 Dockerfile 并构建镜像 | Docker Desktop | P0 |
| 发布配置 | 选择目标平台、端口、数据库路径 | — | P0 |
| 环境变量管理 | 可视化编辑环境变量，区分环境 | Vercel | P1 |
| 构建日志 | 显示编译进度和错误 | Docker Desktop | P0 |
| 一键部署到 VPS | SSH 连接远程服务器，上传并启动 | — | P2 |
| 一键部署到云 | 集成 AWS/GCP/Azure 部署 | Vercel | P2 |

### 9.2 CLI 工具集成

| 功能 | 说明 | 参考 | 优先级 |
|---|---|---|---|
| 内置终端 | 桌面应用内嵌终端，直接运行 CLI 命令 | VS Code Terminal | P0 |
| CLI 命令面板 | 可视化选择并执行 CLI 命令 | VS Code Command Palette | P1 |
| 独立 CLI | 提供 `raisfast` 命令行工具（serve/migrate/ext/backup） | PocketBase CLI | P0 |
| Shell 补全 | 生成 bash/zsh/fish 补全脚本 | — | P2 |

---

## 10. 设置与偏好

### 10.1 应用设置

| 功能 | 说明 | 优先级 |
|---|---|---|
| 主题切换 | 亮色/暗色/跟随系统 | P0 |
| 语言切换 | 中文/英文/跟随系统 | P1 |
| 字体大小 | 编辑器字体大小调节 | P0 |
| 快捷键配置 | 自定义快捷键 | P2 |
| 自动保存 | 编辑器自动保存开关和间隔 | P0 |
| 默认编辑器 | 选择外部编辑器（VS Code / Vim） | P2 |

### 10.2 项目设置

| 功能 | 说明 | 优先级 |
|---|---|---|
| 服务端口 | HTTP 服务监听端口 | P0 |
| 数据库配置 | SQLite 文件路径 / PostgreSQL 连接字符串 | P0 |
| JWT 配置 | 密钥、Access/Refresh Token 过期时间 | P0 |
| 邮件配置 | SMTP 服务器配置 | P1 |
| 存储配置 | 本地路径 / S3 Bucket | P1 |
| 日志配置 | 日志级别、保留天数 | P1 |
| 插件配置 | 全局插件设置（内存限制、超时、热加载） | P0 |
| 多租户配置 | 租户管理、租户隔离策略 | P2 |

---

## 11. 扩展市场（Phase 2+）

> 参考：VS Code Marketplace、Shopify App Store、WordPress Plugin Directory

### 11.1 插件市场

| 功能 | 说明 | 优先级 |
|---|---|---|
| 浏览插件 | 按分类浏览社区插件 | P2 |
| 搜索插件 | 关键词搜索插件 | P2 |
| 一键安装 | 点击安装，自动下载并启用 | P2 |
| 插件详情 | README、截图、版本历史、评分 | P2 |
| 插件评分 | 用户评分和评论 | P3 |
| 插件更新 | 检测更新，一键升级 | P2 |
| 提交插件 | 开发者提交插件到市场 | P3 |

### 11.2 模板市场

| 功能 | 说明 | 优先级 |
|---|---|---|
| 浏览模板 | 按分类浏览前端模板 | P2 |
| 预览模板 | 在应用内预览模板效果 | P2 |
| 一键安装 | 下载模板并应用到当前项目 | P2 |
| 模板详情 | 截图、功能说明、依赖插件列表 | P2 |

---

## 12. 协作（Phase 3+）

> 参考：Figma、GitHub Desktop、Cursor

### 12.1 团队协作

| 功能 | 说明 | 优先级 |
|---|---|---|
| 多用户切换 | 快速切换不同账号的 API Token | P2 |
| 配置同步 | 项目配置同步到 Git 仓库 | P2 |
| Schema Diff | 比较不同环境的 Schema 差异 | P2 |
| 协同编辑 | 多人同时编辑 Schema（类 Figma） | P3 |

---

## 功能优先级总览

### P0 — MVP（2-3 月）

| 模块 | 功能 |
|---|---|
| 项目管理 | 创建/打开/概览 |
| Content-Type Builder | 可视化建表 + API Rule 配置 |
| 数据浏览器 | 表格/表单视图 + CRUD + 筛选 |
| 插件编辑器 | JS/Lua 代码编辑 + 语法高亮 + Host API 补全 |
| 调试面板 | 控制台日志 + API 测试 + SQL 日志 |
| Auth 管理 | 用户/角色/权限管理 |
| 服务管理 | 启停服务 + 请求日志 + 状态监控 |
| 发布 | 导出为二进制 |
| CLI | serve/migrate/ext/backup 命令 |

### P1 — 完善（3-6 月）

数据库管理、文件管理、OAuth2、环境变量、Docker 发布、高级筛选、请求历史、性能监控

### P2 — 生态（6-12 月）

插件市场、模板市场、ER 图、看板视图、断点调试、协同编辑、云部署

### P3 — 进阶（12+ 月）

评分系统、协同编辑、SDK 生成、Mock 服务

---

## 与竞品功能对比

| 功能 | raisfast Desktop | PocketBase Admin | Strapi Admin | Directus | TablePlus | Postman |
|---|---|---|---|---|---|---|
| 桌面应用 | ✅ Tauri | ❌ Web only | ❌ Web only | ❌ Web only | ✅ 原生 | ✅ Electron |
| Content-Type Builder | ✅ 可视化 | ❌ | ✅ | ✅ | — | — |
| 插件编辑器 | ✅ 内置 IDE | ❌ | ❌ | ❌ | — | — |
| 代码调试 | ✅ 控制台+SQL 日志 | ❌ | ❌ | ❌ | — | — |
| 数据浏览器 | ✅ 表格+表单 | ✅ 简单 | ✅ | ✅ | ✅ | — |
| SQL 查询 | ✅ | ❌ | ❌ | ✅ | ✅ | — |
| API 调试 | ✅ 内置 | ❌ | ❌ | ❌ | — | ✅ |
| Auth 管理 | ✅ | ✅ 简单 | ✅ | ✅ | — | — |
| RBAC 配置 | ✅ | ❌ | ✅ | ✅ | — | — |
| 文件管理 | ✅ | ✅ 简单 | ✅ | ✅ | — | — |
| 服务监控 | ✅ | ❌ | ❌ | ❌ | — | — |
| 一键发布 | ✅ | ❌ | ❌ | ❌ | — | — |
| CLI 工具 | ✅ | ✅ | ✅ | ✅ | — | — |
| 离线使用 | ✅ | ❌ | ❌ | ❌ | ✅ | ✅ |
| 零配置启动 | ✅ | ⚠️ 需命令行 | ❌ | ❌ | ⚠️ | ⚠️ |
| 单二进制 | ✅ | ✅ | ❌ | ❌ | ❌ | ❌ |

**唯一同时具备：桌面 IDE + CMS Builder + 插件开发 + API 调试 + 数据库管理 + 一键发布的产品。**
