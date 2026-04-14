# Tantivy 全文搜索方案

> 替代当前 SQL `LIKE '%keyword%'` 全表扫描，实现高性能、支持中文分词的全文搜索。

---

## 1. 现状分析

### 1.1 当前搜索实现

`models/post.rs` 中有 **4 处** `LIKE` 查询：

```sql
-- find_published 系列函数（带 JOIN 和不带 JOIN 各 2 处）
WHERE status = 'published' AND (title LIKE ? OR content LIKE ?)
-- pattern = format!("%{}%", q)
```

**问题：**

| 问题 | 影响 |
|------|------|
| 全表扫描 | 无法利用 B-Tree 索引，O(N) 复杂度 |
| 无分词 | `LIKE '%rust%'` 匹配子串，不支持精确词匹配 |
| 无中文支持 | `LIKE '%搜索%'` 无法匹配含"搜索引擎"的文章 |
| 无排序 | 结果按 `created_at` 排序，非相关性排序 |
| 无高亮 | 前端无法标记搜索词 |
| 重复查询两次 | 一次查数据，一次查 COUNT，搜索条件相同 |

### 1.2 已有基础设施

| 组件 | 状态 | 可复用程度 |
|------|------|-----------|
| `EventBus` | ✅ 已实现 | `PostCreated`/`PostUpdated`/`PostDeleted` 事件已广播 |
| `JobEnqueuer` | ✅ 已实现 | `PostCreated` 已自动入队 `RebuildSearchIndex` 任务 |
| `RebuildSearchIndexHandler` | ✅ 框架在 | 当前只打日志，TODO 占位待替换 |
| `Job` enum | ✅ 已定义 | `RebuildSearchIndex { post_ids: Vec<String> }` |
| Worker 系统 | ✅ 完整 | Cron + JobQueue + WorkerRunner 全部就绪 |

---

## 2. 技术选型

### 2.1 Tantivy vs 其他方案

| 维度 | Tantivy | SQLite FTS5 | Meilisearch |
|------|---------|-------------|-------------|
| 部署方式 | 嵌入式，进程内 | 嵌入式，SQLite 内 | 独立 HTTP 服务 |
| 额外依赖 | `tantivy` crate | 无 | Docker / 二进制 |
| 中文分词 | `tantivy-jieba` 或内置 CJK | `simple` tokenizer，效果差 | 内置中文 |
| 查询性能 | <1ms（内存索引） | ~10ms | ~5ms（网络开销） |
| 写入性能 | ~200MB/s | ~50MB/s | HTTP 开销 |
| 索引存储 | 文件系统目录 | SQLite 数据库内 | 独立数据目录 |
| Rust 原生 | ✅ 纯 Rust | SQL 层面 | ❌ SDK 调用 |
| 运维成本 | 零 | 零 | 高（独立进程监控） |
| 功能上限 | 高（自定义评分/分词/高亮） | 中 | 高 |

### 2.2 选型结论

**Tantivy** — 理由：

1. 纯 Rust，无 FFI 风险，与项目 `#![deny(unsafe_code)]` 兼容
2. 嵌入式，无需额外部署，与 SQLite "单文件"轻量哲学一致
3. 内置 CJK tokenizer 即可满足初步中文需求，后期可无缝升级 `tantivy-jieba`
4. 已有 Worker + EventBus 基础设施直接复用

---

## 3. 架构设计

### 3.1 模块结构

```
src/
├── search/
│   ├── mod.rs           ← SearchEngine trait + 公共类型
│   ├── tantivy.rs       ← TantivyEngine 实现
│   └── noop.rs          ← NoopSearchEngine（feature 关闭时的空实现）
```

### 3.2 SearchEngine Trait

```rust
// src/search/mod.rs

/// 搜索结果条目
pub struct SearchResult {
    pub post_id: String,
    pub score: f32,
    pub title_highlight: Option<String>,
    pub excerpt_highlight: Option<String>,
}

/// 搜索引擎接口
#[async_trait]
pub trait SearchEngine: Send + Sync {
    /// 索引单篇文章
    async fn index_post(&self, post: &SearchablePost) -> AppResult<()>;

    /// 批量索引多篇文章
    async fn index_posts(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// 删除文章索引
    async fn delete_post(&self, post_id: &str) -> AppResult<()>;

    /// 清空并重建全部索引
    async fn rebuild_all(&self, posts: &[SearchablePost]) -> AppResult<()>;

    /// 搜索文章
    async fn search(
        &self,
        query: &str,
        page: i64,
        page_size: i64,
    ) -> AppResult<(Vec<SearchResult>, i64)>;
}

/// 可索引的文章数据（从 DB 提取的扁平结构）
pub struct SearchablePost {
    pub id: String,
    pub title: String,
    pub content: String,
    pub excerpt: Option<String>,
    pub slug: String,
    pub status: String,
    pub published_at: Option<String>,
}
```

### 3.3 Tantivy 索引 Schema

```rust
// 3 个字段
let mut schema_builder = Schema::builder();

// post_id — STORED，用于查询后回表
schema_builder.add_text_field("post_id", STRING | STORED);

// title — 分词 + 存储，用于高亮
schema_builder.add_text_field(
    "title",
    TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("cjk")      // 内置 CJK tokenizer
                .set_index_option(IndexRecordOption::WithFreqsAndPositions)
        )
        .set_stored()
        .set_fast(None),
);

// content — 分词 + 存储，用于高亮
schema_builder.add_text_field(
    "content",
    TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("cjk")
                .set_index_option(IndexRecordOption::WithFreqsAndPositions)
        )
        .set_stored(),
);
```

**Tokenizer 选择：**

| 阶段 | Tokenizer | 说明 |
|------|-----------|------|
| 第一阶段 | `cjk`（内置） | 按字符 bigram 分词，零配置，支持中日韩 |
| 第二阶段（可选） | `tantivy-jieba` | 精确中文分词，需额外 ~10MB 词典 |

`cjk` tokenizer 示例：
- `"Rust编程语言"` → `["ru", "us", "st", "编", "程", "程语", "语言"]`
- 对中文按单字+双字组合索引，召回率高，精度可接受

### 3.4 异步适配

Tantivy 是同步 API，需用 `spawn_blocking` 桥接 tokio：

```rust
impl TantivyEngine {
    async fn index_post(&self, post: &SearchablePost) -> AppResult<()> {
        let index = self.index.clone();
        let post = post.clone();
        tokio::task::spawn_blocking(move || {
            let mut writer = index.writer(15_000_000)?; // 15MB 写缓冲
            writer.add_document(doc!(
                post_id_field => post.id.as_str(),
                title_field => post.title.as_str(),
                content_field => post.content.as_str(),
            ))?;
            writer.commit()?;
            Ok(())
        })
        .await?
    }
}
```

单篇写入耗时 <1ms，`spawn_blocking` 的线程切换开销可忽略。

---

## 4. 集成方案

### 4.1 Feature Flag

```toml
# Cargo.toml
[features]
default = ["db-sqlite", "plugin-all"]
search-tantivy = ["tantivy"]
search-jieba   = ["search-tantivy", "tantivy-jieba"]
```

### 4.2 AppState 注入

```rust
// src/lib.rs
pub struct AppState {
    // ... 现有字段
    #[cfg(feature = "search-tantivy")]
    pub search: Arc<dyn SearchEngine>,
    #[cfg(not(feature = "search-tantivy"))]
    pub search: Arc<NoopSearchEngine>,
}
```

### 4.3 索引同步

**实时同步**（`EventBus` → 直接写索引）：

```
PostCreated  → search.index_post(post)
PostUpdated  → search.index_post(post)   // 内部 delete + add
PostDeleted  → search.delete_post(id)
```

**全量重建**（复用已有 Worker）：

```
RebuildSearchIndexHandler::handle(job)
    → 从 DB 加载指定 post_ids
    → search.index_posts(posts)
```

**全量重建命令**（新增 CLI 或 API）：

```bash
cargo run -- db reindex-search   # 遍历 DB 所有已发布文章，重建索引
```

### 4.4 查询替换

`Repository` 层的 `find_published_joined` 和 `find_published`：

```rust
// 现有逻辑
if let Some(q) = q {
    // LIKE '%keyword%' → 全表扫描
}

// 替换为
if let Some(q) = q {
    // 1. 从 Tantivy 搜索得到 post_ids + 总数
    let (results, total) = state.search.search(q, page, page_size).await?;
    let post_ids: Vec<String> = results.iter().map(|r| r.post_id.clone()).collect();

    // 2. 从 DB 按 IDs 批量查询完整数据
    let posts = post::find_by_ids(&pool, &post_ids).await?;

    // 3. 按 Tantivy 评分排序
    let sorted = sort_by_search_order(posts, &results);

    (sorted, total)
}
```

需要新增 Model 函数：

```rust
// models/post.rs
pub async fn find_by_ids(pool: &Pool, ids: &[String]) -> AppResult<Vec<Post>>
pub async fn find_joined_by_ids(pool: &Pool, ids: &[String]) -> AppResult<Vec<PostJoinedRow>>
```

### 4.5 高亮支持

```rust
// TantivyEngine::search 返回 SearchResult 含高亮片段
pub struct SearchResult {
    pub post_id: String,
    pub score: f32,
    pub title_highlight: Option<String>,     // "<em>Rust</em> 编程入门"
    pub excerpt_highlight: Option<String>,   // "...学习 <em>Rust</em> 的最佳实践..."
}
```

前端可直接渲染高亮 HTML（`<em>` 标签），或 API 响应中单独返回。

---

## 5. 配置

### 5.1 `.env` 新增项

```bash
# 搜索引擎（tantivy | none）
SEARCH_ENGINE=tantivy

# Tantivy 索引目录
SEARCH_INDEX_DIR=./data/search_index

# Tantivy 写缓冲区大小（字节），默认 15MB
SEARCH_INDEX_BUFFER=15000000
```

### 5.2 AppConfig 新增

```rust
pub struct AppConfig {
    // ... 现有字段
    pub search_engine: String,
    pub search_index_dir: String,
    pub search_index_buffer: usize,
}
```

---

## 6. 依赖

```toml
[dependencies]
# 搜索引擎（可选）
tantivy = { version = "0.22", optional = true }
tantivy-jieba = { version = "0.11", optional = true }
```

| 依赖 | 编译体积 | 运行时 |
|------|---------|--------|
| `tantivy` | +~3MB | 索引文件 + 内存映射 |
| `tantivy-jieba` | +~1MB | 词典 ~10MB（首次加载） |

---

## 7. 迁移步骤

### Phase 1 — 核心实现（不含中文分词）

1. 新增 `src/search/mod.rs` — `SearchEngine` trait + `SearchResult` + `SearchablePost`
2. 新增 `src/search/tantivy.rs` — `TantivyEngine` 实现（CJK tokenizer）
3. 新增 `src/search/noop.rs` — `NoopSearchEngine` 空实现
4. `Cargo.toml` 添加 `search-tantivy` feature
5. `AppConfig` 新增搜索配置项
6. `AppState` 注入 `SearchEngine`
7. `server/mod.rs` 构建 `TantivyEngine` 或 `NoopSearchEngine`
8. 改造 `RebuildSearchIndexHandler` — 调用 `search.index_posts()`
9. 新增 `models/post.rs::find_by_ids()` / `find_joined_by_ids()`
10. 改造 `PostRepository` 查询 — 关键词搜索走 Tantivy
11. 新增 CLI 命令 `db reindex-search` — 全量重建
12. 补充单元测试 + 集成测试

### Phase 2 — 增量同步

1. EventBus 订阅者：`PostCreated`/`PostUpdated` → `search.index_post()`
2. EventBus 订阅者：`PostDeleted` → `search.delete_post()`
3. 启动时自动检测索引目录是否存在，首次启动自动全量索引

### Phase 3 — 中文分词（可选）

1. 添加 `search-jieba` feature flag
2. `TantivyEngine` 根据 feature 选择 `jieba` 或 `cjk` tokenizer
3. 全量重建索引（tokenizer 变更需要重索引）

---

## 8. 测试策略

| 测试类型 | 覆盖范围 | 方式 |
|---------|---------|------|
| 单元测试 | `TantivyEngine` 增删查 | 内存索引（`RamDirectory`） |
| 单元测试 | `NoopSearchEngine` 空操作 | 验证不 panic |
| 集成测试 | 搜索 API 端到端 | 通过 `/api/v1/posts?q=keyword` |
| 集成测试 | 索引同步 | EventBus emit → Worker 执行 → 搜索验证 |
| 性能测试 | 搜索延迟 | 10 万篇文章，P99 < 5ms |
| 回归测试 | 关闭 feature 时 fallback | `search-tantivy` 关闭 → LIKE 查询 |

---

## 9. 风险与缓解

| 风险 | 概率 | 缓解 |
|------|------|------|
| 索引文件损坏 | 低 | `reindex-search` 命令全量重建；定期备份 `data/search_index/` |
| `spawn_blocking` 线程池耗尽 | 低 | 单次搜索 <1ms，默认线程池 512 线程足够 |
| 磁盘空间 | 低 | 索引大小约为原文的 30%-50%，10 万篇文章约 50-100MB |
| CJK tokenizer 精度不够 | 中 | Phase 3 升级 `tantivy-jieba`；CJK bigram 召回率已足够 |
| 与 PostgreSQL/MySQL 兼容 | 无 | Tantivy 与 DB 引擎无关，`find_by_ids` 已用 `dialect::translate` |
| `tantivy` 版本升级 API 变更 | 中 | 锁定 0.22.x；升级时查看 changelog |
