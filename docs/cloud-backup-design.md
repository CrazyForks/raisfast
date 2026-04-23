# 云备份服务设计

> rust-blog Cloud Backup — 私有化数据的安全网
>
> 定位：**零配置、端到端加密、增量备份的云端备份服务**

---

## 1. 为什么需要这个功能

| 场景 | 后果 | 用户心理 |
|---|---|---|
| 硬盘损坏 | 所有客户数据丢失 | "我用私有化就是为了安全，结果数据反而丢了？" |
| 误删数据 | 重要商机/联系人丢失 | "有没有办法恢复？" |
| 勒索病毒 | 数据被加密，无法恢复 | "我愿意花钱恢复数据" |
| 服务器故障 | 业务中断 | "备份在哪里？" |
| 迁移新机器 | 需要手动复制数据 | "能不能一键迁移？" |

**私有化用户最大的焦虑 = 数据丢失没有兜底。** 解决这个焦虑 = 核心卖点 + 商业化入口。

---

## 2. 设计目标

| 目标 | 说明 |
|---|---|
| **零配置** | 注册账号 → 自动备份，不需要配置 S3/Azure |
| **端到端加密** | 数据在客户端加密，云端只存密文，服务端无法解密 |
| **增量备份** | 只传输变更部分，节省带宽和时间 |
| **版本历史** | 保留 N 个历史版本，支持回滚到任意时间点 |
| **自动调度** | 每小时/每天自动备份，无需手动操作 |
| **跨平台** | 桌面应用 + 服务器都支持 |
| **一键恢复** | 选一个版本 → 一键恢复到任意设备 |

---

## 3. 技术架构

### 3.1 整体流程

```
┌─────────────────────────────────────────────────────────────┐
│                     用户设备（客户端）                        │
│                                                             │
│  SQLite DB → 变更检测 → 增量计算 → AES-256加密 → 压缩       │
│                                                ↓            │
│                                         上传到备份服务器      │
│                                                ↓            │
│  备份记录（本地） ← 备份元数据 ← 备份服务器确认               │
└─────────────────────────────────────────────────────────────┘
                          ↓ 加密数据上传
┌─────────────────────────────────────────────────────────────┐
│                     备份服务器（云端）                        │
│                                                             │
│  接收加密块 → 存储到对象存储（S3/R2）→ 记录元数据             │
│                                                             │
│  存储内容：                                                  │
│  ├── 用户账号信息（邮箱、设备列表）                           │
│  ├── 备份元数据（时间、大小、版本号）                         │
│  └── 加密数据块（AES-256-GCM，服务端无法解密）               │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 端到端加密方案

```
密钥层级：

用户密码
  ↓ Argon2id
主密钥 (Master Key)
  ↓ HKDF-SHA256
  ├── 数据加密密钥 (DEK)  → 加密备份数据
  ├── 密钥加密密钥 (KEK)  → 加密存储 DEK
  └── 文件名加密密钥 (FEK) → 加密备份文件名

流程：
1. 注册时：用户密码 → Argon2id → 主密钥
2. 备份时：主密钥 → HKDF → DEK → AES-256-GCM 加密数据
3. 上传时：只传密文，服务端看不到明文
4. 恢复时：密码 → 重新派生主密钥 → 解密

安全保证：
- 服务端被黑客入侵 → 无法解密（没有用户密码）
- 备份文件泄露 → 无法解密（AES-256-GCM）
- 用户忘记密码 → 无法恢复（零知识架构）
```

### 3.3 增量备份方案

```
SQLite WAL 模式：

首次备份（全量）：
  SQLite 文件 → 分块（4MB/块）→ 每块计算 SHA-256 → 加密 → 上传

后续备份（增量）：
  1. 对比当前块 SHA-256 与上次备份的块列表
  2. 只上传 SHA 变化的块
  3. 生成新的快照清单（指向块列表）

  快照 A（全量）: [block-1, block-2, block-3, block-4]
  快照 B（增量）: [block-1, block-2, block-3-new, block-4]
                                       ↑ 只有这个变了

  存储开销：每次增量只存变化的块（通常 < 1MB）
```

### 3.4 SQLite 备份具体实现

```
方案选择：SQLite Backup API（sqlite3_backup_init）

Step 1: 创建临时 SQLite 副本
  → 使用 SQLite Backup API 将热数据库复制到临时文件
  → 确保数据一致性（不锁库）

Step 2: 计算块哈希
  → 临时文件按 4MB 分块
  → 计算每块 SHA-256

Step 3: 增量对比
  → 与上次备份的块列表对比
  → 识别变化的块

Step 4: 加密 + 压缩
  → 变化的块 → zstd 压缩 → AES-256-GCM 加密

Step 5: 上传
  → 加密块上传到备份服务器
  → 生成新快照清单

Step 6: 清理
  → 删除临时文件
  → 保留本地备份元数据
```

---

## 4. 备份服务端设计

### 4.1 技术选型

| 组件 | 选型 | 说明 |
|---|---|---|
| 备份服务器 | Rust (Axum) | 与 rust-blog 同技术栈，可独立部署 |
| 对象存储 | Cloudflare R2 | 免出站流量，S3 兼容 |
| 用户认证 | JWT | 简单可靠 |
| 数据库 | PostgreSQL | 存储用户/元数据 |
| 支付 | Stripe | 国际通用 |

### 4.2 API 设计

```
认证：
POST   /api/v1/auth/register          # 注册（邮箱+密码）
POST   /api/v1/auth/login             # 登录
POST   /api/v1/auth/token             # 刷新 Token

设备管理：
GET    /api/v1/devices                 # 列出设备
POST   /api/v1/devices                 # 注册设备
DELETE /api/v1/devices/:id             # 移除设备

备份：
POST   /api/v1/backups/upload          # 上传备份块
POST   /api/v1/backups/snapshot        # 创建快照
GET    /api/v1/backups/snapshots        # 列出快照
GET    /api/v1/backups/snapshots/:id    # 获取快照详情
DELETE /api/v1/backups/snapshots/:id    # 删除快照

恢复：
POST   /api/v1/backups/restore/:id     # 获取恢复所需的块列表
GET    /api/v1/backups/blocks/:hash    # 下载指定块

订阅：
GET    /api/v1/subscription            # 当前订阅状态
POST   /api/v1/subscription/checkout   # Stripe Checkout
POST   /api/v1/subscription/webhook    # Stripe Webhook
```

### 4.3 数据模型

```sql
-- 用户
CREATE TABLE users (
    id          UUID PRIMARY KEY,
    email       TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,        -- Argon2id
    created_at  TIMESTAMPTZ NOT NULL
);

-- 设备
CREATE TABLE devices (
    id          UUID PRIMARY KEY,
    user_id     UUID REFERENCES users(id),
    name        TEXT NOT NULL,           -- "MacBook Pro" / "Server-01"
    device_key  TEXT NOT NULL,           -- 设备公钥指纹
    created_at  TIMESTAMPTZ NOT NULL
);

-- 备份快照
CREATE TABLE snapshots (
    id          UUID PRIMARY KEY,
    device_id   UUID REFERENCES devices(id),
    snapshot_no BIGINT NOT NULL,         -- 快照序号
    size_bytes  BIGINT NOT NULL,         -- 原始大小
    block_count INT NOT NULL,            -- 块数量
    created_at  TIMESTAMPTZ NOT NULL
);

-- 备份块（去重）
CREATE TABLE blocks (
    hash        TEXT PRIMARY KEY,        -- SHA-256
    size_bytes  INT NOT NULL,
    storage_key TEXT NOT NULL,            -- S3/R2 对象键
    created_at  TIMESTAMPTZ NOT NULL
);

-- 快照 → 块映射
CREATE TABLE snapshot_blocks (
    snapshot_id UUID REFERENCES snapshots(id),
    block_hash  TEXT REFERENCES blocks(hash),
    block_index INT NOT NULL,             -- 块序号
    PRIMARY KEY (snapshot_id, block_index)
);

-- 订阅
CREATE TABLE subscriptions (
    id              UUID PRIMARY KEY,
    user_id         UUID REFERENCES users(id),
    plan            TEXT NOT NULL,         -- free/pro/enterprise
    storage_limit   BIGINT NOT NULL,       -- 存储上限（字节）
    stripe_id       TEXT,                  -- Stripe 订阅 ID
    status          TEXT NOT NULL,         -- active/canceled
    created_at      TIMESTAMPTZ NOT NULL
);
```

---

## 5. 客户端集成

### 5.1 桌面应用集成

```
设置页面：
┌─────────────────────────────────────────────┐
│  云备份设置                                   │
│                                              │
│  账号：chris@example.com       [登录/注册]    │
│  状态：✅ 已连接  上次备份：3 分钟前            │
│                                              │
│  备份频率：                                    │
│  ○ 每小时  ● 每天  ○ 每周                     │
│                                              │
│  备份历史：                                    │
│  ┌─────────────────────────────────────────┐ │
│  │ 2024-04-23 14:30  12.5 MB  [恢复] [删除]│ │
│  │ 2024-04-22 14:30  12.3 MB  [恢复] [删除]│ │
│  │ 2024-04-21 14:30  11.8 MB  [恢复] [删除]│ │
│  └─────────────────────────────────────────┘ │
│                                              │
│  恢复到其他设备：                               │
│  [选择快照] → [生成恢复码] → 在新设备输入恢复码    │
│                                              │
│  [立即备份]                                    │
└─────────────────────────────────────────────┘
```

### 5.2 CLI 集成

```bash
# 登录
rust-backup login --email chris@example.com

# 手动备份
rust-backup backup --project ./my-project

# 查看备份历史
rust-backup snapshots --project ./my-project

# 恢复到指定版本
rust-backup restore --snapshot abc-123 --output ./restored-project

# 恢复到新设备（使用恢复码）
rust-backup restore --code ABCD-1234-EFGH-5678 --output ./my-project

# 生成恢复码（在其他设备上使用）
rust-backup recovery-code --snapshot abc-123
```

### 5.3 Plugin API（可在插件中触发备份）

```javascript
// 在插件中触发备份（如创建重要数据后）
Plugin.on_content_created = function(input) {
    const data = parseBody(input);
    if (data.content_type === "deal" && data.stage === "closed_won") {
        // 赢单后自动备份
        Host.backup();
    }
    return ok(data);
};
```

---

## 6. 定价

| 套餐 | 存储空间 | 保留版本 | 设备数 | 价格 |
|---|---|---|---|---|
| **免费** | 1 GB | 7 天 | 1 台 | $0 |
| **Pro** | 50 GB | 90 天 | 5 台 | **$9.9/月** |
| **Team** | 500 GB | 365 天 | 无限 | **$29/月** |
| **Enterprise** | 无限 | 无限 | 无限 | **$99/月** |

### 6.1 成本估算

| 项目 | 单价 | 1000 个 Pro 用户成本 |
|---|---|---|
| Cloudflare R2 存储 | $0.015/GB/月 | $750（50GB × 1000 × $0.015） |
| R2 上传（Class A 操作） | $4.50/百万次 | ~$50 |
| R2 下载（恢复） | 免费（R2 无出站费） | $0 |
| 服务器（1 台） | $20/月 | $20 |
| **总成本** | | **~$820/月** |
| **收入** | $9.9 × 1000 | **$9,900/月** |
| **利润率** | | **~92%** |

---

## 7. 开发路线

### Phase 1 — 核心备份（1-2 月）

| 任务 | 说明 |
|---|---|
| 备份服务端 | Rust + Axum，用户注册/登录/上传/下载 |
| 端到端加密 | AES-256-GCM + Argon2id 密钥派生 |
| 增量备份引擎 | SQLite 分块 + SHA-256 对比 + zstd 压缩 |
| 桌面应用集成 | 设置页面 + 自动调度 + 状态显示 |
| CLI 命令 | `rust-backup login/backup/restore` |

### Phase 2 — 恢复体验（2-3 月）

| 任务 | 说明 |
|---|---|
| 一键恢复 | 选择快照 → 一键恢复 |
| 恢复码 | 跨设备恢复（类似恢复短语） |
| 版本对比 | 对比两个快照的差异 |
| 选择性恢复 | 只恢复特定表/数据 |

### Phase 3 — 高级功能（3-6 月）

| 任务 | 说明 |
|---|---|
| 团队功能 | 多设备管理、团队共享备份 |
| Point-in-time | 恢复到精确时间点 |
| 备份监控 | 备份失败告警、异常检测 |
| 合规报告 | 备份报告生成（企业审计用） |

---

## 8. 商业价值

### 8.1 为什么备份服务是最佳商业化入口

| 优势 | 说明 |
|---|---|
| **刚需** | 每个私有化用户都需要备份 |
| **持续付费** | 按月/年订阅，不是一次性购买 |
| **高粘性** | 备份数据在云端，用户不会轻易离开 |
| **高利润率** | 存储成本极低（~8%），利润率 90%+ |
| **信任入口** | 用户把数据交给你备份 → 建立信任 → 后续卖更多服务 |
| **天然获客** | 桌面应用内提示"开启云备份" → 零成本转化 |

### 8.2 与主产品的协同

```
rust-blog 开源版（免费）
  ↓ 用户使用，积累数据
  ↓ 看到备份提示
rust-backup 云备份（付费）
  ↓ 用户付费备份
  ↓ 建立信任
  ↓ 推荐其他付费服务
rust-blog 企业插件包（付费）
rust-blog 行业模板包（付费）
rust-blog 技术支持（付费）
```

**备份是整个商业化飞轮的起点。**
