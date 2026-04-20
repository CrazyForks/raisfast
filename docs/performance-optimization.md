# 性能优化方案

> 基于 20 并发压力测试的瓶颈分析，覆盖中间件、数据库、CMS 引擎三个层面。
> 当前测试环境：macOS 单机，dev 编译，SQLite 本地文件。

## 当前基准性能

```
20 并发 × 5 秒，httpx Python 客户端

原生 GET /posts (列表)      1,046 RPS   P50=18.5ms
CMS   GET /cms/articles     531  RPS    P50=38.9ms
原生 GET /posts/{slug}      1,047 RPS   P50=18.0ms
CMS   GET /cms/articles/{id} 1,043 RPS  P50=18.3ms
原生 POST /posts            287  RPS    P50=66.0ms
CMS   POST /cms/articles    313  RPS    P50=54.4ms
```

## 请求热路径

```
HTTP 请求到达
  │
  ├─ CORS                              ~0μs
  ├─ TraceLayer (日志 span)             ~1μs
  ├─ Request ID (UUID v7)               ~1μs
  ├─ Metrics (计时器)                   ~1μs
  ├─ Locale (Header 解析)               ~5μs
  │
  ├─ ★ Rate Limit (全局 Mutex + O(N))   ~10-100μs    ← 瓶颈 #1
  │
  ├─ JWT 验证 (每次新建 DecodingKey)     ~10-50μs     ← 瓶颈 #2
  │
  ├─ CMS 路由额外:
  │   ├─ Registry.get() (std::sync::RwLock)  ~1-5μs   ← 瓶颈 #3
  │   ├─ SQL 1: COUNT(*)              ~500-2000μs
  │   ├─ SQL 2: SELECT ... LIMIT      ~1000-5000μs
  │   └─ Relation 解析 (额外 N 次查询)           ← 瓶颈 #4
  │
  └─ 原生路由额外:
      ├─ Cache RwLock 读               ~1μs (命中时)
      ├─ SQL 1: LEFT JOIN 主查询       ~1000-5000μs
      ├─ SQL 2: COUNT(*)               ~500-2000μs
      └─ SQL 3: 批量查 tags            ~1000-3000μs
```

---

## 瓶颈 #1：Rate Limit 全局 Mutex + O(N) retain

**位置**：`src/middleware/rate_limit.rs:77-95`

**现状**：

```rust
struct MemoryStore {
    entries: tokio::sync::Mutex<HashMap<String, Entry>>,
}

async fn check(&self, key: &str, config: &RateLimitConfig) -> bool {
    let mut entries = self.entries.lock().await;  // 所有请求竞争一把锁
    entries.retain(|_, entry| { ... });           // O(N) 全量遍历清理
    let entry = entries.entry(key.to_string())    // 每次分配 String
        .or_insert(Entry { count: 0, window_start: now });
    // ...
}
```

**问题**：

| 问题 | 影响 |
|------|------|
| 单一 `tokio::sync::Mutex` | 所有 API 请求串行化，高并发下排队 |
| `retain()` 每次全量遍历 | O(N)，N = 活跃 IP 数，随流量线性增长 |
| `key.to_string()` 每次分配 | 即使 key 已存在也分配新 String |
| 锁持有期间做清理 | 阻塞所有其他请求 |

**方案**：

1. 替换为 `DashMap<String, Entry>`（分片锁，16 个分片）
2. 去掉 `retain()`，改为惰性过期（检查时判断单条是否过期）
3. 用 `entry()` API 避免 key 重复分配
4. 后台定时任务清理过期条目（已有 300 秒清理，保留）

**预估提升**：RPS +100~200%

---

## 瓶颈 #2：JWT DecodingKey 每次创建

**位置**：`src/services/auth.rs:134-145`

**现状**：

```rust
pub fn verify_token(token: &str, secret: &str) -> AppResult<Claims> {
    jsonwebtoken::decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),  // 每次分配+拷贝
        &Validation::default(),
    )
    // ...
}
```

**方案**：

启动时创建 `DecodingKey` 存入 `AppState`：

```rust
pub struct AppState {
    pub jwt_decoding_key: DecodingKey,  // 新增
    // ...
}
```

**预估提升**：-10~30μs/请求

---

## 瓶颈 #3：ContentTypeRegistry 用 std::sync::RwLock

**位置**：`src/content_type.rs:48`

**现状**：

```rust
pub struct ContentTypeRegistry {
    inner: std::sync::RwLock<HashMap<String, Arc<ContentTypeSchema>>>,
}
```

**问题**：
- `std::sync::RwLock` 在 tokio 异步运行时中会阻塞线程
- 如果有写操作（注册新 Content Type），读端也会被阻塞
- 每次 CMS 请求都需要获取读锁

**方案**：

替换为 `arc-swap::ArcSwap<HashMap<...>>`：

```rust
pub struct ContentTypeRegistry {
    inner: ArcSwap<HashMap<String, Arc<ContentTypeSchema>>>,
}

// 读：无锁，直接 load()
fn get(&self, name: &str) -> Option<Arc<ContentTypeSchema>> {
    self.inner.load().get(name).cloned()
}

// 写：替换整个 map
fn register(&self, schema: ContentTypeSchema) {
    let mut new_map = self.inner.load().as_ref().clone();
    new_map.insert(schema.name.clone(), Arc::new(schema));
    self.inner.store(Arc::new(new_map));
}
```

**预估提升**：消除尾部延迟，CMS 请求 -1~5μs

---

## 瓶颈 #4：OneToMany/ManyToMany 的 N+1 查询

**位置**：`src/content_type/resolver.rs:53-63`

**现状**：

```rust
// OneToMany — 每个 item 单独查询
for item in items.iter_mut() {
    resolve_one_to_many(pool, ct, field.name, rel, item).await?;
}

// ManyToMany — 同上
for item in items.iter_mut() {
    resolve_many_to_many(pool, ct, field.name, rel, item).await?;
}
```

20 条记录 × 1 个 OneToMany 字段 = 20 次额外 SQL 查询。

**方案**：

改为和 ManyToOne 一样的批量模式：

```
OneToMany:  收集所有 item 的 id → WHERE fk_col IN (id1, id2, ...) → 按 fk_col 分组 → 分发回各 item
ManyToMany: 收集所有 item 的 id → WHERE junction.source_id IN (...) JOIN target → 按 source_id 分组 → 分发
```

**预估提升**：N 次查询 → 1 次查询，CMS 列表 -50%

---

## 瓶颈 #5：SQLite PRAGMA 未优化

**位置**：`src/db/connection.rs:33-43`

**现状**：

```rust
.after_connect(|conn, _meta| {
    sqlx::query("PRAGMA journal_mode = WAL").execute(&mut *conn).await?;
    sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *conn).await?;
    Ok(())
})
```

**缺少的 PRAGMA**：

| PRAGMA | 当前值 | 建议值 | 说明 |
|--------|--------|--------|------|
| `busy_timeout` | 未设置（默认 0） | `5000` | 写冲突时等待 5 秒而非立即报错 |
| `synchronous` | `FULL`（默认） | `NORMAL` | WAL 模式下 NORMAL 足够安全，性能提升显著 |
| `cache_size` | `-2000`（~2MB，默认） | `-64000`（64MB） | 增大页缓存，减少磁盘 IO |
| `temp_store` | `DEFAULT` | `MEMORY` | 临时表在内存中，排序/聚合更快 |
| `mmap_size` | `0`（默认） | `268435456`（256MB） | 内存映射 IO，减少系统调用 |

**预估提升**：SQL 延迟 -30~50%

---

## 瓶颈 #6：CMS 列表无缓存

**现状**：

- 原生 `/posts` 走 `CachedPostRepository`（内存缓存，命中时 <0.5ms）
- CMS `/cms/articles` 走 `ContentRepository`（**每次都查数据库**）

**方案**：

为 CMS 动态路由加缓存层：

```
缓存 key:  "cms:{plural}:{tenant}:{page}:{page_size}:{status}:{filters_hash}"
缓存 TTL:  可配置，默认 30 秒
失效策略:  create/update/delete 时清除该 content type 的所有缓存
```

**预估提升**：热数据从 38ms → <1ms，等效 RPS +10x

---

## 优化路线图

### 第一阶段：低风险高收益（1-2 天）

| # | 优化项 | 改动量 | 预估收益 |
|---|--------|--------|----------|
| 1 | SQLite PRAGMA 调优 | 5 行 | SQL -30~50% |
| 2 | JWT DecodingKey 缓存 | 3 行 | -10~30μs/请求 |
| 3 | CMS 列表内存缓存 | ~100 行 | 热数据 +10x |

### 第二阶段：中等改动（2-3 天）

| # | 优化项 | 改动量 | 预估收益 |
|---|--------|--------|----------|
| 4 | Rate Limit → DashMap | ~80 行 | RPS +100~200% |
| 5 | ContentTypeRegistry → ArcSwap | ~50 行 | 消除尾部延迟 |
| 6 | OneToMany/ManyToMany 批量化 | ~100 行 | CMS 列表 -50% |

### 第三阶段：进阶优化（可选）

| # | 优化项 | 说明 |
|---|--------|------|
| 7 | CMS 列表预 JOIN | 将 ManyToOne 改为 LEFT JOIN 合并到主查询，减少 1 次 SQL |
| 8 | 响应压缩 (gzip/brotli) | 中间件层压缩 JSON，减少传输时间 |
| 9 | 连接池调优 | 动态调整 max_connections 基于 CPU 核数 |
| 10 | release 编译优化 | LTO + codegen-units=1，预估 +20~30% |

---

## 优化后预估性能

```
当前 (20 并发, dev):
  原生列表:  1,046 RPS   P50=18.5ms
  CMS 列表:    531 RPS   P50=38.9ms
  详情:      1,043 RPS   P50=18.0ms

第一阶段后 (dev):
  原生列表:  1,500-2,000 RPS   P50=10-13ms
  CMS 列表:  3,000-5,000 RPS   P50=4-7ms (缓存命中)
  详情:      1,500-2,000 RPS   P50=10-13ms

全部优化后 (dev):
  原生列表:  2,000-3,000 RPS   P50=7-10ms
  CMS 列表:  1,500-2,500 RPS   P50=8-13ms (缓存未命中)
  详情:      3,000-5,000 RPS   P50=4-7ms

全部优化后 (release):
  原生列表:  5,000-8,000 RPS   P50=2-4ms
  CMS 列表:  3,000-5,000 RPS   P50=4-7ms
  详情:      8,000-15,000 RPS  P50=1-3ms
```

目标：release + 全部优化后，**达到 PocketBase 同一梯队（3,000-8,000 RPS）**。

---

## 横向对比参考

| 系统 | 语言 | 列表 RPS | 详情 RPS |
|------|------|----------|----------|
| PocketBase | Go | 2,000-5,000 | 3,000-6,000 |
| rust-blog (当前 dev) | Rust | 531-1,046 | 1,043 |
| rust-blog (预估 release 全优化) | Rust | 3,000-8,000 | 8,000-15,000 |
| Strapi | Node.js | 100-300 | 200-400 |
| Ghost | Node.js | 200-500 | 300-600 |
| Directus | Node.js | 80-200 | 150-300 |
