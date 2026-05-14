# All-in-One 功能规划

> raisfast 的核心原则：能自己解决的尽量不依赖第三方。一个二进制 = 完整的应用平台。

---

## 1. 已有能力（不重复造轮子）

| 能力 | 实现方式 | 状态 |
|------|---------|------|
| HTTP 反向代理 | 内置 proxy 模块（`docs/proxy-design.md`） | 设计完成，待实现 |
| TLS/HTTPS | rustls（tls feature） | ✅ 已完成 |
| 邮件发送 | lettre（SMTP/SendGrid/Resend/阿里云/腾讯云） | ✅ 已完成 |
| 短信通知 | 阿里云/Twilio | ✅ 已完成 |
| 全文搜索 | Tantivy 内置（search-tantivy feature） | ✅ 已完成 |
| 定时任务 | SQLite worker 内置（14 个 job handler） | ✅ 已完成 |
| 支付 | Alipay/Stripe/WeChat/Creem/Dodo（5 家渠道） | ✅ 已完成 |
| 对象存储 | Local + S3（storage-s3 feature） | ✅ 已完成 |
| 插件系统 | JS（rquickjs）/ Lua（mlua）/ Rhai / WASM | ✅ 已完成 |
| OAuth2 | GitHub / Google / WeChat | ✅ 已完成 |
| 模板引擎 | Tera | ✅ 已完成 |
| Markdown | comrak | ✅ 已完成 |
| API 文档 | utoipa + Swagger UI | ✅ 已完成 |
| GraphQL | async-graphql | ✅ 已完成 |
| WebSocket / SSE | axum ws + tokio broadcast | ✅ 已完成 |
| RBAC 权限 | 角色 + 权限 + 多租户 | ✅ 已完成 |
| 审计日志 | eventbus + audit_log 表 | ✅ 已完成 |
| Webhook | 出站 webhook + 签名验证 | ✅ 已完成 |
| 工作流引擎 | workflow 模块 | ✅ 已完成 |
| 国际化 | rust-i18n + locale 中间件 | ✅ 已完成 |
| 图片上传 | image crate（JPEG/PNG/GIF/WebP） | ✅ 已完成 |

---

## 2. 第一梯队：立刻做（投入小产出大）

### 2.1 图片处理（缩略图 / 裁剪 / 水印）

**现状**：上传原图后，前端或外部工具（Sharp/ImageMagick/Cloudinary）做缩略图。
**目标**：上传时自动生成多尺寸缩略图，支持 URL 参数实时裁剪。

```
GET /uploads/photo.jpg?w=300&h=200&fit=cover   → 实时裁剪
GET /uploads/photo.jpg?w=300                    → 等比缩放
```

**实现要点**：
- `image` crate 已在依赖中，零新依赖
- 上传时预设尺寸（`media.sizes = ["thumb", "medium", "large"]`）
- 可选：URL 参数实时裁剪（tower 中间件拦截）
- 可选：文字/图片水印

**预估工作量**：2 天

---

### 2.2 邮件模板引擎

**现状**：邮件内容硬编码在 Rust 代码中，修改需要重新编译。
**目标**：用 Tera 模板渲染邮件 HTML，支持管理员在 Admin UI 中编辑模板。

```
templates/email/
├── welcome.html          # 欢迎邮件
├── reset-password.html   # 密码重置
├── verify-email.html     # 邮箱验证
├── order-confirmed.html  # 订单确认
└── notification.html     # 通用通知
```

**实现要点**：
- Tera 已在依赖中，零新依赖
- 模板变量：`{{ user.username }}`、`{{ site.name }}`、`{{ link }}` 等
- 邮件模板存数据库（options 表）或文件系统
- Admin UI 可编辑预览

**预估工作量**：1 天

---

### 2.3 自动定时备份

**现状**：`raisfast db backup` CLI 已有，但需要外部 cron 手动触发。
**目标**：worker 系统自动定时备份，管理员在 Admin UI 配置。

**实现要点**：
- 已有 worker job `db_backup`，已注册在 `cron_schedules` 默认配置中
- 确认 worker 系统是否正确调度该 job
- 备份保留策略：保留最近 N 个（`backup_retention` 已在 AppConfig 中）
- Admin UI 展示备份列表、一键恢复

**预估工作量**：半天（验证现有实现 + Admin UI）

---

### 2.4 缓存预热

**现状**：首次访问冷启动慢，需要手动 curl 或等用户触发。
**目标**：部署/重启后自动预热缓存，保证首次访问即快速。

**实现要点**：
- worker job：解析 sitemap.xml → 并发请求所有页面
- 触发时机：启动后自动执行 / Admin API 手动触发
- 预热目标：首页、列表页、热门文章

**预估工作量**：1 天

---

### 2.5 IP 地理解析

**现状**：无法知道用户地理位置，日志只有 IP。
**目标**：内置 GeoIP 数据库，支持按国家/城市统计、地域限流。

**实现要点**：
- 引入 `maxmind-db` crate（轻量，读取 mmdb 文件）
- GeoLite2 免费数据库：~5MB（国家/城市精度）
- 下载地址：`https://cdn.jsdelivr.io/npm/geolite2-city/GeoLite2-City.mmdb`
- 用途：访问统计按地域分布、评论 IP 归属地展示

**预估工作量**：1 天

---

## 3. 第二梯队：差异化竞争力（做了能卖钱）

### 3.1 LLM Gateway（AI 网关）

**现状**：用户直调 OpenAI API，无统一管理。
**目标**：内置多渠道 LLM 转发网关，统一 API、计费、限流。

**详细设计**：见 `docs/llm-gateway-reference.md`

**核心功能**：
- 多上游渠道（OpenAI / Claude / Gemini / 国产模型）
- API Key 管理 + 用量计费
- SSE 流式转发
- 模型路由（按能力/价格自动选择）
- 请求日志 + 审计

**预估工作量**：1-2 周

---

### 3.2 表单收集（Contact Form / Lead Collection）

**现状**：用户需要 Typeform / 自建前端表单。
**目标**：Content Type 系统扩展，支持公开提交表单 + 邮件通知。

**示例场景**：
- 联系我们表单（姓名/邮箱/消息）
- 报名表单（姓名/手机/选择课程）
- 反馈收集（评分/文字/截图）

**实现要点**：
- Content Type 增加 `public_submit: true` 配置
- 新增 `POST /api/v1/cms/{plural}/submit` 公开 API（无需登录）
- 提交后触发事件 → 邮件通知管理员
- 可选：reCAPTCHA / honeypot 防垃圾
- Admin UI 查看提交列表

**预估工作量**：3 天

---

### 3.3 站内通知中心

**现状**：没有站内信，通知只能通过邮件。
**目标**：notifications 表 + SSE 实时推送 + Admin UI 通知铃铛。

**实现要点**：
- 新增 `notifications` 表（id, user_id, type, title, body, read, created_at）
- SSE 已有基础设施，直接复用
- 事件触发：评论通知、订单状态、系统公告
- Admin UI：通知铃铛 + 未读数 + 通知列表
- API：`GET /api/v1/notifications`、`PUT /api/v1/notifications/{id}/read`、`PUT /api/v1/notifications/read-all`

**预估工作量**：3 天

---

### 3.4 SEO 分析器

**现状**：需要 Google Search Console / Ahrefs 等外部工具。
**目标**：内置 SEO 评分，发布文章时自动检查并给建议。

**检查项**：
| 检查项 | 规则 | 权重 |
|--------|------|------|
| 标题长度 | 30-60 字符 | 10% |
| Meta Description | 120-160 字符 | 10% |
| H1 标签 | 有且仅有一个 | 10% |
| 图片 ALT | 所有图片都有 alt | 10% |
| URL 友好 | 短横线分隔，无特殊字符 | 10% |
| 关键词密度 | 正文出现 2-5 次 | 15% |
| 内链 | 至少 2 个内部链接 | 10% |
| OG 标签 | og:title/og:description/og:image | 15% |
| Canonical | 有 canonical URL | 10% |

**实现要点**：
- 纯 Rust 实现，无外部依赖
- Content Type 保存前 hook 触发检查
- Admin UI 展示评分 + 改进建议
- 可选：定时批量检查全站 SEO

**预估工作量**：3 天

---

### 3.5 邮件营销（批量发送 / 模板 / 跟踪）

**现状**：需要 Mailchimp / SendGrid Marketing。
**目标**：内置邮件营销，支持订阅者管理 + 模板 + 批量发送 + 打开跟踪。

**实现要点**：
- 新增 `subscribers` 表（email, name, status, tags, subscribed_at）
- 新增 `campaigns` 表（subject, template, status, sent_at, open_count, click_count）
- Tera 模板渲染邮件内容
- lettre 批量发送（分批 + 间隔，避免被封）
- 打开跟踪：1x1 像素 `<img src="/api/v1/email/track/{id}.gif">`
- 点击跟踪：链接替换为 `/api/v1/email/click/{id}?url=...`
- 退订：邮件底部退订链接

**预估工作量**：1 周

---

## 4. 第三梯队：锦上添花（长期目标）

### 4.1 双因素认证 (2FA)

**现状**：仅密码登录。
**方案**：TOTP 实现（Google Authenticator 兼容）。

- `totp-lite` 或自己实现（RFC 6238，~100 行代码）
- 用户绑定：生成 secret + QR 码
- 登录验证：密码 + 6 位动态码
- 恢复码：生成 10 个一次性恢复码

**预估工作量**：1 天

---

### 4.2 短链服务

**现状**：需要 Bitly 或自建。
**方案**：内置短链生成 + 301 重定向 + 点击统计。

- 新增 `short_links` 表（code, target_url, click_count, created_at）
- 生成算法：Base62 编码（`[a-zA-Z0-9]`）
- API：`POST /api/v1/short-links`、`GET /s/{code}` → 301
- 统计：每次点击记录 IP/UA/来源

**预估工作量**：2 天

---

### 4.3 数据导入 / 导出

**现状**：手动 SQL 或写脚本。
**方案**：JSON/CSV 批量导入导出 API。

- 导出：`GET /api/v1/{plural}/export?format=json|csv`
- 导入：`POST /api/v1/{plural}/import`（上传 JSON/CSV 文件）
- WordPress 导入器：解析 WXR/XML
- 验证：导入前校验数据格式，失败跳过并报告

**预估工作量**：3 天

---

### 4.4 日志分析仪表盘

**现状**：需要 ELK / Grafana。
**方案**：SQLite 聚合查询 + Admin UI 看板。

- 请求统计：按时间/路径/状态码/租户聚合
- 慢请求 Top N
- 错误率趋势
- UV/PV 统计
- 实时流量（SSE 推送）

**预估工作量**：1 周

---

### 4.5 A/B 测试

**现状**：需要 Optimizely / 自建。
**方案**：Content Type 多版本 + 流量分配中间件。

- 新增 `ab_tests` 表（name, variants, traffic_split, status）
- 每个请求根据 user_id hash 分配到变体
- 统计每个变体的转化率
- Admin UI 配置测试 + 查看结果

**预估工作量**：1 周

---

## 5. 总体优先级排序

| 优先级 | 功能 | 工作量 | 商业价值 |
|--------|------|--------|---------|
| **P0** | 图片处理 | 2 天 | 高（CMS 刚需） |
| **P0** | 邮件模板 | 1 天 | 高（运营刚需） |
| **P0** | 自动定时备份验证 | 0.5 天 | 高（数据安全） |
| **P1** | LLM Gateway | 1-2 周 | 极高（商业壁垒） |
| **P1** | 站内通知中心 | 3 天 | 高（用户体验） |
| **P1** | 表单收集 | 3 天 | 高（低代码核心） |
| **P1** | SEO 分析器 | 3 天 | 中（差异化） |
| **P2** | 邮件营销 | 1 周 | 高（变现手段） |
| **P2** | 双因素认证 | 1 天 | 中（安全增强） |
| **P2** | 缓存预热 | 1 天 | 低（性能优化） |
| **P2** | IP 地理解析 | 1 天 | 低（增值功能） |
| **P3** | 短链服务 | 2 天 | 低（营销工具） |
| **P3** | 数据导入导出 | 3 天 | 中（迁移便利） |
| **P3** | 日志分析仪表盘 | 1 周 | 中（运维便利） |
| **P3** | A/B 测试 | 1 周 | 低（高级功能） |

---

## 6. 依赖关系

```
邮件模板 ──→ 邮件营销 ──→ 打开/点击跟踪
    │
    └──→ 表单收集（提交通知）
    
站内通知中心 ──→ 所有业务事件（评论/订单/系统）

图片处理 ──→ 上传流程扩展

LLM Gateway ──→ 独立模块，无前置依赖

SEO 分析器 ──→ 依赖 Content Type 系统（已有）

2FA ──→ 依赖 Auth 系统（已有）
```

**建议实施顺序**：

```
第一周：图片处理 + 邮件模板 + 备份验证
第二周：站内通知中心
第三周：表单收集 + SEO 分析器
第四~五周：LLM Gateway
第六周：邮件营销
后续：按需实现第三梯队
```
