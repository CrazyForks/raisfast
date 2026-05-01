# AOP 框架设计 — 横切面基础设施

> 版本：v1.0 | 最后更新：2026-05-01
> 状态：设计阶段

## 1. 为什么需要 AOP

当前系统的横切面逻辑散落在四层中，没有统一抽象：

```
一个 POST /cms/articles 实际经过的逻辑：

Middleware 层
  ├─ CORS 检查
  ├─ Rate Limit
  ├─ JWT 解析

Handler 层
  ├─ access control 检查         ← handler 内 if 判断
  ├─ 参数校验                   ← validation.rs
  ├─ 缓存检查（read 时）        ← handler 内 if cache
  ├─ 缓存清除（write 后）       ← handler 内 if cache
  ├─ 字段过滤（response）       ← filter_fields()
  │
Repository 层
  ├─ auto_fill 注入             ← save_ctx
  ├─ timestamps 注入            ← if ct.timestamps
  ├─ 软删除处理                 ← if ct.soft_delete
  ├─ 版本快照                   ← if ct.versioning
  ├─ INSERT/UPDATE/DELETE SQL
  │
EventBus 层
  ├─ 审计日志                   ← subscriber
  ├─ Webhook 通知               ← subscriber
  └─ 搜索索引                   ← 未实现
```

**问题：** 内置表（posts/pages/comments）的横切面逻辑全靠手写，无法复用 Content Type 的任何机制。每新增一个横切面需求，要在多个地方重复实现。

**目标：** 每个 Aspect（横切面）定义一次，所有数据路径自动生效。

## 2. 概念模型

```
┌──────────────────────────────────────────────────────────┐
│                        Aspect                            │
│  "我叫什么"            (name)                             │
│  "我关心什么事件"     (pointcut)                          │
│  "我排在第几位"         (priority)                        │
│  "我做什么"                      (advise)                 │
├──────────────────────────────────────────────────────────┤
│  Layer       Aspect 关心的系统层级                         │
│  Pointcut    Aspect 关心的拦截点（精确匹配或模式匹配）      │
│  Advice      Aspect 在拦截点上的具体行为                    │
│  Context     拦截点的上下文数据（可读/可写）                 │
│  Engine      调度器，管理 Aspect 注册和执行                  │
└──────────────────────────────────────────────────────────┘
```

### 2.1 Layer（系统层级）

```
┌─ HTTP Layer ──────────────────────────────────────────┐
│  HTTP 请求/响应拦截                                     │
│  适用：CORS、Rate Limit、请求日志、请求追踪             │
│  注意：现有 middleware 已经在做 HTTP 层 AOP，            │
│        未来可统一注册到 AspectEngine，但不急于迁移       │
└───────────────────────────────────────────────────────┘

┌─ Access Layer ────────────────────────────────────────┐
│  路由级 + 数据级权限检查                                │
│  适用：角色校验、RBAC、API Rule 数据过滤、字段级 ACL     │
└───────────────────────────────────────────────────────┘

┌─ Data Layer ──────────────────────────────────────────┐
│  数据 CRUD 操作前后拦截                                 │
│  适用：字段注入、校验、版本快照、缓存、搜索索引          │
│  这是 AOP 的核心层                                      │
└───────────────────────────────────────────────────────┘

┌─ Event Layer ─────────────────────────────────────────┐
│  事件发布/消费拦截（异步，事务外）                       │
│  适用：审计日志、Webhook、通知、搜索索引更新              │
└───────────────────────────────────────────────────────┘
```

### 2.2 四层之间的关系

```
HTTP 请求进入
  │
  ▼
┌─ HTTP Layer ──────── on_http_before ──────────────────┐
│  请求到达 handler 之前                                  │
└───────────────────────────────────────────────────────┘
  │
  ▼
┌─ Access Layer ────── on_access_check ─────────────────┐
│  路由级权限：这个用户能访问这个路由吗？                  │
│  数据级过滤：这个用户能看到哪些数据？                    │
└───────────────────────────────────────────────────────┘
  │
  ▼
┌─ Data Layer ──────── 同步，在事务内 ──────────────────┐
│                                                        │
│  on_data_before_create ──→ [INSERT] ──→ on_data_after_create  │
│  on_data_before_read   ──→ [SELECT] ──→ on_data_after_read    │
│  on_data_before_update ──→ [UPDATE] ──→ on_data_after_update  │
│  on_data_before_delete ──→ [DELETE] ──→ on_data_after_delete  │
│                                                        │
└───────────────────────────────────────────────────────┘
  │
  ▼
┌─ Event Layer ─────── 异步，事务外 ───────────────────┐
│                                                        │
│  on_event_publish ──→ EventBus ──→ on_event_consume   │
│                                                        │
│  审计日志、Webhook、通知、搜索索引                      │
└───────────────────────────────────────────────────────┘
  │
  ▼
┌─ HTTP Layer ──────── on_http_after ───────────────────┐
│  响应返回之前                                          │
└───────────────────────────────────────────────────────┘
```

### 2.3 同步 vs 异步边界

| 层 | 执行方式 | 事务 | 失败影响 |
|---|---|---|---|
| HTTP | 同步 | 无 | 阻断请求 |
| Access | 同步 | 无 | 阻断请求 |
| Data Before | 同步 | **在事务内** | 回滚整个操作 |
| Data After | 同步 | **在事务内** | 回滚整个操作 |
| Event | 异步 | 事务外 | 仅记日志，不影响主操作 |

## 3. Join Points 完整目录

### 3.1 HTTP Layer

| Join Point | 时机 | Context 可变性 | 用途 |
|---|---|---|---|
| `on_http_before` | 请求到达 handler 前 | 可修改 headers、可注入扩展数据 | 请求追踪、日志、限流 |
| `on_http_after` | 响应返回前 | 可修改 response body | 响应日志、响应头注入 |

### 3.2 Access Layer

| Join Point | 时机 | Context 可变性 | 用途 |
|---|---|---|---|
| `on_access_check` | handler 内权限检查时 | 只读，返回 Allow/Deny | 角色校验、RBAC、API Rule |
| `on_access_filter` | 查询前追加 WHERE 条件 | 可修改 query conditions | 数据级过滤、多租户隔离 |

### 3.3 Data Layer

| Join Point | 时机 | Context 可变性 | 用途 |
|---|---|---|---|
| `on_data_before_create` | INSERT 前 | **record 可修改** | 注入 author_id、timestamps、slug |
| `on_data_after_create` | INSERT 后 | 只读 | 搜索索引更新（事务内） |
| `on_data_before_read` | SELECT 前 | 可修改 query（排序、过滤、字段） | 缓存命中短路、追加默认条件 |
| `on_data_after_read` | SELECT 后 | 可修改 record | 字段解密、关联数据填充 |
| `on_data_before_update` | UPDATE 前 | **new_record 可修改**，old_record 只读 | timestamps、版本快照、slug 重新生成 |
| `on_data_after_update` | UPDATE 后 | 只读 | 缓存清除、搜索索引更新 |
| `on_data_before_delete` | DELETE 前 | record 只读，可改为软删除 | 软删除拦截、删除前校验 |
| `on_data_after_delete` | DELETE 后 | 只读 | 缓存清除、搜索索引删除 |

### 3.4 Event Layer

| Join Point | 时机 | Context 可变性 | 用途 |
|---|---|---|---|
| `on_event_before_publish` | 事件发布前 | 可修改 event payload | 事件过滤、事件增强 |
| `on_event_after_publish` | 事件发布后 | 只读 | 发布确认 |
| `on_event_before_consume` | 消费者处理前 | 可修改 event payload | 消费过滤 |
| `on_event_after_consume` | 消费者处理后 | 只读 | 消费确认、死信处理 |

## 4. Pointcut 匹配系统

### 4.1 数据模型

```rust
/// 拦截点标识
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct JoinPointId {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Http,
    Access,
    Data,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Operation {
    Create,
    Read,
    Update,
    Delete,
    Publish,
    Consume,
    Check,    // access check
    Filter,   // access filter
    Request,  // http request
    Response, // http response
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    Before,
    After,
}

/// 匹配目标
#[derive(Debug, Clone)]
pub enum TargetMatcher {
    /// 匹配所有目标
    All,
    /// 精确匹配表名列表
    Tables(Vec<String>),
    /// glob 模式匹配表名（如 "blog_*"）
    TablePattern(String),
    /// 路由匹配（如 "/admin/*"）
    Routes(Vec<String>),
    /// 事件类型匹配
    Events(Vec<String>),
    /// 自定义谓词（最高灵活性）
    Custom(fn(&TargetInfo) -> bool),
}

/// 匹配目标的信息
#[derive(Debug, Clone)]
pub struct TargetInfo {
    pub table: Option<String>,
    pub route: Option<String>,
    pub event_type: Option<String>,
    pub is_content_type: bool,
    pub is_builtin: bool,
}

/// Pointcut 定义
#[derive(Debug, Clone)]
pub struct Pointcut {
    pub layer: Layer,
    pub operation: Operation,
    pub when: When,
    pub target: TargetMatcher,
}
```

### 4.2 匹配规则

一个 Aspect 可以声明多个 Pointcut，只要任何一个匹配就触发：

```rust
impl Aspect for OwnableAspect {
    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::All,
            },
        ]
    }
}
```

### 4.3 注册时优化

AspectEngine 在注册时预计算每个 JoinPointId 对应的 Aspect 列表（排序后），运行时 O(1) 查找：

```rust
pub struct AspectEngine {
    /// JoinPointId → 排序后的 Aspect 列表
    dispatch_table: HashMap<JoinPointId, Vec<Arc<dyn Aspect>>>,
    /// 所有已注册 Aspect（用于管理）
    aspects: Vec<AspectEntry>,
}
```

## 5. Context 类型体系

### 5.1 基础上下文

所有 Context 共享的基础字段：

```rust
/// 所有 Context 的基础字段
pub struct BaseContext {
    pub user_id: Option<String>,
    pub user_role: Option<String>,
    pub tenant_id: String,
    pub now: String,           // ISO 8601
    pub request_id: String,    // 请求追踪 ID
    /// Aspect 间通信用的扩展存储
    pub extensions: Extensions,
}

/// 类型安全的扩展存储
/// Aspect 可以写入数据，后续 Aspect 可以读取
pub struct Extensions {
    map: HashMap<std::any::TypeId, Box<dyn std::any::Any + Send + Sync>>,
}

impl Extensions {
    pub fn insert<T: Send + Sync + 'static>(&mut self, val: T) { ... }
    pub fn get<T: 'static>(&self) -> Option<&T> { ... }
    pub fn remove<T: 'static>(&mut self) -> Option<T> { ... }
}
```

### 5.2 各层 Context

```rust
// ─── HTTP Layer ───

pub struct HttpBeforeContext {
    pub base: BaseContext,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub query_params: HashMap<String, String>,
}

pub struct HttpAfterContext {
    pub base: BaseContext,
    pub status_code: u16,
    pub response_body: Option<serde_json::Value>,
}

// ─── Access Layer ───

pub struct AccessCheckContext {
    pub base: BaseContext,
    pub route: String,
    pub method: String,
    pub table: Option<String>,
    pub action: String,  // "create" | "read" | "update" | "delete"
}

pub struct AccessFilterContext {
    pub base: BaseContext,
    pub table: String,
    /// Aspect 追加的 WHERE 条件（AND 组合）
    pub conditions: Vec<String>,
    /// Aspect 追加的 WHERE 参数
    pub params: Vec<String>,
}

// ─── Data Layer ───

pub struct DataBeforeCreateContext {
    pub base: BaseContext,
    pub table: String,
    /// 即将写入的记录，Aspect 可修改
    pub record: Record,
    /// 当前表的 schema 元数据（如果有）
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterCreateContext {
    pub base: BaseContext,
    pub table: String,
    /// 已写入的记录（只读）
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeReadContext {
    pub base: BaseContext,
    pub table: String,
    /// 查询条件，Aspect 可修改
    pub query: ReadQuery,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterReadContext {
    pub base: BaseContext,
    pub table: String,
    /// 查询结果，Aspect 可修改（如解密字段）
    pub records: Vec<Record>,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeUpdateContext {
    pub base: BaseContext,
    pub table: String,
    /// 更新前的旧数据（只读）
    pub old_record: Record,
    /// 即将写入的新数据，Aspect 可修改
    pub new_record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterUpdateContext {
    pub base: BaseContext,
    pub table: String,
    pub old_record: Record,
    pub new_record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataBeforeDeleteContext {
    pub base: BaseContext,
    pub table: String,
    /// 即将删除的记录
    pub record: Record,
    /// 软删除标记：Aspect 可设为 true 来阻止真实 DELETE
    pub soft_delete: bool,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct DataAfterDeleteContext {
    pub base: BaseContext,
    pub table: String,
    pub record: Record,
    pub schema: Option<Arc<ContentTypeSchema>>,
}

pub struct ReadQuery {
    pub filters: Vec<(String, String)>,  // (column, value)
    pub order_by: Option<String>,
    pub page: u64,
    pub page_size: u64,
    pub fields: Option<Vec<String>>,
}

// ─── Event Layer ───

pub struct EventContext {
    pub base: BaseContext,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub table: Option<String>,
}
```

### 5.3 Aspect 间通信

通过 `extensions` 实现：

```
执行顺序：ownable (priority -500) → validation (priority 500)

// ownable 写入
ctx.base.extensions.insert(OwnableData { author_id: user_id.clone() });

// validation 读取
if let Some(ownable) = ctx.base.extensions.get::<OwnableData>() {
    // 知道 author_id 已被注入
}
```

## 6. Aspect trait 设计

### 6.1 核心 trait

```rust
use async_trait::async_trait;

/// Aspect 执行结果
pub enum Advice {
    /// 继续执行后续 Aspect
    Continue,
    /// 跳过当前 Aspect 的后续处理（不阻断）
    Skip,
    /// 短路返回（用于缓存命中、权限拒绝等场景）
    Return(serde_json::Value),
}

/// Aspect 错误
pub type AspectError = Box<dyn std::error::Error + Send + Sync>;
pub type AspectResult = Result<Advice, AspectError>;

/// Aspect 核心 trait
///
/// 所有方法有默认实现（返回 Continue），Aspect 只需覆盖关心的方法。
/// Engine 通过 pointcuts() 判断是否调用，避免无谓的虚函数调用。
#[async_trait]
pub trait Aspect: Send + Sync + 'static {
    /// Aspect 名称（唯一标识）
    fn name(&self) -> &str;

    /// 优先级（越小越先执行）
    fn priority(&self) -> i32 {
        0
    }

    /// 声明关心的拦截点
    fn pointcuts(&self) -> Vec<Pointcut>;

    // ─── HTTP Layer ───

    async fn on_http_before(&self, _ctx: &mut HttpBeforeContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_http_after(&self, _ctx: &mut HttpAfterContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── Access Layer ───

    async fn on_access_check(&self, _ctx: &mut AccessCheckContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_access_filter(&self, _ctx: &mut AccessFilterContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── Data Layer ───

    async fn on_data_before_create(&self, _ctx: &mut DataBeforeCreateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_create(&self, _ctx: &mut DataAfterCreateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_read(&self, _ctx: &mut DataBeforeReadContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_read(&self, _ctx: &mut DataAfterReadContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(&self, _ctx: &mut DataBeforeUpdateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_update(&self, _ctx: &mut DataAfterUpdateContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_before_delete(&self, _ctx: &mut DataBeforeDeleteContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_data_after_delete(&self, _ctx: &mut DataAfterDeleteContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    // ─── Event Layer ───

    async fn on_event_before_publish(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_after_publish(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_before_consume(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }

    async fn on_event_after_consume(&self, _ctx: &mut EventContext) -> AspectResult {
        Ok(Advice::Continue)
    }
}
```

### 6.2 为什么用 "胖 trait" 而不是分层 trait

**替代方案：** 每层一个 trait（`HttpAspect` / `DataAspect` / …）

**选择胖 trait 的理由：**

1. **统一注册：** `Vec<Arc<dyn Aspect>>` 一个列表管理所有 Aspect
2. **跨层 Aspect：** 一个 Aspect 可以同时拦截 HTTP + Data 层（如安全审计）
3. **简洁：** 不需要多 trait object 转换
4. **性能可接受：** Engine 在注册时通过 `pointcuts()` 预过滤，运行时只调用匹配的 hook

**代价：** 每个 Aspect 有 16 个默认方法。但 Aspect 只需覆盖关心的方法，实际没有负担。

## 7. AspectEngine 设计

### 7.1 数据结构

```rust
pub struct AspectEngine {
    /// JoinPointId + Target → 排序后的 Aspect 列表
    /// 注册时预计算，运行时 O(1) 查找
    dispatch_table: DashMap<JoinPointId, Vec<Arc<dyn Aspect>>>,

    /// 所有 Aspect 的注册信息
    registry: RwLock<Vec<AspectEntry>>,
}

pub struct AspectEntry {
    pub aspect: Arc<dyn Aspect>,
    pub pointcuts: Vec<Pointcut>,
    pub enabled: bool,
}
```

### 7.2 注册流程

```rust
impl AspectEngine {
    /// 注册一个 Aspect
    ///
    /// 1. 读取 pointcuts() 确定关心的 JoinPoint
    /// 2. 按 priority 插入到 dispatch_table 对应列表
    /// 3. 保存到 registry
    pub fn register(&self, aspect: impl Aspect) {
        let arc = Arc::new(aspect);
        let pointcuts = arc.pointcuts();
        let priority = arc.priority();

        for pc in &pointcuts {
            let jp_id = JoinPointId {
                layer: pc.layer,
                operation: pc.operation,
                when: pc.when,
            };
            let mut list = self.dispatch_table.entry(jp_id).or_default();
            list.push(arc.clone());
            list.sort_by_key(|a| a.priority());
        }

        self.registry.write().unwrap().push(AspectEntry {
            aspect: arc,
            pointcuts,
            enabled: true,
        });
    }
}
```

### 7.3 调度方法

```rust
impl AspectEngine {
    /// 调度数据层 before create
    ///
    /// 遍历匹配的 Aspect，按 priority 顺序执行。
    /// 任何 Aspect 返回 Err 或 Advice::Return 则中断。
    pub async fn dispatch_data_before_create(
        &self,
        table: &str,
        ctx: &mut DataBeforeCreateContext,
    ) -> Result<Option<serde_json::Value>, AspectError> {
        let jp_id = JoinPointId {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
        };

        let aspects = self.dispatch_table.get(&jp_id);
        let Some(aspects) = aspects else { return Ok(None) };

        for aspect in aspects.iter() {
            if !matches_target(&aspect.pointcuts(), table) {
                continue;
            }
            match aspect.on_data_before_create(ctx).await {
                Ok(Advice::Continue) => continue,
                Ok(Advice::Skip) => continue,
                Ok(Advice::Return(val)) => return Ok(Some(val)),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// 其他 dispatch 方法类似...
    /// dispatch_data_after_create
    /// dispatch_data_before_read
    /// dispatch_data_after_read
    /// dispatch_data_before_update
    /// dispatch_data_after_update
    /// dispatch_data_before_delete
    /// dispatch_data_after_delete
    /// dispatch_access_check
    /// dispatch_access_filter
    /// dispatch_http_before
    /// dispatch_http_after
    /// dispatch_event_*
}
```

### 7.4 优先级约定

```
优先级范围        用途                           示例
─────────────────────────────────────────────────────────
 -9999 ~ -1000   核心基础设施                   认证、租户隔离
  -999 ~ -500    数据注入                       ownable, timestampable
  -499 ~    0    默认
     1 ~  499    业务逻辑                       validation, slug 生成
   500 ~  999    副作用（事务内）                版本快照、搜索索引
  1000 ~  9999   事后处理（事务内）              缓存清除
 10000+          非关键（可降级）                统计、监控
```

## 8. 执行模型

### 8.1 数据写入操作的完整流程

```
Handler 收到请求
  │
  ├─ 1. 构建 BaseContext（user_id, tenant_id, now, request_id）
  │
  ├─ 2. [同步] dispatch_access_check(route, user, action)
  │     └─ 返回 Deny → 403 Forbidden
  │
  ├─ 3. [事务开始]
  │     │
  │     ├─ 4. dispatch_data_before_create(table, ctx)
  │     │     ├─ ownable (-500):     ctx.record["author_id"] = ctx.user_id
  │     │     ├─ timestampable (-400): ctx.record["created_at"] = ctx.now
  │     │     └─ validation (500):    校验字段约束
  │     │
  │     ├─ 5. repository.insert(table, ctx.record)
  │     │
  │     ├─ 6. dispatch_data_after_create(table, ctx)
  │     │     └─ searchable (600): INSERT INTO search_index(...)
  │     │
  │     └─ 7. [事务提交]
  │            │
  │            ├─ 事务失败 → 全部回滚，返回错误
  │            │
  │            └─ 事务成功 ↓
  │
  ├─ 8. [异步] dispatch_event_publish("record.created", payload)
  │     ├─ audit_aspect:     INSERT INTO audit_log(...)
  │     ├─ webhook_aspect:   HTTP POST to webhook_url
  │     └─ notification_aspect: 发送通知
  │
  └─ 9. 返回响应
```

### 8.2 读取操作（带缓存）

```
Handler 收到 GET 请求
  │
  ├─ 1. dispatch_data_before_read(table, ctx)
  │     └─ cacheable (1000): 检查缓存
  │        └─ 缓存命中 → Advice::Return(cached_data)
  │
  ├─ 2. 如果 Advice::Return(short_circuit_value)
  │     └─ 跳过实际查询，直接使用返回值
  │
  ├─ 3. 否则：repository.find(table, ctx.query)
  │
  ├─ 4. dispatch_data_after_read(table, ctx)
  │     ├─ cacheable (1000): 写入缓存
  │     └─ decrypt_aspect: 解密加密字段
  │
  ├─ 5. dispatch_access_filter(table, ctx)
  │     └─ 字段过滤：移除 private 字段
  │
  └─ 6. 返回响应
```

### 8.3 错误处理策略

```rust
/// Before hook 错误 → 中断操作，回滚事务
///
/// 这是有意为之的拦截（如校验失败、权限不足），
/// 错误信息向上传播给 handler，转为 HTTP 错误响应。
dispatch_data_before_create(...) → Err(e)
  → 事务回滚
  → handler 收到 AppError
  → 返回 400/403/500

/// After hook 错误 → 中断操作，回滚事务
///
/// after hook 在事务内，失败意味着数据没写入。
/// 对于非关键副作用，after hook 应该自己 catch 错误并降级。
dispatch_data_after_create(...) → Err(e)
  → 事务回滚
  → handler 收到 AppError

/// Event hook 错误 → 仅记日志，不影响主操作
///
/// 事件层在事务外，异步执行。
/// 失败的事件会被重试或进入死信队列。
dispatch_event_consume(...) → Err(e)
  → 记录 warn! 日志
  → 不影响已返回的 HTTP 响应
```

### 8.4 短路（Short-circuit）

```rust
/// Advice::Return(Value) 用于：
///
/// 1. 缓存命中 → 返回缓存数据，跳过数据库查询
/// 2. 权限拒绝 → 返回错误信息，跳过后续处理
/// 3. 限流 → 返回 429 响应
///
/// Engine 收到 Advice::Return 后：
/// - 不再执行后续 Aspect
/// - 不执行实际操作（INSERT/SELECT 等）
/// - 将 Value 返回给调用方
```

## 9. 内置 Aspects

### 9.1 ownable（默认内置）

```rust
pub struct OwnableAspect;

#[async_trait]
impl Aspect for OwnableAspect {
    fn name(&self) -> &str { "ownable" }
    fn priority(&self) -> i32 { -500 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Create,
            when: When::Before,
            target: TargetMatcher::All,
        }]
    }

    async fn on_data_before_create(
        &self,
        ctx: &mut DataBeforeCreateContext,
    ) -> AspectResult {
        if let Some(user_id) = &ctx.base.user_id {
            ctx.record.insert(
                "author_id".into(),
                serde_json::Value::String(user_id.clone()),
            );
        }
        Ok(Advice::Continue)
    }
}
```

**行为：**
- 自动注入 `author_id`（不需要字段定义里有 `auto_fill`）
- 适用于所有表（内置表 + Content Type）
- migration 层确保表有 `author_id TEXT` 列

### 9.2 timestampable（默认内置）

```rust
pub struct TimestampableAspect;

#[async_trait]
impl Aspect for TimestampableAspect {
    fn name(&self) -> &str { "timestampable" }
    fn priority(&self) -> i32 { -400 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::Before,
                target: TargetMatcher::All,
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::Before,
                target: TargetMatcher::All,
            },
        ]
    }

    async fn on_data_before_create(
        &self,
        ctx: &mut DataBeforeCreateContext,
    ) -> AspectResult {
        ctx.record.insert("created_at".into(), json!(ctx.base.now));
        ctx.record.insert("updated_at".into(), json!(ctx.base.now));
        Ok(Advice::Continue)
    }

    async fn on_data_before_update(
        &self,
        ctx: &mut DataBeforeUpdateContext,
    ) -> AspectResult {
        ctx.new_record.insert("updated_at".into(), json!(ctx.base.now));
        Ok(Advice::Continue)
    }
}
```

**行为：**
- 创建时自动填 `created_at` + `updated_at`
- 更新时自动更新 `updated_at`
- 替代现有 `timestamps: true` 标志和 `auto_fill: CurrentTimestamp`

### 9.3 versionable（可选声明）

```rust
pub struct VersionableAspect;

#[async_trait]
impl Aspect for VersionableAspect {
    fn name(&self) -> &str { "versionable" }
    fn priority(&self) -> i32 { 600 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Update,
            when: When::Before,
            // 仅对声明了 versionable 的表生效
            target: TargetMatcher::Custom(|info| {
                // 从 TargetInfo 判断表是否声明了 versionable
                versionable_tables.contains(info.table.unwrap_or(""))
            }),
        }]
    }

    async fn on_data_before_update(
        &self,
        ctx: &mut DataBeforeUpdateContext,
    ) -> AspectResult {
        // 将旧数据快照写入 content_revisions
        sqlx::query("INSERT INTO content_revisions (table_name, record_id, snapshot, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ctx.table)
            .bind(ctx.old_record.get("id").unwrap_or(&json!("")))
            .bind(serde_json::to_string(&ctx.old_record)?)
            .bind(&ctx.base.now)
            .execute(&ctx.pool)  // 需要 pool 访问
            .await?;
        Ok(Advice::Continue)
    }
}
```

### 9.4 soft_deletable（可选声明）

```rust
pub struct SoftDeletableAspect;

#[async_trait]
impl Aspect for SoftDeletableAspect {
    fn name(&self) -> &str { "soft_deletable" }
    fn priority(&self) -> i32 { -800 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Data,
            operation: Operation::Delete,
            when: When::Before,
            target: TargetMatcher::Custom(soft_delete_tables_matcher),
        }]
    }

    async fn on_data_before_delete(
        &self,
        ctx: &mut DataBeforeDeleteContext,
    ) -> AspectResult {
        // 将 DELETE 转为 UPDATE deleted_at
        ctx.soft_delete = true;
        Ok(Advice::Continue)
    }
}
```

### 9.5 cacheable（可选声明）

```rust
pub struct CacheableAspect {
    cache: Arc<DashMap<String, (serde_json::Value, Instant)>>,
    ttl: Duration,
}

#[async_trait]
impl Aspect for CacheableAspect {
    fn name(&self) -> &str { "cacheable" }
    fn priority(&self) -> i32 { 1000 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Read,
                when: When::Before,
                target: TargetMatcher::Custom(cacheable_tables),
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Read,
                when: When::After,
                target: TargetMatcher::Custom(cacheable_tables),
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Create,
                when: When::After,
                target: TargetMatcher::Custom(cacheable_tables),
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Update,
                when: When::After,
                target: TargetMatcher::Custom(cacheable_tables),
            },
            Pointcut {
                layer: Layer::Data,
                operation: Operation::Delete,
                when: When::After,
                target: TargetMatcher::Custom(cacheable_tables),
            },
        ]
    }

    /// 读取前：检查缓存
    async fn on_data_before_read(
        &self,
        ctx: &mut DataBeforeReadContext,
    ) -> AspectResult {
        let key = self.cache_key(&ctx.table, &ctx.query);
        if let Some((data, ts)) = self.cache.get(&key) {
            if ts.elapsed() < self.ttl {
                return Ok(Advice::Return(data.clone()));
            }
        }
        Ok(Advice::Continue)
    }

    /// 读取后：写入缓存
    async fn on_data_after_read(
        &self,
        ctx: &mut DataAfterReadContext,
    ) -> AspectResult {
        let key = self.cache_key(&ctx.table, &ctx.query);
        self.cache.insert(key, (json!(ctx.records), Instant::now()));
        Ok(Advice::Continue)
    }

    /// 写入后：清除缓存
    async fn on_data_after_create(&self, ...) -> AspectResult {
        self.invalidate_table(&ctx.table);
        Ok(Advice::Continue)
    }

    // after_update, after_delete 同样清除缓存
}
```

### 9.6 audit（默认内置，Event 层）

```rust
pub struct AuditAspect {
    pool: Pool,
}

#[async_trait]
impl Aspect for AuditAspect {
    fn name(&self) -> &str { "audit" }
    fn priority(&self) -> i32 { 0 }

    fn pointcuts(&self) -> Vec<Pointcut> {
        vec![Pointcut {
            layer: Layer::Event,
            operation: Operation::Consume,
            when: When::Before,
            target: TargetMatcher::Events(vec![
                "record.created".into(),
                "record.updated".into(),
                "record.deleted".into(),
            ]),
        }]
    }

    async fn on_event_before_consume(
        &self,
        ctx: &mut EventContext,
    ) -> AspectResult {
        sqlx::query("INSERT INTO audit_log (action, table_name, record_id, user_id, tenant_id, detail, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(&ctx.event_type)
            .bind(ctx.payload.get("table").unwrap_or(&json!("")))
            .bind(ctx.payload.get("id").unwrap_or(&json!("")))
            .bind(&ctx.base.user_id)
            .bind(&ctx.base.tenant_id)
            .bind(serde_json::to_string(&ctx.payload)?)
            .bind(&ctx.base.now)
            .execute(&self.pool)
            .await?;
        Ok(Advice::Continue)
    }
}
```

## 10. 与现有系统集成

### 10.1 AspectEngine 在 AppState 中的位置

```rust
pub struct AppState {
    pub pool: Pool,
    pub config: Arc<AppConfig>,
    pub aspect_engine: Arc<AspectEngine>,  // ← 新增，核心基础设施
    pub plugins: Arc<PluginManager>,
    pub content_type_registry: Arc<ContentTypeRegistry>,
    pub eventbus: EventBus,
    // ... 其他字段
}
```

### 10.2 启动流程

```rust
pub async fn build_app_state(config: &AppConfig) -> anyhow::Result<AppState> {
    // 1. 创建 AspectEngine（最先初始化）
    let aspect_engine = Arc::new(AspectEngine::new());

    // 2. 注册默认内置 Aspects（所有表自动生效）
    aspect_engine.register(OwnableAspect);
    aspect_engine.register(TimestampableAspect);
    aspect_engine.register(AuditAspect { pool: pool.clone() });
    aspect_engine.register(CacheableAspect { ... });

    // 3. 为内置表注册可选 Aspects
    aspect_engine.register_for_tables(
        &["posts", "pages", "comments"],
        vec![Arc::new(VersionableAspect)],
    );

    // 4. 为 Content Type 按 implements 注册 Aspects
    for schema in ct_registry.all() {
        for protocol_name in &schema.implements {
            match protocol_name.as_str() {
                "versionable" => aspect_engine.register_for_table(
                    &schema.table,
                    Arc::new(VersionableAspect),
                ),
                "cacheable" => aspect_engine.register_for_table(
                    &schema.table,
                    Arc::new(CacheableAspect { ... }),
                ),
                _ => {}
            }
        }
    }

    // ...
}
```

### 10.3 ContentRepository 集成

**现状：**
```rust
// repository.rs create()
if ct.timestamps { obj.insert("created_at", now.clone()); ... }
if ct.draft_publish { obj.insert("status", "draft"); }
save_ctx.inject_auto_fill(ct, obj);
// ... INSERT ...
if ct.versioning { save_revision(...); }
```

**改为：**
```rust
// repository.rs create()
let mut ctx = DataBeforeCreateContext {
    base: BaseContext::from_save_ctx(save_ctx),
    table: ct.table.clone(),
    record: obj,
    schema: Some(ct.clone()),
};

// 所有横切面逻辑由 AspectEngine 统一处理
if let Some(short_circuit) = state.aspect_engine
    .dispatch_data_before_create(&ct.table, &mut ctx)
    .await?
{
    return Ok(short_circuit);
}

let record = ctx.record;

// 纯粹的数据操作，不含任何横切面逻辑
let result = self.do_insert(&ct.table, &record, tenant_id).await?;

let mut after_ctx = DataAfterCreateContext { ... };
state.aspect_engine
    .dispatch_data_after_create(&ct.table, &mut after_ctx)
    .await?;

Ok(result)
```

### 10.4 内置表 Service 层集成

**现状（services/post.rs）：**
```rust
pub async fn create(&self, post: CreatePostRequest, user_id: &str) -> Result<Post> {
    let id = new_id();
    let now = now_iso8601();
    sqlx::query("INSERT INTO posts (id, title, author_id, created_at, updated_at, ...) VALUES (?, ?, ?, ?, ?, ...)")
        .bind(&id)
        .bind(&post.title)
        .bind(user_id)         // 手写
        .bind(&now)            // 手写
        .bind(&now)            // 手写
        .execute(&self.pool)
        .await?;
}
```

**改为：**
```rust
pub async fn create(&self, post: CreatePostRequest, user_id: &str) -> Result<Post> {
    let mut record = post_to_record(&post);

    let mut ctx = DataBeforeCreateContext {
        base: BaseContext { user_id: Some(user_id.into()), ... },
        table: "posts".into(),
        record: record,
        schema: None,
    };

    // ownable 自动注入 author_id
    // timestampable 自动注入 created_at, updated_at
    self.aspect_engine.dispatch_data_before_create("posts", &mut ctx).await?;

    let result = self.do_insert("posts", &ctx.record).await?;

    let after_ctx = DataAfterCreateContext { ... };
    self.aspect_engine.dispatch_data_after_create("posts", &mut after_ctx).await?;

    Ok(result)
}
```

### 10.5 migration 层集成

```rust
// migration.rs — 建表时查询 AspectEngine 获取列定义

fn generate_create_table_sql(ct: &ContentTypeSchema, engine: &AspectEngine) -> String {
    let mut cols = vec!["id TEXT PRIMARY KEY".into()];

    // 用户定义的字段
    for field in &ct.fields {
        cols.push(field_to_sql(field));
    }

    // Aspect 注入的系统列
    for col_def in engine.columns_for(&ct.table) {
        if !cols.iter().any(|c| c.starts_with(&col_def.name)) {
            cols.push(format!("{} {}", col_def.name, col_def.sql_type));
        }
    }

    // 固定系统列
    cols.push("tenant_id TEXT NOT NULL DEFAULT 'default'".into());
    cols.push("__meta TEXT DEFAULT '{}'".into());

    format!("CREATE TABLE IF NOT EXISTS {} (\n{}\n)", ct.table, cols.join(",\n"))
}
```

## 11. Aspect 配置化

### 11.1 ContentTypeSchema TOML

```toml
[content_type]
name = "Product"
singular = "product"
plural = "products"
table = "products"

# 以下字段全部废弃，由 Aspect 接管：
# timestamps = true     → timestampable Aspect（默认内置）
# soft_delete = true    → soft_deletable Aspect
# versioning = true     → versionable Aspect
# cache = true          → cacheable Aspect

# 声明式启用可选 Aspect
implements = ["versionable", "cacheable"]
```

### 11.2 内置表配置

内置表通过代码注册：

```rust
// 启动时
aspect_engine.register_for_tables(
    &["posts", "pages"],
    vec![Arc::new(VersionableAspect)],
);
aspect_engine.register_for_tables(
    &["posts"],
    vec![Arc::new(SoftDeletableAspect)],
);
```

未来可改为配置文件：

```toml
# config/builtin_tables.toml
[[table]]
name = "posts"
aspects = ["versionable", "soft_deletable"]

[[table]]
name = "pages"
aspects = ["versionable"]
```

## 12. 未来：插件 Aspect

### 12.1 插件注册 Aspect

```toml
# extensions/plugins/sluggable/manifest.toml
[plugin]
name = "sluggable"
version = "1.0.0"

[[aspect]]
name = "sluggable"
layer = "data"
pointcuts = [
    { operation = "create", when = "before" },
    { operation = "update", when = "before" },
]

[aspect.config]
source_field = "title"
algorithm = "uuid"
```

### 12.2 插件 Aspect 的 JS 实现

```javascript
// extensions/plugins/sluggable/index.js

export function onBeforeCreate(ctx) {
    const source = ctx.record[ctx.config.source_field];
    if (source) {
        ctx.record.slug = generateSlug(source);
    }
    return { advice: "continue" };
}

export function onBeforeUpdate(ctx) {
    const source = ctx.newRecord[ctx.config.source_field];
    const oldSource = ctx.oldRecord[ctx.config.source_field];
    if (source && source !== oldSource) {
        ctx.newRecord.slug = generateSlug(source);
    }
    return { advice: "continue" };
}
```

### 12.3 插件 Aspect 的执行模型

```
Rust AspectEngine.dispatch_data_before_create()
  ├─ [Rust] OwnableAspect.on_data_before_create()
  ├─ [Rust] TimestampableAspect.on_data_before_create()
  ├─ [JS]   SluggablePlugin.onBeforeCreate()    ← 通过 JS engine 调用
  ├─ [Rust] ValidationAspect.on_data_before_create()
  └─ ...
```

插件 Aspect 和 Rust Aspect 在同一个 dispatch chain 中，按 priority 排序。

### 12.4 性能隔离

插件 Aspect 需要额外的安全措施：

- **超时：** 每个 JS/WASM Aspect 有执行超时（默认 5s）
- **内存限制：** JS engine 有内存上限
- **错误降级：** 插件 Aspect 失败时可配置为 Skip 而非 Abort
- **异步：** 非关键插件 Aspect 可标记为 async（在 Event 层执行）

## 13. 迁移计划

### Phase 1 — 基础设施（当前阶段）

1. `src/aspects/mod.rs` — Aspect trait + AspectEngine + Context 类型
2. `src/aspects/engine.rs` — AspectEngine 实现（注册、调度、匹配）
3. `src/aspects/ownable.rs` — ownable Aspect 实现
4. `src/aspects/timestampable.rs` — timestampable Aspect 实现
5. 集成到 ContentRepository（替代 auto_fill + timestamps）
6. 所有测试通过

### Phase 2 — 扩展 Aspects

1. `src/aspects/versionable.rs` — versionable Aspect
2. `src/aspects/cacheable.rs` — cacheable Aspect
3. `src/aspects/soft_deletable.rs` — soft_deletable Aspect
4. `src/aspects/audit.rs` — audit Aspect（Event 层）
5. 废弃 ContentTypeSchema 的 timestamps/soft_delete/versioning 标志

### Phase 3 — 内置表迁移

1. 内置表 service 层接入 AspectEngine
2. 移除 service 层手写的 author_id/timestamps 逻辑
3. 统一所有数据路径

### Phase 4 — 插件 Aspect

1. 插件 manifest.toml 的 `[[aspect]]` 声明
2. JS/Lua/WASM Aspect 执行器
3. 性能隔离和错误降级
4. SDK 提供 `onBeforeCreate` / `onAfterCreate` 等 API

### Phase 5 — Access Layer 和 HTTP Layer

1. access_check / access_filter 集成
2. 替换现有 rule_engine 为 Access Aspect
3. HTTP Layer Aspect（统一 middleware 注册）
4. 请求追踪、响应日志

## 14. 文件结构

```
src/
  aspects/
    mod.rs                    — Aspect trait, Context 类型, Advice, Extensions
    engine.rs                 — AspectEngine 实现
    ownable.rs                — ownable Aspect
    timestampable.rs          — timestampable Aspect
    versionable.rs            — versionable Aspect
    cacheable.rs              — cacheable Aspect
    soft_deletable.rs         — soft_deletable Aspect
    audit.rs                  — audit Aspect (Event 层)
    validation.rs             — 字段校验 Aspect
    access.rs                 — Access Layer Aspect
    field_filter.rs           — 响应字段过滤 Aspect

  content_type/
    schema.rs                 — 移除 timestamps/soft_delete/versioning 字段
    migration.rs              — 通过 AspectEngine 获取系统列
    repository.rs             — 调用 AspectEngine 替代手写逻辑
    handler.rs                — 简化，横切面由 Aspect 处理

  services/
    post.rs                   — 接入 AspectEngine
    page.rs                   — 接入 AspectEngine
    comment.rs                — 接入 AspectEngine
```

## 15. 与现有 auto_fill 的关系

| 机制 | 迁移后角色 |
|---|---|
| `auto_fill: UserId` | **废弃**，由 ownable Aspect 替代 |
| `auto_fill: CurrentTimestamp` | **废弃**，由 timestampable Aspect 替代 |
| `auto_fill: UserRole` | 保留，属于字段级注入，不是横切面 |
| `auto_fill: CurrentTenantId` | 保留，属于字段级注入，不是横切面 |
| `timestamps: true` | **废弃**，由 timestampable Aspect 替代 |
| `soft_delete: true` | **废弃**，由 soft_deletable Aspect 替代 |
| `versioning: true` | **废弃**，由 versionable Aspect 替代 |
| `cache: true` (api endpoint) | **废弃**，由 cacheable Aspect 替代 |

## 16. 设计原则

1. **一次定义，全局生效** — 每个 Aspect 定义一次，对所有表自动生效
2. **声明式** — 通过 `implements` 或代码注册启用，不需要修改业务代码
3. **优先级驱动** — 明确的执行顺序，避免隐式依赖
4. **类型安全** — 每个 JoinPoint 有专属 Context 类型，编译期检查
5. **可扩展** — 插件可注册 JS/Lua/WASM Aspect
6. **事务感知** — Data 层在事务内，Event 层在事务外
7. **优雅降级** — 非关键 Aspect 失败不影响主操作
