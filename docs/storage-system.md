# 分布式文件存储系统设计

## 需求分析

### 当前项目存储场景

| 场景 | 文件类型 | 频率 | 大小 |
|---|---|---|---|
| 博客封面图 | jpg/png/webp | 低 | 100KB-5MB |
| 编辑器内嵌图片 | jpg/png/gif | 中 | 50KB-2MB |
| 编辑器内嵌视频 | mp4 | 低 | 10MB-500MB |
| 附件下载 | pdf/zip | 低 | 1MB-50MB |
| 电商 Extension 商品图 | jpg/png | 中 | 50KB-2MB |
| 用户头像 | jpg/png | 低 | 50KB-500KB |

### 核心诉求

```
单机部署 → 多机扩展，代码零改动
```

## 三种方案对比

### 方案 A：直接集成 RustFS（外部服务）

```
[Blog Server] → S3 API → [RustFS Cluster]
```

- 单机也要跑 RustFS 容器，最小编译 40MB+ 二进制
- 引入外部运维复杂度（单机博客跑个分布式存储，过度工程）
- 调试链路长

### 方案 B：从零造轮子（自研分布式存储）

- 工作量巨大（一致性协议、数据分片、副本、故障恢复）
- 不现实，也没必要

### 方案 C：抽象存储层 + 可插拔后端（推荐）

```
                    ┌─────────────────┐
                    │  StorageService  │  ← 统一 API
                    │  (trait Storage) │
                    └────────┬────────┘
                             │
                ┌────────────┼────────────┐
                ▼            ▼            ▼
          LocalFS       RustFS/S3    <future>
         (单机)        (分布式)      OSS/R2
```

**Phase 1 — 单机**：文件存本地磁盘，零外部依赖，零运维

**Phase 2 — 扩展**：改一行配置切换到 RustFS/S3，代码零改动

## 推荐架构（方案 C 详细设计）

### 存储 Trait

```rust
// src/storage/mod.rs
#[async_trait]
pub trait Storage: Send + Sync {
    async fn put(&self, key: &str, data: &[u8], content_type: &str) -> AppResult<()>;
    async fn get(&self, key: &str) -> AppResult<Vec<u8>>;
    async fn delete(&self, key: &str) -> AppResult<()>;
    async fn url(&self, key: &str) -> AppResult<String>;       // 读取 URL
    async fn presigned_upload(&self, key: &str, ttl: Duration) -> AppResult<String>;
}
```

### 两个实现

| | LocalFS | S3Storage |
|---|---|---|
| 存储 | `{DATA_DIR}/uploads/` | RustFS / MinIO / S3 / R2 |
| URL | `/uploads/{bucket}/{key}` (Axum 静态文件) | presigned URL 或 CDN |
| 配置 | `STORAGE_DRIVER=local` | `STORAGE_DRIVER=s3` |
| 依赖 | 无 | `aws-sdk-s3` (feature gate) |

### 上传流程

```
前端 ──POST /api/v1/upload──▶ Axum Handler
                                    │
                              StorageService.put()
                                    │
                              ┌─────┴──────┐
                              │  LocalFS   │  S3Storage
                              │  写磁盘     │  PUT object
                              └─────┬──────┘
                                    │
                              返回 { url: "/uploads/blog/xxx.jpg" }
```

### 文件组织

```
{bucket}/{year}/{month}/{uuid}.{ext}
  blog/    /2026/  /04/  /a1b2c3d4.jpg      ← 博客图片
  avatar/  /2026/  /04/  /e5f6g7h8.png      ← 头像
  product/ /2026/  /04/  /i9j0k1l2.webp     ← 电商商品图
  attachment/ ...                              ← 附件
```

### Feature Flags

```toml
[features]
storage-local = []           # 默认
storage-s3 = ["aws-sdk-s3"]  # 分布式时启用
```

### 环境变量配置

```bash
# 通用
STORAGE_DRIVER=local          # local | s3

# S3 模式（RustFS / MinIO / AWS S3 / Cloudflare R2）
S3_ENDPOINT=http://rustfs:9000
S3_ACCESS_KEY=xxx
S3_SECRET_KEY=xxx
S3_BUCKET=blog
S3_REGION=us-east-1
```

### 扩展路径

```
现在：  STORAGE_DRIVER=local    → 本地磁盘
       ↓
将来：  STORAGE_DRIVER=s3       → RustFS 单节点
       S3_ENDPOINT=http://rustfs:9000
       S3_ACCESS_KEY=xxx
       S3_SECRET_KEY=xxx
       S3_BUCKET=blog
       ↓
集群：  RustFS 多节点 + Nginx/CDN → 全球分发
```

## 实施计划

| 阶段 | 内容 | 工作量 |
|---|---|---|
| **P1** | Storage trait + LocalFS 实现 + 上传/下载 API + 前端对接 | 1-2 天 |
| **P2** | S3Storage 实现 + feature flag + presigned URL | 1 天 |
| **P3** | 图片处理（缩略图、WebP 转换） | 1 天 |
| **P4** | docker-compose 加 RustFS + 生产配置 | 半天 |

## 外部存储引擎评估

### RustFS

- Rust 编写，与项目同语言
- S3 兼容 API
- Apache 2.0 许可，商业友好
- 比 MinIO 快 2.3x（4KB 小文件）
- 26k+ GitHub Stars，社区活跃
- Docker 一行部署
- 支持分布式模式、Bitrot 保护、版本控制、桶复制
- 部分功能（生命周期管理、分布式模式）仍在测试中

### MinIO

- Go 编写，成熟稳定
- S3 兼容 API，生态最全
- AGPL v3 许可，商用有法律风险
- 生产级分布式部署经验丰富

### Garage

- Rust 编写，轻量级 S3 兼容
- 去中心化架构，适合资源有限场景
- 社区规模较小

### 托管服务

| 服务 | 特点 |
|---|---|
| Cloudflare R2 | S3 兼容，无出站流量费，自带 CDN |
| AWS S3 | 行业标准，生态最全 |
| 阿里云 OSS / 腾讯 COS | 国内延迟低 |

## 结论

推荐 **方案 C（抽象存储层）+ RustFS 作为 S3 后端**：

1. 抽象层解耦，单机用 LocalFS 零依赖，集群切 S3 零改代码
2. RustFS 技术栈统一（Rust）、许可证友好（Apache 2.0）、性能领先
3. 分阶段实施，P1 即可投入使用
