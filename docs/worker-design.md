# 后台任务队列设计文档

> 基于 SQLite 持久化的异步任务队列，通过 EventBus 与业务层解耦。

---

## 1. 设计目标

- **可靠** — 任务持久化到 SQLite，进程重启不丢失，ACID 保证
- **解耦** — service 层只 emit 事件，任务创建由 EventBus 订阅者自动完成
- **可扩展** — 新增任务类型只需加 enum variant + handler，不改框架
- **可观测** — jobs 表即仪表盘，管理页面可查看任务状态
- **可替换** — trait 抽象，后续可切换为 Redis Stream

---

## 2. 架构概览

```
Service 层                  EventBus                 JobEnqueuer
──────────                  ────────                 ───────────
post_service::create_post()
  → emit(PostCreated)  ───→ broadcast ───→ enqueue(RebuildSearchIndex)
                                           enqueue(SendWebhook)
                                           enqueue(InvalidateCache)

auth::register()
  → emit(UserRegistered)──→ broadcast ───→ enqueue(SendWelcomeEmail)

media_service::upload()
  → emit(MediaUploaded) ──→ broadcast ───→ enqueue(GenerateThumbnail)
                                           enqueue(ConvertWebP)
```

```
┌──────────────────────────────────────────────────────┐
│                   SQLite jobs 表                      │
│                                                      │
│  status: pending → running → completed               │
│                    ↓ (失败)                           │
│                  failed → pending (退避重试)           │
│                    ↓ (超过 max_attempts)              │
│                  dead (不再重试)                       │
└──────────────────────────────────────────────────────┘
        ↑ enqueue                    ↓ dequeue
   JobEnqueuer                   Worker Loop
   (EventBus sub)              (tokio::spawn N 个)
                                     │
                                     ▼
                              Job Handlers
                              ─────────────
                              SendEmail
                              ProcessImage
                              ScheduledPublish
                              WebhookNotify
                              RebuildSearchIndex
                              GenerateSitemap
```

---

## 3. 数据库 Schema

```sql
-- migrations/006_jobs.sql

CREATE TABLE IF NOT EXISTS jobs (
    id           TEXT PRIMARY KEY,            -- UUID v7
    job_type     TEXT NOT NULL,               -- 'send_email', 'process_image' ...
    payload      TEXT NOT NULL,               -- JSON 格式参数
    status       TEXT NOT NULL DEFAULT 'pending',
    attempts     INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    run_after    TEXT,                         -- ISO 8601，延迟/定时执行
    error        TEXT,                         -- 最近一次错误信息
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_run_after ON jobs(run_after) WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_jobs_type ON jobs(job_type);
```

### 状态机

```
pending ──→ running ──→ completed
  ↑            │
  │            ↓
  │         failed (attempts < max_attempts → 退避后回到 pending)
  │            │
  │            ↓
  └─── pending (run_after = now + 退避时间)
               │
               ↓ (attempts >= max_attempts)
              dead
```

---

## 4. 核心类型

### 4.1 Job 类型

```rust
// src/worker/mod.rs

/// 任务类型与参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
#[non_exhaustive]
pub enum Job {
    SendEmail {
        to: String,
        subject: String,
        body: String,
    },
    SendWelcomeEmail {
        user_id: String,
        email: String,
        username: String,
    },
    ProcessImage {
        media_id: String,
        operations: Vec<ImageOperation>,
    },
    GenerateThumbnail {
        media_id: String,
        size: u32,
    },
    ConvertWebP {
        media_id: String,
    },
    ScheduledPublish {
        post_id: String,
    },
    WebhookNotify {
        url: String,
        payload: serde_json::Value,
    },
    RebuildSearchIndex {
        post_ids: Vec<String>,
    },
    InvalidateCache {
        keys: Vec<String>,
    },
    GenerateSitemap,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ImageOperation {
    Resize { width: u32, height: u32 },
    Compress { quality: u8 },
}
```

### 4.2 JobQueue trait

```rust
#[async_trait]
pub trait JobQueue: Send + Sync {
    async fn enqueue(&self, job: NewJob) -> AppResult<()>;
    async fn enqueue_delayed(&self, job: NewJob, run_after: DateTime<Utc>) -> AppResult<()>;
    async fn dequeue(&self, limit: usize) -> AppResult<Vec<QueuedJob>>;
    async fn complete(&self, id: &str) -> AppResult<()>;
    async fn fail(&self, id: &str, error: &str) -> AppResult<()>;
    async fn dead(&self, id: &str, error: &str) -> AppResult<()>;
    async fn retry_count(&self, status: &str) -> AppResult<i64>;
}

/// 入队参数
pub struct NewJob {
    pub job: Job,
    pub max_attempts: Option<u32>,
    pub run_after: Option<DateTime<Utc>>,
}

/// 出队的任务记录
pub struct QueuedJob {
    pub id: String,
    pub job: Job,
    pub attempts: u32,
    pub max_attempts: u32,
    pub created_at: String,
}
```

### 4.3 SqliteJobQueue 实现

```rust
// src/worker/sqlite_queue.rs

pub struct SqliteJobQueue {
    pool: Pool,
}

#[async_trait]
impl JobQueue for SqliteJobQueue {
    async fn enqueue(&self, job: NewJob) -> AppResult<()> {
        // INSERT INTO jobs (id, job_type, payload, status, max_attempts, run_after, ...)
        // VALUES (?, ?, json(Job), 'pending', ?, ?, ...)
    }

    async fn dequeue(&self, limit: usize) -> AppResult<Vec<QueuedJob>> {
        // UPDATE jobs SET status = 'running', attempts = attempts + 1, updated_at = ?
        // WHERE id IN (
        //   SELECT id FROM jobs
        //   WHERE status = 'pending' AND (run_after IS NULL OR run_after <= ?)
        //   ORDER BY created_at ASC LIMIT ?
        // )
        // RETURNING id, job_type, payload, attempts, max_attempts, created_at
    }

    async fn complete(&self, id: &str) -> AppResult<()> {
        // DELETE FROM jobs WHERE id = ?
        // 或 UPDATE jobs SET status = 'completed', updated_at = ? WHERE id = ?
    }

    async fn fail(&self, id: &str, error: &str) -> AppResult<()> {
        // UPDATE jobs SET status = 'pending', error = ?, updated_at = ?,
        //   run_after = ? -- 退避时间
        // WHERE id = ? AND attempts < max_attempts
        // 如果 attempts >= max_attempts → dead
    }
}
```

---

## 5. 退避策略

指数退避 + 抖动：

```rust
fn backoff_duration(attempts: u32) -> Duration {
    let base = Duration::from_secs(10);
    let delay = base * 2u32.saturating_pow(attempts);
    let jitter = rand::random::<f64>() * 0.3 + 0.85; // 0.85 ~ 1.15
    Duration::from_secs_f64(delay.as_secs_f64() * jitter)
}

// attempt 1 失败 → ~10s 后重试
// attempt 2 失败 → ~20s 后重试
// attempt 3 失败 → ~40s 后重试
// attempt 4 → dead（默认 max_attempts = 3）
```

---

## 6. Worker Loop

```rust
// src/worker/runner.rs

pub struct WorkerRunner {
    queue: Arc<dyn JobQueue>,
    handlers: Arc<JobHandlerRegistry>,
}

impl WorkerRunner {
    pub async fn run(self) {
        let mut interval = tokio::time::interval(Duration::from_millis(500));

        loop {
            interval.tick().await;

            match self.queue.dequeue(10).await {
                Ok(jobs) => {
                    for job in jobs {
                        if let Err(e) = self.execute(&job).await {
                            tracing::error!("job {} failed: {e}", job.id);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("dequeue error: {e}");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn execute(&self, job: &QueuedJob) -> AppResult<()> {
        match self.handlers.handle(&job.job).await {
            Ok(()) => self.queue.complete(&job.id).await,
            Err(e) => {
                let err_msg = format!("{e}");
                if job.attempts >= job.max_attempts {
                    self.queue.dead(&job.id, &err_msg).await
                } else {
                    self.queue.fail(&job.id, &err_msg).await
                }
            }
        }
    }
}
```

---

## 7. Job Handler 注册

```rust
// src/worker/handler.rs

#[async_trait]
pub trait JobHandler: Send + Sync {
    async fn handle(&self, job: &Job) -> AppResult<()>;
}

pub struct JobHandlerRegistry {
    handlers: HashMap<String, Box<dyn JobHandler>>,
}

impl JobHandlerRegistry {
    pub fn new() -> Self { /* ... */ }

    pub fn register(&mut self, job_type: &str, handler: Box<dyn JobHandler>) {
        self.handlers.insert(job_type.to_string(), handler);
    }
}
```

### 内置 Handler

| Job 类型 | Handler | 依赖 |
|----------|---------|------|
| `send_email` | SmtpEmailHandler | `lettre` crate |
| `send_welcome_email` | WelcomeEmailHandler | `lettre` crate |
| `generate_thumbnail` | ThumbnailHandler | `image` crate |
| `convert_webp` | WebPHandler | `image` crate + `webp` feature |
| `scheduled_publish` | ScheduledPublishHandler | 无（SQL UPDATE） |
| `webhook_notify` | WebhookHandler | `reqwest`（已有） |
| `rebuild_search_index` | SearchIndexHandler | 无（FTS5 SQL） |
| `invalidate_cache` | CacheInvalidationHandler | CacheStore trait |
| `generate_sitemap` | SitemapHandler | 无 |

---

## 8. EventBus 集成 — JobEnqueuer

```rust
// src/worker/enqueuer.rs

pub struct JobEnqueuer {
    queue: Arc<dyn JobQueue>,
}

impl JobEnqueuer {
    pub fn spawn(eventbus: &EventBus, queue: Arc<dyn JobQueue>) {
        let mut rx = eventbus.subscribe();
        let enqueuer = Self { queue };

        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => enqueuer.on_event(event).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("job enqueuer lagged, skipped {n} events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn on_event(&self, event: Arc<Event>) {
        match event.as_ref() {
            Event::PostCreated { id, .. } => {
                let _ = self.queue.enqueue(NewJob::from(Job::RebuildSearchIndex {
                    post_ids: vec![id.clone()],
                })).await;
            }
            Event::UserRegistered { user_id, email, username } => {
                let _ = self.queue.enqueue(NewJob::from(Job::SendWelcomeEmail {
                    user_id: user_id.clone(),
                    email: email.clone(),
                    username: username.clone(),
                })).await;
            }
            Event::MediaUploaded { id, .. } => {
                let _ = self.queue.enqueue(NewJob::from(Job::GenerateThumbnail {
                    media_id: id.clone(),
                    size: 300,
                })).await;
                let _ = self.queue.enqueue(NewJob::from(Job::ConvertWebP {
                    media_id: id.clone(),
                })).await;
            }
            _ => {}
        }
    }
}
```

---

## 9. 服务端启动集成

```rust
// src/server/mod.rs — build_app 中新增

// 创建 JobQueue
let job_queue = Arc::new(SqliteJobQueue::new(pool.clone()));

// 注册 handlers
let mut registry = JobHandlerRegistry::new();
registry.register("send_welcome_email", Box::new(WelcomeEmailHandler::new(config.clone())));
registry.register("generate_thumbnail", Box::new(ThumbnailHandler::new(config.clone())));
registry.register("rebuild_search_index", Box::new(SearchIndexHandler::new(pool.clone())));
// ...

// 启动 JobEnqueuer (EventBus → Queue)
JobEnqueuer::spawn(&eventbus, job_queue.clone());

// 启动 Worker (Queue → Execute)
let runner = WorkerRunner::new(job_queue, Arc::new(registry));
tokio::spawn(runner.run());
```

---

## 10. 配置

`.env` 新增：

```env
# Worker 配置
WORKER_ENABLED=true
WORKER_CONCURRENCY=2            # 并发 worker 数
WORKER_POLL_INTERVAL_MS=500     # 轮询间隔
WORKER_DEFAULT_MAX_ATTEMPTS=3   # 默认最大重试次数
```

`AppConfig` 新增：

```rust
pub worker_enabled: bool,
pub worker_concurrency: usize,
pub worker_poll_interval_ms: u64,
pub worker_default_max_attempts: u32,
```

---

## 11. 管理 API

复用现有 plugin 管理的模式：

| 端点 | 说明 |
|------|------|
| `GET /admin/jobs` | 任务列表（分页，按 status 筛选） |
| `GET /admin/jobs/:id` | 任务详情（含 payload 和 error） |
| `POST /admin/jobs/:id/retry` | 手动重试 dead 任务 |
| `DELETE /admin/jobs/:id` | 删除任务记录 |
| `POST /admin/jobs/cleanup` | 清理 completed/dead 任务 |

---

## 12. 文件结构

```
src/worker/
├── mod.rs              # Job enum, JobQueue trait, 公共类型
├── sqlite_queue.rs     # SqliteJobQueue 实现
├── runner.rs           # WorkerRunner — 后台轮询执行
├── enqueuer.rs         # JobEnqueuer — EventBus → Queue 桥接
├── handler.rs          # JobHandler trait + Registry
└── handlers/
    ├── mod.rs
    ├── email.rs        # SendEmail, SendWelcomeEmail
    ├── image.rs        # GenerateThumbnail, ConvertWebP
    ├── publish.rs      # ScheduledPublish
    ├── webhook.rs      # WebhookNotify
    ├── search.rs       # RebuildSearchIndex
    └── cache.rs        # InvalidateCache
```

---

## 13. 后续扩展

### 切换 Redis Stream

```rust
pub struct RedisJobQueue {
    client: redis::Client,
    stream_key: String,
}

#[async_trait]
impl JobQueue for RedisJobQueue { /* ... */ }
```

通过 feature flag 切换：

```toml
[features]
queue-sqlite = []
queue-redis  = ["redis"]
```

### 优先级队列

jobs 表新增 `priority INTEGER DEFAULT 5`，dequeue 时 `ORDER BY priority ASC, created_at ASC`。

### 定时任务 (Cron)

新增 `scheduled_jobs` 表存储 cron 表达式，独立 scheduler tick 按配置生成 job：

```sql
CREATE TABLE scheduled_jobs (
    id         TEXT PRIMARY KEY,
    job_type   TEXT NOT NULL,
    payload    TEXT NOT NULL,
    cron_expr  TEXT NOT NULL,       -- '0 */5 * * *' (每 5 分钟)
    last_run   TEXT,
    next_run   TEXT NOT NULL,
    enabled    BOOLEAN DEFAULT true
);
```

### 并发控制

按 `job_type` 设置并发上限，防止某类任务占满 worker：

```rust
pub struct WorkerRunner {
    concurrency_limits: HashMap<String, Semaphore>,
}
```
